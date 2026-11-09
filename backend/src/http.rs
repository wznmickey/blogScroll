//! The only way this program talks to other people's sites.
//!
//! Every request goes through [`PoliteClient`], which enforces:
//! - a global cap on concurrent requests,
//! - a minimum gap between two requests to the same host,
//! - conditional GET (ETag / Last-Modified) so unchanged feeds cost a 304,
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

#[derive(Debug, Clone, Default)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug)]
pub enum Fetched {
    NotModified,
    Ok {
        final_url: Url,
        validators: Validators,
        body: Bytes,
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
    #[error("{0}")]
    Network(#[from] reqwest::Error),
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
            return Ok(Fetched::NotModified);
        }
        if !status.is_success() {
            let retry_after = resp
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            return Err(FetchError::Status { status, retry_after });
        }
        if resp.content_length().is_some_and(|len| len as usize > self.max_body) {
            return Err(FetchError::TooLarge(self.max_body));
        }

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

        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if body.len() + chunk.len() > self.max_body {
                return Err(FetchError::TooLarge(self.max_body));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Fetched::Ok {
            final_url,
            validators,
            body: body.into(),
        })
    }
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
