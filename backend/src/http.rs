//! The only way this program talks to other people's sites.
//!
//! Every request goes through [`PoliteClient`], which enforces:
//! - a global cap on concurrent requests,
//! - a minimum gap between two requests to the same host,
//! - conditional GET (ETag / Last-Modified) so unchanged feeds cost a 304,
//! - reporting the server's own freshness lifetime (Cache-Control / Expires),
//! - a body size limit and timeouts,
//! - an honest User-Agent.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{StatusCode, redirect};
use tokio::sync::Semaphore;
use tokio::time::Instant;
use url::Url;

use crate::config::CrawlerConfig;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug)]
pub enum Fetched {
    NotModified {
        fresh_for: Option<Duration>,
    },
    Ok {
        final_url: Url,
        validators: Validators,
        body: Bytes,
        /// How long the server says the response stays fresh; we do not ask again sooner.
        fresh_for: Option<Duration>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("HTTP {status}")]
    Status {
        status: StatusCode,
        /// From `Retry-After` on 429/503.
        retry_after: Option<Duration>,
    },
    #[error("response larger than {0} bytes")]
    TooLarge(usize),
    #[error("{}", network_message(.0))]
    Network(#[from] reqwest::Error),
}

/// reqwest's own message ("error sending request for url (...)") hides the
/// cause; the source chain has it (DNS failure, certificate for another
/// name, connection refused, ...).
fn network_message(e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_decode() {
        "decode"
    } else if e.is_redirect() {
        "redirect"
    } else if e.is_body() {
        "body"
    } else {
        "request"
    };
    let mut parts: Vec<String> = Vec::new();
    let mut cur = std::error::Error::source(e);
    while let Some(c) = cur {
        let msg = c.to_string();
        if !parts.iter().any(|p| p.contains(&msg)) {
            parts.push(msg);
        }
        cur = c.source();
    }
    if parts.is_empty() {
        e.to_string()
    } else {
        format!("{kind}: {}", parts.join(": "))
    }
}

impl FetchError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            FetchError::Status { status, .. } => Some(*status),
            _ => None,
        }
    }
}

pub const ACCEPT_FEED: &str = "application/rss+xml, application/atom+xml, application/feed+json, \
     application/xml;q=0.9, text/xml;q=0.9, */*;q=0.5";
pub const ACCEPT_HTML: &str = "text/html, application/xhtml+xml;q=0.9, */*;q=0.5";
pub const ACCEPT_JSON: &str = "application/json, */*;q=0.5";

pub struct PoliteClient {
    client: reqwest::Client,
    global: Semaphore,
    per_host_interval: Duration,
    max_body: usize,
    /// Earliest instant the next request to each host may start.
    next_slot: Mutex<HashMap<String, Instant>>,
}

impl PoliteClient {
    pub fn new(cfg: &CrawlerConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(&cfg.user_agent)
            .timeout(Duration::from_secs(cfg.request_timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .redirect(redirect::Policy::limited(5))
            .pool_max_idle_per_host(1)
            .build()?;
        Ok(Self {
            client,
            global: Semaphore::new(cfg.max_concurrency.max(1)),
            per_host_interval: Duration::from_secs(cfg.per_host_interval_secs),
            max_body: cfg.max_body_bytes,
            next_slot: Mutex::new(HashMap::new()),
        })
    }

    /// Waits until `host` may be contacted again and reserves that slot.
    async fn wait_for_host(&self, host: &str) {
        let start = {
            let mut slots = self.next_slot.lock().unwrap();
            let now = Instant::now();
            if slots.len() > 4096 {
                slots.retain(|_, t| *t > now);
            }
            let start = slots.get(host).copied().filter(|t| *t > now).unwrap_or(now);
            slots.insert(host.to_string(), start + self.per_host_interval);
            start
        };
        tokio::time::sleep_until(start).await;
    }

    pub async fn get(&self, url: &Url, accept: &str, validators: Option<&Validators>) -> Result<Fetched, FetchError> {
        self.wait_for_host(url.host_str().unwrap_or_default()).await;
        let _permit = self.global.acquire().await.expect("semaphore never closed");

        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
        if let Some(v) = validators {
            if let Some(etag) = v.etag.as_deref().and_then(|s| HeaderValue::from_str(s).ok()) {
                headers.insert(header::IF_NONE_MATCH, etag);
            }
            if let Some(lm) = v.last_modified.as_deref().and_then(|s| HeaderValue::from_str(s).ok()) {
                headers.insert(header::IF_MODIFIED_SINCE, lm);
            }
        }

        let mut resp = self.client.get(url.clone()).headers(headers).send().await?;
        let status = resp.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified {
                fresh_for: freshness(resp.headers()),
            });
        }
        check_status(&resp)?;

        let header_str = |name| {
            resp.headers()
                .get(name)
                .and_then(|v: &HeaderValue| v.to_str().ok())
                .map(str::to_string)
        };
        let validators = Validators {
            etag: header_str(header::ETAG),
            last_modified: header_str(header::LAST_MODIFIED),
        };
        let final_url = resp.url().clone();
        let fresh_for = freshness(resp.headers());
        let body = self.read_body(&mut resp).await?;
        Ok(Fetched::Ok {
            final_url,
            validators,
            body,
            fresh_for,
        })
    }

