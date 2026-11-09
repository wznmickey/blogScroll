//! HTTP caching for what *we* serve.
//!
//! - API responses are kept in a small in-memory cache (bounded by entry
//!   count, short TTL per endpoint) and get a weak ETag, so repeat requests
//!   cost neither a database query nor, on revalidation, a body.
//! - Static files: hashed `/assets/*` are immutable for a year, HTML is always
//!   revalidated so a new deploy shows up immediately.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::time::Instant;

const MAX_CACHED_BODY: usize = 2 * 1024 * 1024;

pub struct ResponseCache {
    entries: Mutex<HashMap<String, Entry>>,
    max_entries: usize,
}

#[derive(Clone)]
struct Entry {
    body: Bytes,
    etag: HeaderValue,
    content_type: Option<HeaderValue>,
    expires: Instant,
}

impl ResponseCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_entries: max_entries.max(1),
        }
    }

    fn get(&self, key: &str) -> Option<Entry> {
        let mut entries = self.entries.lock().unwrap();
        match entries.get(key) {
            Some(e) if e.expires > Instant::now() => Some(e.clone()),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    fn put(&self, key: String, entry: Entry) {
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= self.max_entries {
            let now = Instant::now();
            entries.retain(|_, e| e.expires > now);
        }
        if entries.len() >= self.max_entries {
            // Still full of live entries: drop the one closest to expiry.
            if let Some(oldest) = entries.iter().min_by_key(|(_, e)| e.expires).map(|(k, _)| k.clone()) {
                entries.remove(&oldest);
            }
        }
        entries.insert(key, entry);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

/// How long each endpoint's answer may be reused, by us and by browsers.
fn ttl(path: &str) -> Duration {
    let secs = if path.starts_with("/api/posts/") {
        600
    } else if path == "/api/meta" || path == "/api/tags" || path.starts_with("/api/blogs/") {
        300
    } else {
        60
    };
    Duration::from_secs(secs)
}

pub async fn api_cache(State(cache): State<Arc<ResponseCache>>, req: Request, next: Next) -> Response {
    if req.method() != Method::GET {
        return next.run(req).await;
    }
    let key = req.uri().to_string();
    let if_none_match = req.headers().get(IF_NONE_MATCH).cloned();

    let entry = match cache.get(&key) {
        Some(e) => e,
        None => {
            let ttl = ttl(req.uri().path());
            let resp = next.run(req).await;
            if resp.status() != StatusCode::OK {
                return resp;
            }
            let (parts, body) = resp.into_parts();
            let Ok(body) = axum::body::to_bytes(body, MAX_CACHED_BODY).await else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            let mut h = DefaultHasher::new();
            body.hash(&mut h);
            let entry = Entry {
                etag: HeaderValue::from_str(&format!("W/\"{:016x}\"", h.finish())).unwrap(),
                content_type: parts.headers.get(CONTENT_TYPE).cloned(),
                expires: Instant::now() + ttl,
                body,
            };
            cache.put(key, entry.clone());
            entry
        }
    };

    // Browsers may reuse it for whatever is left of our own TTL.
    let remaining = entry.expires.saturating_duration_since(Instant::now()).as_secs();
    let cache_control = HeaderValue::from_str(&format!(
        "public, max-age={remaining}, stale-while-revalidate={}",
        remaining * 5
    ))
    .unwrap();

    if if_none_match.is_some_and(|v| etag_matches(&v, &entry.etag)) {
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        resp.headers_mut().insert(ETAG, entry.etag);
        resp.headers_mut().insert(CACHE_CONTROL, cache_control);
        return resp;
    }
    let mut resp = Response::new(Body::from(entry.body));
    let headers = resp.headers_mut();
    if let Some(ct) = entry.content_type {
        headers.insert(CONTENT_TYPE, ct);
    }
    headers.insert(ETAG, entry.etag);
    headers.insert(CACHE_CONTROL, cache_control);
    resp
}

fn etag_matches(if_none_match: &HeaderValue, etag: &HeaderValue) -> bool {
    let strip = |s: &str| s.trim().trim_start_matches("W/").to_string();
    let ours = strip(etag.to_str().unwrap_or_default());
    if_none_match
        .to_str()
        .unwrap_or_default()
        .split(',')
        .any(|t| t.trim() == "*" || strip(t) == ours)
}

/// Cache-Control for the built frontend.
pub async fn static_headers(req: Request, next: Next) -> Response {
    let hashed_asset = req.uri().path().starts_with("/assets/");
    let mut resp = next.run(req).await;
    let is_html = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));
    // The SPA fallback answers unknown paths (even under /assets/) with
    // index.html, which must never be cached as immutable.
    let value = if is_html {
        "no-cache"
    } else if hashed_asset {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=86400"
    };
    resp.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(value));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::http::Request;
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    fn app(cache: Arc<ResponseCache>, hits: Arc<AtomicUsize>) -> Router {
        Router::new()
            .route(
                "/api/posts",
                get(move || {
                    let n = hits.fetch_add(1, Ordering::SeqCst);
                    async move { axum::Json(serde_json::json!({ "n": n })) }
                }),
            )
            .route_layer(axum::middleware::from_fn_with_state(cache, api_cache))
    }

    async fn call(app: &Router, uri: &str, inm: Option<&HeaderValue>) -> Response {
        let mut req = Request::get(uri);
        if let Some(v) = inm {
            req = req.header(IF_NONE_MATCH, v);
        }
        app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn serves_from_cache_and_revalidates_with_etag() {
        let hits = Arc::new(AtomicUsize::new(0));
        let app = app(Arc::new(ResponseCache::new(10)), hits.clone());

        let first = call(&app, "/api/posts?x=1", None).await;
        assert_eq!(first.status(), StatusCode::OK);
        let etag = first.headers()[ETAG].clone();
        assert!(etag.to_str().unwrap().starts_with("W/\""));
        let cc = first.headers()[CACHE_CONTROL].to_str().unwrap().to_string();
        assert!(cc.starts_with("public, max-age=") && cc.contains("stale-while-revalidate"));

        let second = call(&app, "/api/posts?x=1", None).await;
        assert_eq!(second.headers()[ETAG], etag);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "second request served from cache");

        let not_modified = call(&app, "/api/posts?x=1", Some(&etag)).await;
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);

        call(&app, "/api/posts?x=2", None).await;
        assert_eq!(hits.load(Ordering::SeqCst), 2, "different query, different entry");
    }

    #[tokio::test(start_paused = true)]
    async fn entries_expire_and_size_is_bounded() {
        let hits = Arc::new(AtomicUsize::new(0));
        let cache = Arc::new(ResponseCache::new(3));
        let app = app(cache.clone(), hits.clone());
        for i in 0..10 {
            call(&app, &format!("/api/posts?i={i}"), None).await;
        }
        assert_eq!(cache.len(), 3);

        call(&app, "/api/posts?i=9", None).await;
        assert_eq!(hits.load(Ordering::SeqCst), 10);
        tokio::time::advance(Duration::from_secs(61)).await;
        call(&app, "/api/posts?i=9", None).await;
        assert_eq!(hits.load(Ordering::SeqCst), 11, "expired entry is fetched again");
    }

    #[test]
    fn etag_matching() {
        let ours = HeaderValue::from_static("W/\"abc\"");
        assert!(etag_matches(&HeaderValue::from_static("W/\"abc\""), &ours));
        assert!(etag_matches(&HeaderValue::from_static("\"x\", \"abc\""), &ours));
        assert!(etag_matches(&HeaderValue::from_static("*"), &ours));
        assert!(!etag_matches(&HeaderValue::from_static("\"abd\""), &ours));
    }
}
