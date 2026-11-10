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
        /// The response may not be shown in a frame on another site.
        frame_blocked: bool,
    },
}

/// Whether a response forbids other sites from showing it in a frame:
/// CSP `frame-ancestors` without a wildcard, or else `X-Frame-Options`
/// (browsers ignore the latter when the former is present).
pub fn frame_blocked(headers: &HeaderMap) -> bool {
    let ancestors: Vec<String> = headers
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| d.split_whitespace().next() == Some("frame-ancestors"))
        .collect();
    if !ancestors.is_empty() {
        return ancestors.iter().any(|d| {
            !d.split_whitespace()
                .skip(1)
                .any(|s| matches!(s, "*" | "https:" | "http:" | "https://*" | "http://*"))
        });
    }
    headers
        .get_all(header::X_FRAME_OPTIONS)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "deny" || v == "sameorigin" || v.starts_with("allow-from")
        })
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

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            FetchError::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

pub const ACCEPT_FEED: &str = "application/rss+xml, application/atom+xml, application/feed+json, \
     application/xml;q=0.9, text/xml;q=0.9, */*;q=0.5";
pub const ACCEPT_HTML: &str = "text/html, application/xhtml+xml;q=0.9, */*;q=0.5";
pub const ACCEPT_JSON: &str = "application/json, */*;q=0.5";
pub const ACCEPT_IMAGE: &str = "image/avif, image/webp, image/png, image/svg+xml, image/*;q=0.8, */*;q=0.5";

/// Whether a body is a picture, by its first bytes (servers often send
/// pictures as `application/octet-stream`, and error pages as 200).
pub fn looks_like_image(body: &[u8]) -> bool {
    let starts = |magic: &[u8]| body.starts_with(magic);
    starts(b"\x89PNG")
        || starts(b"\xFF\xD8\xFF")
        || starts(b"GIF8")
        || (starts(b"RIFF") && body.get(8..12) == Some(&b"WEBP"[..]))
        || starts(b"\0\0\x01\0")
        || starts(b"BM")
        || body.get(4..8) == Some(&b"ftyp"[..])
        || {
            // SVG: text with an <svg element near the start.
            let head = String::from_utf8_lossy(&body[..body.len().min(1024)]).to_ascii_lowercase();
            head.trim_start_matches('\u{feff}').trim_start().starts_with('<') && head.contains("<svg")
        }
}

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
        let frame_blocked = frame_blocked(resp.headers());
        let body = self.read_body(&mut resp).await?;
        Ok(Fetched::Ok {
            final_url,
            validators,
            body,
            fresh_for,
            frame_blocked,
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
    fn pictures_are_told_by_their_first_bytes() {
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\n...."));
        assert!(looks_like_image(b"\xFF\xD8\xFF\xE0..JFIF"));
        assert!(looks_like_image(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(looks_like_image(b"\0\0\0\x1cftypavif"));
        assert!(looks_like_image(b"\0\0\x01\0\x01\0"), "an .ico");
        assert!(looks_like_image(
            b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"
        ));
        assert!(!looks_like_image(b"<!doctype html><html><body>Not found</body></html>"));
        assert!(!looks_like_image(b"Couldn't find the requested file."));
    }

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

    #[test]
    fn frame_policy_from_headers() {
        let h = |pairs: &[(&str, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.append(
                    header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                    HeaderValue::from_str(v).unwrap(),
                );
            }
            frame_blocked(&m)
        };
        assert!(!h(&[]));
        assert!(h(&[("x-frame-options", "DENY")]));
        assert!(h(&[("x-frame-options", "SAMEORIGIN")]));
        assert!(h(&[(
            "content-security-policy",
            "default-src 'self'; frame-ancestors 'self'"
        )]));
        assert!(
            !h(&[("content-security-policy", "default-src 'self'")]),
            "no frame-ancestors: no limit"
        );
        assert!(
            !h(&[
                ("x-frame-options", "SAMEORIGIN"),
                ("content-security-policy", "frame-ancestors *")
            ]),
            "frame-ancestors wins over X-Frame-Options"
        );
        assert!(!h(&[("content-security-policy", "frame-ancestors 'self' https:")]));
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