    /// POST a JSON body and parse the JSON answer (`Null` if it is not JSON).
    /// Same politeness rules as `get`; used for comment-count APIs.
    pub async fn post_json(&self, url: &Url, body: &serde_json::Value) -> Result<serde_json::Value, FetchError> {
        self.wait_for_host(url.host_str().unwrap_or_default()).await;
        let _permit = self.global.acquire().await.expect("semaphore never closed");
        let mut resp = self
            .client
            .post(url.clone())
            .header(header::ACCEPT, ACCEPT_JSON)
            .json(body)
            .send()
            .await?;
        check_status(&resp)?;
        let bytes = self.read_body(&mut resp).await?;
        Ok(serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    async fn read_body(&self, resp: &mut reqwest::Response) -> Result<Bytes, FetchError> {
        if resp.content_length().is_some_and(|len| len as usize > self.max_body) {
            return Err(FetchError::TooLarge(self.max_body));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if body.len() + chunk.len() > self.max_body {
                return Err(FetchError::TooLarge(self.max_body));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body.into())
    }
}

fn check_status(resp: &reqwest::Response) -> Result<(), FetchError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let retry_after = resp
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after);
    Err(FetchError::Status { status, retry_after })
}

/// Freshness lifetime from `Cache-Control: max-age`, else `Expires`.
/// `no-cache` / `no-store` mean "revalidate", so they give no lifetime.
fn freshness(headers: &HeaderMap) -> Option<Duration> {
    if let Some(cc) = headers.get(header::CACHE_CONTROL).and_then(|v| v.to_str().ok()) {
        let mut max_age = None;
        for directive in cc.split(',').map(|d| d.trim().to_ascii_lowercase()) {
            if directive == "no-cache" || directive == "no-store" {
                return None;
            }
            if let Some(v) = directive.strip_prefix("max-age=") {
                max_age = v.trim_matches('"').parse::<u64>().ok();
            }
        }
        if max_age.is_some() {
            return max_age.map(Duration::from_secs);
        }
    }
    let expires = headers.get(header::EXPIRES)?.to_str().ok()?;
    let when = chrono::DateTime::parse_from_rfc2822(expires.trim()).ok()?;
    let secs = when.timestamp() - chrono::Utc::now().timestamp();
    (secs > 0).then(|| Duration::from_secs(secs as u64))
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(secs) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = chrono::DateTime::parse_from_rfc2822(value.trim()).ok()?;
    let secs = when.timestamp() - chrono::Utc::now().timestamp();
    Some(Duration::from_secs(secs.max(0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_forms() {
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("soon"), None);
    }

    #[test]
    fn freshness_from_headers() {
        let h = |pairs: &[(header::HeaderName, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.insert(k.clone(), HeaderValue::from_str(v).unwrap());
            }
            freshness(&m)
        };
        assert_eq!(
            h(&[(header::CACHE_CONTROL, "public, max-age=600")]),
            Some(Duration::from_secs(600))
        );
        assert_eq!(h(&[(header::CACHE_CONTROL, "max-age=600, no-cache")]), None);
        assert_eq!(h(&[(header::EXPIRES, "Wed, 21 Oct 2015 07:28:00 GMT")]), None);
        let later = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc2822();
        let fresh = h(&[(header::EXPIRES, &later)]).unwrap();
        assert!(fresh > Duration::from_secs(7000) && fresh <= Duration::from_secs(7200));
        assert_eq!(h(&[]), None);
    }

    #[tokio::test(start_paused = true)]
    async fn same_host_requests_are_spaced() {
        let cfg = CrawlerConfig {
            per_host_interval_secs: 10,
            ..Default::default()
        };
        let client = PoliteClient::new(&cfg).unwrap();
        let t0 = Instant::now();
        client.wait_for_host("a.example").await;
        client.wait_for_host("b.example").await;
        assert!(t0.elapsed() < Duration::from_secs(1), "different hosts do not wait");
        client.wait_for_host("a.example").await;
        assert!(t0.elapsed() >= Duration::from_secs(10), "same host waits for its slot");
    }
}
