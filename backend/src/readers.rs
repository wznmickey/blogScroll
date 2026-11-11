//! What the server does for readers besides showing the site, still without
//! accounts:
//!
//! - `/api/sync`: their settings and lists, carried between their devices.
//!   One blob per sync code, encrypted in the browser with a key the server
//!   never sees; known here by a hash of a token derived from the same code
//!   (frontend/src/sync.ts). Every write names the version it builds on
//!   (`If-Match`, or `If-None-Match: *` for the first), so two devices never
//!   overwrite each other: the one that lost reads, merges and writes again.
//! - `/api/fetch`: feeds readers add for themselves (设置 → 来源), fetched for
//!   their browser, which may not read other sites. Passes on only feeds (RSS,
//!   Atom, JSON Feed) and OPML lists, as a download; for a web page, the feeds
//!   it links to. Never an address on this machine or its network. Answers
//!   are kept a while, so readers subscribed to the same feed share a fetch.
//! - `/api/readers/feeds`: the feeds a reader added, registered (without who
//!   they are) so the crawler takes them in like any other blog
//!   (sources/readers.rs), shown only to readers who have them; answers which
//!   blog each one is, for their lists (`own=`).
//! - `/api/refresh`: a visitor asks for one blog to be read again now (the
//!   button on its page); at most every REFRESH_SECS, never over a site's own
//!   request to wait.
//! - `/api/events`: what readers do, counted without them in it (stats.rs).
//! - `/api/capabilities`: which of these are here (none on plain static hosting).
//!
//! The Cloudflare Worker (worker/src/index.js) answers the same for the
//! static site.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::time::Instant;
use url::{Host, Url};

use crate::crawler::Crawler;
use crate::http::{ACCEPT_FEED, FetchError};

/// Largest sync blob (the Worker's database keeps at most 2 MB in a row).
pub const MAX_SYNC_BYTES: usize = 1_900_000;
/// Sync codes kept at most; past this no new ones (those in use still work).
const MAX_SYNC_DOCS: i64 = 20_000;
/// Largest feed passed on.
const MAX_FEED_BYTES: usize = 4 * 1024 * 1024;
const FEED_TTL: Duration = Duration::from_secs(15 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHED_FEEDS: usize = 256;

/// Feeds one registration may carry, and that all readers may have.
const MAX_OWN_FEEDS: usize = 500;
const MAX_READER_FEEDS: i64 = 5000;
/// A blog is read again on request at most this often.
pub const REFRESH_SECS: i64 = 15 * 60;

#[derive(Clone)]
struct ReaderState {
    pool: SqlitePool,
    proxy: Arc<FeedProxy>,
    /// None when this server does not crawl (`serve --no-crawl`).
    crawler: Option<Arc<Crawler>>,
}

pub fn router(pool: SqlitePool, proxy: FeedProxy, crawler: Option<Arc<Crawler>>) -> Router {
    Router::new()
        .route("/api/capabilities", get(capabilities))
        .route("/api/sync", get(pull).put(push).delete(forget))
        .route("/api/fetch", get(fetch))
        .route("/api/readers/feeds", post(register))
        .route("/api/refresh", post(refresh))
        .route("/api/events", post(events))
        .layer(DefaultBodyLimit::max(MAX_SYNC_BYTES))
        .with_state(ReaderState {
            pool,
            proxy: Arc::new(proxy),
            crawler,
        })
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn error(status: StatusCode, message: &str) -> Response {
    no_store((status, Json(json!({ "error": message }))).into_response())
}

fn db_error(e: sqlx::Error) -> Response {
    tracing::error!(error = %e, "database error");
    error(StatusCode::INTERNAL_SERVER_ERROR, "database error")
}

async fn capabilities(State(s): State<ReaderState>) -> Response {
    let crawls = s.crawler.is_some();
    no_store(
        Json(json!({ "sync": true, "fetch": true, "readers": crawls, "refresh": crawls, "stats": true }))
            .into_response(),
    )
}

// ---- Sync ----

/// `Authorization: Bearer <64 hex>` → the blob's id: the token's SHA-256, so
/// what is stored here is not enough to write to it.
fn doc_id(headers: &HeaderMap) -> Option<String> {
    let token = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim()
        .to_ascii_lowercase();
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex::encode(Sha256::digest(token.as_bytes())))
}

/// `"12"` → 12, an entity tag as we send them.
fn version_of(v: &HeaderValue) -> Option<i64> {
    v.to_str()
        .ok()?
        .trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .parse()
        .ok()
}

fn etag(version: i64) -> (header::HeaderName, HeaderValue) {
    (header::ETAG, HeaderValue::from_str(&format!("\"{version}\"")).unwrap())
}

async fn pull(State(s): State<ReaderState>, headers: HeaderMap) -> Response {
    let Some(id) = doc_id(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "missing or malformed token");
    };
    let row: Option<(i64, Vec<u8>, bool)> =
        match sqlx::query_as("SELECT version, data, deleted FROM sync_docs WHERE id = ?")
            .bind(&id)
            .fetch_optional(&s.pool)
            .await
        {
            Ok(row) => row,
            Err(e) => return db_error(e),
        };
    match row {
        None => error(StatusCode::NOT_FOUND, "nothing synced with this code yet"),
        Some((_, _, true)) => error(StatusCode::GONE, "sync was turned off for this code"),
        Some((v, _, _)) if headers.get(header::IF_NONE_MATCH).and_then(version_of) == Some(v) => {
            no_store((StatusCode::NOT_MODIFIED, [etag(v)]).into_response())
        }
        Some((v, data, _)) => no_store(
            (
                [
                    etag(v),
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/octet-stream"),
                    ),
                ],
                data,
            )
                .into_response(),
        ),
    }
}

async fn push(State(s): State<ReaderState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(id) = doc_id(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "missing or malformed token");
    };
    if body.is_empty() {
        return error(StatusCode::BAD_REQUEST, "empty body");
    }
    let now = crate::util::now();
    let written = if headers.get(header::IF_NONE_MATCH).is_some_and(|v| v == "*") {
        let count: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM sync_docs")
            .fetch_one(&s.pool)
            .await
        {
            Ok(n) => n,
            Err(e) => return db_error(e),
        };
        if count >= MAX_SYNC_DOCS {
            return error(StatusCode::INSUFFICIENT_STORAGE, "no room for another sync code");
        }
        sqlx::query(
            "INSERT INTO sync_docs (id, version, data, deleted, updated_at) VALUES (?, 1, ?, 0, ?)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&id)
        .bind(body.as_ref())
        .bind(now)
        .execute(&s.pool)
        .await
        .map(|r| (r.rows_affected() == 1).then_some(1))
    } else if let Some(base) = headers.get(header::IF_MATCH).and_then(version_of) {
        sqlx::query(
            "UPDATE sync_docs SET version = version + 1, data = ?, updated_at = ?
             WHERE id = ? AND version = ? AND deleted = 0",
        )
        .bind(body.as_ref())
        .bind(now)
        .bind(&id)
        .bind(base)
        .execute(&s.pool)
        .await
        .map(|r| (r.rows_affected() == 1).then_some(base + 1))
    } else {
        return error(
            StatusCode::PRECONDITION_REQUIRED,
            "send If-Match, or If-None-Match: * for the first write",
        );
    };
    match written {
        Ok(Some(v)) => no_store(([etag(v)], Json(json!({ "version": v }))).into_response()),
        // Someone wrote first, or sync was turned off.
        Ok(None) => match sqlx::query_scalar::<_, bool>("SELECT deleted FROM sync_docs WHERE id = ?")
            .bind(&id)
            .fetch_optional(&s.pool)
            .await
        {
            Ok(Some(true)) => error(StatusCode::GONE, "sync was turned off for this code"),
            Ok(_) => error(StatusCode::PRECONDITION_FAILED, "changed since; read it again"),
            Err(e) => db_error(e),
        },
        Err(e) => db_error(e),
    }
}

/// Drops sync copies nobody wrote for this long (also the Worker's, daily).
const KEEP_DAYS: i64 = 400;

pub async fn prune(pool: &SqlitePool) -> sqlx::Result<u64> {
    let before = crate::util::now() - KEEP_DAYS * 86400;
    Ok(sqlx::query("DELETE FROM sync_docs WHERE updated_at < ?")
        .bind(before)
        .execute(pool)
        .await?
        .rows_affected())
}

/// Turns sync off for this code, on every device: its data goes, a marker stays.
async fn forget(State(s): State<ReaderState>, headers: HeaderMap) -> Response {
    let Some(id) = doc_id(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "missing or malformed token");
    };
    let done = sqlx::query(
        "INSERT INTO sync_docs (id, version, data, deleted, updated_at) VALUES (?, 1, x'', 1, ?)
         ON CONFLICT (id) DO UPDATE SET version = version + 1, data = x'', deleted = 1,
             updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(crate::util::now())
    .execute(&s.pool)
    .await;
    match done {
        Ok(_) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(e) => db_error(e),
    }
}

// ---- Feeds readers add themselves ----

/// Fetches feeds for readers' browsers, with its own HTTP client: public
/// addresses only, checked on every redirect and after every DNS lookup.
pub struct FeedProxy {
    client: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Answer)>>,
    allow_private: bool,
}

const PRIVATE: &str = "not a public address";

impl FeedProxy {
    pub fn new(user_agent: &str) -> anyhow::Result<Self> {
        Self::build(user_agent, false)
    }

    fn build(user_agent: &str, allow_private: bool) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(8))
            .redirect(redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many redirects")
                } else if !allowed(attempt.url(), allow_private) {
                    attempt.error(PRIVATE)
                } else {
                    attempt.follow()
                }
            }))
            .dns_resolver(PublicOnly { allow_private })
            .build()?;
        Ok(Self {
            client,
            cache: Mutex::new(HashMap::new()),
            allow_private,
        })
    }

    async fn get(&self, url: &Url) -> Answer {
        let key = url.as_str().to_string();
        if let Some((at, answer)) = self.cache.lock().unwrap().get(&key)
            && at.elapsed() < answer.ttl()
        {
            return answer.clone();
        }
        let answer = self.fetch(url).await;
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= CACHED_FEEDS {
            cache.retain(|_, (at, a)| at.elapsed() < a.ttl());
            if cache.len() >= CACHED_FEEDS {
                cache.clear();
            }
        }
        cache.insert(key, (Instant::now(), answer.clone()));
        answer
    }

    async fn fetch(&self, url: &Url) -> Answer {
        if !allowed(url, self.allow_private) {
            return Answer::failed(StatusCode::FORBIDDEN, "不能访问内网或本机的地址");
        }
        let mut resp = match self
            .client
            .get(url.clone())
            .header(header::ACCEPT, ACCEPT_FEED)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Answer::network(e),
        };
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
            let retry_after = resp
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            return Answer::Failed {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "对方网站暂时不让访问，稍后再试".into(),
                retry_after,
            };
        }
        if !status.is_success() {
            return Answer::failed(
                StatusCode::BAD_GATEWAY,
                &format!("对方网站返回 HTTP {}", status.as_u16()),
            );
        }
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let mut body = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) if body.len() + chunk.len() > MAX_FEED_BYTES => {
                    return Answer::failed(StatusCode::BAD_GATEWAY, "内容太大，不像是 RSS");
                }
                Ok(Some(chunk)) => body.extend_from_slice(&chunk),
                Ok(None) => break,
                Err(e) => return Answer::network(e),
            }
        }
        match sniff(&body) {
            Kind::Feed => Answer::Feed {
                body: Bytes::from(body),
                content_type,
                final_url,
            },
            Kind::Page => {
                let base = Url::parse(&final_url).unwrap_or_else(|_| url.clone());
                let feeds = crate::html::feed_links(&String::from_utf8_lossy(&body), &base)
                    .into_iter()
                    .map(String::from)
                    .collect();
                Answer::Page { feeds, final_url }
            }
            Kind::Other => Answer::failed(StatusCode::UNPROCESSABLE_ENTITY, "不是 RSS、Atom 或 OPML"),
        }
    }
}

#[derive(Clone)]
enum Answer {
    Feed {
        body: Bytes,
        content_type: String,
        final_url: String,
    },
    /// A web page: the feeds it links to.
    Page { feeds: Vec<String>, final_url: String },
    Failed {
        status: StatusCode,
        message: String,
        retry_after: Option<u64>,
    },
}

impl Answer {
    fn failed(status: StatusCode, message: &str) -> Self {
        Answer::Failed {
            status,
            message: message.into(),
            retry_after: None,
        }
    }

    fn network(e: reqwest::Error) -> Self {
        let timeout = e.is_timeout();
        let detail = FetchError::from(e).to_string();
        if detail.contains(PRIVATE) {
            Answer::failed(StatusCode::FORBIDDEN, "不能访问内网或本机的地址")
        } else if timeout {
            Answer::failed(StatusCode::GATEWAY_TIMEOUT, "对方网站没有及时响应")
        } else {
            Answer::failed(StatusCode::BAD_GATEWAY, &format!("连不上对方网站（{detail}）"))
        }
    }

    fn ttl(&self) -> Duration {
        match self {
            Answer::Failed {
                retry_after: Some(s), ..
            } => Duration::from_secs((*s).clamp(60, 6 * 3600)),
            Answer::Failed { .. } => FAILURE_TTL,
            _ => FEED_TTL,
        }
    }
}

impl IntoResponse for Answer {
    fn into_response(self) -> Response {
        let private = (header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=600"));
        let nosniff = (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        let value = |v: &str| HeaderValue::from_str(v).unwrap_or(HeaderValue::from_static(""));
        match self {
            // A download, never a page of ours: what a feed holds must not run here.
            Answer::Feed {
                body,
                content_type,
                final_url,
            } => (
                [
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/octet-stream"),
                    ),
                    (header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment")),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        HeaderValue::from_static("sandbox; default-src 'none'"),
                    ),
                    nosniff,
                    private,
                    (
                        header::HeaderName::from_static("x-kind"),
                        HeaderValue::from_static("feed"),
                    ),
                    (header::HeaderName::from_static("x-feed-type"), value(&content_type)),
                    (header::HeaderName::from_static("x-final-url"), value(&final_url)),
                ],
                body,
            )
                .into_response(),
            Answer::Page { feeds, final_url } => (
                [
                    nosniff,
                    private,
                    (
                        header::HeaderName::from_static("x-kind"),
                        HeaderValue::from_static("page"),
                    ),
                ],
                Json(json!({ "feeds": feeds, "final_url": final_url })),
            )
                .into_response(),
            Answer::Failed {
                status,
                message,
                retry_after,
            } => {
                let mut r = error(status, &message);
                if let Some(s) = retry_after {
                    r.headers_mut().insert(header::RETRY_AFTER, s.into());
                }
                r
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Kind {
    Feed,
    Page,
    Other,
}

/// What a body is, by its start: feeds and OPML lists (XML), JSON Feed, or a
/// web page.
fn sniff(body: &[u8]) -> Kind {
    let head = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with('{') {
        return if head.contains("jsonfeed.org") {
            Kind::Feed
        } else {
            Kind::Other
        };
    }
    if head.starts_with("<!doctype html") || head.starts_with("<html") {
        return Kind::Page;
    }
    let xml = head.starts_with('<');
    if xml && ["<rss", "<feed", "<rdf:rdf", "<opml"].iter().any(|t| head.contains(t)) {
        Kind::Feed
    } else if xml && head.contains("<html") {
        Kind::Page
    } else {
        Kind::Other
    }
}

/// Addresses on the open internet only: no IPs of this machine or its
/// networks, no names that only resolve inside one.
fn allowed(url: &Url, allow_private: bool) -> bool {
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.host() {
        Some(Host::Ipv4(ip)) => allow_private || is_public(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => allow_private || is_public(IpAddr::V6(ip)),
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            allow_private
                || !(d == "localhost"
                    || [".localhost", ".local", ".internal", ".lan", ".home.arpa"]
                        .iter()
                        .any(|s| d.ends_with(s))
                    || !d.contains('.'))
        }
        None => false,
    }
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                // Shared (carrier-grade NAT), IETF protocol assignments, benchmarking.
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (18..20).contains(&b)))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] == 0x2001 && s[1] == 0xdb8) // documentation
                || (s[0] == 0x64 && s[1] == 0xff9b)) // NAT64
        }
    }
}

/// DNS that answers only with public addresses (a name may point anywhere).
struct PublicOnly {
    allow_private: bool,
}

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let allow_private = self.allow_private;
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| allow_private || is_public(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(PRIVATE.into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

#[derive(Deserialize)]
struct FetchQuery {
    url: String,
}

async fn fetch(State(s): State<ReaderState>, Query(q): Query<FetchQuery>) -> Response {
    let Some(url) = crate::util::parse_http_url(&q.url) else {
        return error(StatusCode::BAD_REQUEST, "地址无效");
    };
    s.proxy.get(&url).await.into_response()
}

// ---- Readers' own feeds, and reading a blog again on request ----

#[derive(Deserialize)]
struct Registration {
    feeds: Vec<String>,
    /// The browser's random id (statistics), to count each feed's readers.
    #[serde(default)]
    who: Option<String>,
    /// False: statistics turned off, so they are not counted (any more).
    #[serde(default)]
    counted: Option<bool>,
}

/// All the feeds a reader has, each time they change (and now and then, to
/// say they still want them): those new to the server are crawled from now
/// on. Answers which blog each one is, once crawled.
async fn register(State(s): State<ReaderState>, Json(r): Json<Registration>) -> Response {
    let now = crate::util::now();
    let mut count: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM reader_feeds")
        .fetch_one(&s.pool)
        .await
    {
        Ok(n) => n,
        Err(e) => return db_error(e),
    };
    let mut blogs = serde_json::Map::new();
    let mut fresh = Vec::new();
    for raw in r.feeds.iter().map(|f| f.trim()).take(MAX_OWN_FEEDS) {
        // Kept as sent: the reader's browser finds its file by this text.
        if raw.len() > 2000 || !crate::util::parse_http_url(raw).is_some_and(|u| allowed(&u, s.proxy.allow_private)) {
            continue;
        }
        let seen = sqlx::query("UPDATE reader_feeds SET last_at = ? WHERE url = ?")
            .bind(now)
            .bind(raw)
            .execute(&s.pool)
            .await;
        match seen {
            Ok(r) if r.rows_affected() > 0 => {}
            Ok(_) if count >= MAX_READER_FEEDS => continue,
            Ok(_) => {
                if let Err(e) =
                    sqlx::query("INSERT OR IGNORE INTO reader_feeds (url, first_at, last_at) VALUES (?, ?, ?)")
                        .bind(raw)
                        .bind(now)
                        .bind(now)
                        .execute(&s.pool)
                        .await
                {
                    return db_error(e);
                }
                count += 1;
                fresh.push(raw.to_string());
            }
            Err(e) => return db_error(e),
        }
        let blog = match crate::db::blog_by_site_or_feed(&s.pool, raw).await {
            Ok(b) => b.map(|b| b.id),
            Err(e) => return db_error(e),
        };
        blogs.insert(raw.to_string(), blog.into());
    }
    if let Some(who) = r.who.as_deref().and_then(crate::stats::who) {
        let feeds: Vec<String> = blogs.keys().cloned().collect();
        if let Err(e) = crate::stats::note_subscriptions(&s.pool, &who, &feeds, r.counted != Some(false)).await {
            return db_error(e);
        }
    }
    // New to this server: listed and read now, not at the next sync.
    if let Some(crawler) = s.crawler.clone().filter(|_| !fresh.is_empty()) {
        let pool = s.pool.clone();
        tokio::spawn(async move {
            if let Err(e) = crawler.sync_local_directories().await {
                tracing::warn!(error = %e, "listing readers' new feeds failed");
                return;
            }
            let mut new = Vec::new();
            for url in &fresh {
                if let Ok(Some(b)) = crate::db::blog_by_site_or_feed(&pool, url).await {
                    new.push(b);
                }
            }
            let stats = crawler.crawl_blogs(new).await;
            tracing::info!(?stats, "readers' new feeds read");
        });
    }
    no_store(Json(json!({ "blogs": blogs })).into_response())
}

#[derive(Deserialize)]
struct Events {
    /// The browser's random id: hashed, only to count each visitor once a day.
    #[serde(default)]
    id: Option<String>,
    events: Vec<crate::stats::Event>,
}

/// What a reader did (a batch, sent now and then and when they leave).
async fn events(State(s): State<ReaderState>, Json(e): Json<Events>) -> Response {
    let who = e.id.as_deref().and_then(crate::stats::who);
    let list = &e.events[..e.events.len().min(200)];
    match crate::stats::record(&s.pool, who.as_deref(), list).await {
        Ok(n) => no_store(Json(json!({ "counted": n })).into_response()),
        Err(e) => db_error(e),
    }
}

#[derive(Deserialize)]
struct RefreshRequest {
    /// The blog's address (`example.com/blog`), or a reader's feed address.
    site: String,
}

/// Reads one blog again now (the button on its page).
async fn refresh(State(s): State<ReaderState>, Json(q): Json<RefreshRequest>) -> Response {
    let Some(crawler) = s.crawler.clone() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "这里的服务器不抓取");
    };
    let blog = match crate::db::blog_by_site_or_feed(&s.pool, q.site.trim()).await {
        Ok(Some(b)) => b,
        Ok(None) => return error(StatusCode::NOT_FOUND, "找不到这个博客"),
        Err(e) => return db_error(e),
    };
    let (last, next): (Option<i64>, i64) =
        match sqlx::query_as("SELECT last_checked_at, next_check_at FROM blogs WHERE id = ?")
            .bind(blog.id)
            .fetch_one(&s.pool)
            .await
        {
            Ok(r) => r,
            Err(e) => return db_error(e),
        };
    let now = crate::util::now();
    let later = |at: i64, message: &str| {
        no_store(
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "status": "later", "error": message, "retry_at": at })),
            )
                .into_response(),
        )
    };
    // After errors (and a site's 429 / 503) the crawler waits as told.
    if blog.failures > 0 && next > now {
        return later(next, "这个网站上次没读成，或请我们晚点再来，到时会自动再读");
    }
    if let Some(last) = last.filter(|l| now - l < REFRESH_SECS) {
        return later(last + REFRESH_SECS, "刚刚读过");
    }
    let stats = crawler.crawl_blogs(vec![blog]).await;
    no_store(Json(json!({ "status": "done", "new_posts": stats.new_posts })).into_response())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    async fn call(app: &Router, method: &str, uri: &str, headers: &[(&str, &str)], body: &[u8]) -> Response {
        let mut req = Request::builder().method(method).uri(uri);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        app.clone()
            .oneshot(req.body(Body::from(body.to_vec())).unwrap())
            .await
            .unwrap()
    }

    async fn bytes(r: Response) -> Vec<u8> {
        axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()
    }

    fn tag(r: &Response) -> &str {
        r.headers().get(header::ETAG).unwrap().to_str().unwrap()
    }

    #[tokio::test]
    async fn sync_never_lets_one_device_overwrite_another() {
        let app = router(crate::db::connect_memory().await, FeedProxy::new("test").unwrap(), None);
        let auth = format!("Bearer {TOKEN}");
        let auth = auth.as_str();

        assert_eq!(
            call(&app, "GET", "/api/sync", &[], b"").await.status(),
            StatusCode::UNAUTHORIZED
        );
        let r = call(&app, "GET", "/api/sync", &[("authorization", "Bearer abc")], b"").await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let r = call(&app, "GET", "/api/sync", &[("authorization", auth)], b"").await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);

        // The first write creates it; a second "first" write loses.
        let r = call(
            &app,
            "PUT",
            "/api/sync",
            &[("authorization", auth), ("if-none-match", "*")],
            b"one",
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(tag(&r), "\"1\"");
        let r = call(
            &app,
            "PUT",
            "/api/sync",
            &[("authorization", auth), ("if-none-match", "*")],
            b"two",
        )
        .await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
        let r = call(&app, "PUT", "/api/sync", &[("authorization", auth)], b"two").await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_REQUIRED);

        let r = call(&app, "GET", "/api/sync", &[("authorization", auth)], b"").await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(bytes(r).await, b"one");
        let r = call(
            &app,
            "GET",
            "/api/sync",
            &[("authorization", auth), ("if-none-match", "\"1\"")],
            b"",
        )
        .await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);

        // Two devices build on version 1: the second has to read again.
        let r = call(
            &app,
            "PUT",
            "/api/sync",
            &[("authorization", auth), ("if-match", "\"1\"")],
            b"a",
        )
        .await;
        assert_eq!(tag(&r), "\"2\"");
        let r = call(
            &app,
            "PUT",
            "/api/sync",
            &[("authorization", auth), ("if-match", "\"1\"")],
            b"b",
        )
        .await;
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);
        let r = call(
            &app,
            "GET",
            "/api/sync",
            &[("authorization", auth), ("if-none-match", "\"1\"")],
            b"",
        )
        .await;
        assert_eq!(tag(&r), "\"2\"");
        assert_eq!(bytes(r).await, b"a");

        // Another code is another blob.
        let other = format!("Bearer {}", TOKEN.replace('0', "f"));
        let r = call(&app, "GET", "/api/sync", &[("authorization", &other)], b"").await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);

        // Turned off: gone for every device, and not filled again.
        let r = call(&app, "DELETE", "/api/sync", &[("authorization", auth)], b"").await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let r = call(&app, "GET", "/api/sync", &[("authorization", auth)], b"").await;
        assert_eq!(r.status(), StatusCode::GONE);
        let r = call(
            &app,
            "PUT",
            "/api/sync",
            &[("authorization", auth), ("if-match", "\"3\"")],
            b"c",
        )
        .await;
        assert_eq!(r.status(), StatusCode::GONE);
        let r = call(
            &app,
            "PUT",
            "/api/sync",
            &[("authorization", auth), ("if-none-match", "*")],
            b"c",
        )
        .await;
        assert_eq!(r.status(), StatusCode::GONE);

        let big = vec![0u8; MAX_SYNC_BYTES + 1];
        let r = call(
            &app,
            "PUT",
            "/api/sync",
            &[("authorization", &other), ("if-none-match", "*")],
            &big,
        )
        .await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn readers_register_their_feeds_and_nothing_else() {
        let pool = crate::db::connect_memory().await;
        let app = router(pool.clone(), FeedProxy::new("test").unwrap(), None);
        let body = br#"{"feeds": ["https://a.example/feed.xml", "http://127.0.0.1/x", "file:///etc/passwd"]}"#;
        let r = call(
            &app,
            "POST",
            "/api/readers/feeds",
            &[("content-type", "application/json")],
            body,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&bytes(r).await).unwrap();
        // Not crawled yet: no blog to show for it.
        assert_eq!(v["blogs"], json!({ "https://a.example/feed.xml": null }));
        let urls: Vec<String> = sqlx::query_scalar("SELECT url FROM reader_feeds")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(urls, ["https://a.example/feed.xml"]);
        // Asking again only says it is still wanted.
        call(
            &app,
            "POST",
            "/api/readers/feeds",
            &[("content-type", "application/json")],
            body,
        )
        .await;
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reader_feeds")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
        // Without a crawler, nothing to refresh with.
        let r = call(
            &app,
            "POST",
            "/api/refresh",
            &[("content-type", "application/json")],
            br#"{"site":"a.example"}"#,
        )
        .await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn only_public_addresses() {
        let ok = |u: &str| allowed(&Url::parse(u).unwrap(), false);
        assert!(ok("https://example.com/feed.xml"));
        assert!(ok("http://93.184.215.14/rss"));
        assert!(ok("https://[2606:4700::1]/"));
        for bad in [
            "http://localhost:8080/",
            "http://127.0.0.1/",
            "http://10.1.2.3/",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://router/",
            "http://nas.local/",
            "http://printer.lan/",
            "http://user:pw@example.com/",
            "ftp://example.com/",
            "file:///etc/passwd",
        ] {
            assert!(!ok(bad), "{bad}");
        }
    }

    #[test]
    fn tells_feeds_from_pages() {
        assert_eq!(
            sniff(b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<rss version=\"2.0\">"),
            Kind::Feed
        );
        assert_eq!(sniff(b"<feed xmlns=\"http://www.w3.org/2005/Atom\">"), Kind::Feed);
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><rdf:RDF>"), Kind::Feed);
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><opml version=\"2.0\">"), Kind::Feed);
        assert_eq!(sniff(br#"{"version": "https://jsonfeed.org/version/1.1"}"#), Kind::Feed);
        assert_eq!(sniff(b"<!DOCTYPE html><html><head>"), Kind::Page);
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><urlset>"), Kind::Other);
        assert_eq!(sniff(b"{\"error\": 1}"), Kind::Other);
        assert_eq!(sniff(b"hello"), Kind::Other);
    }

    #[tokio::test]
    async fn fetches_feeds_and_finds_them_on_pages() {
        let hits = Arc::new(AtomicUsize::new(0));
        let site = {
            let hits = hits.clone();
            Router::new()
                .route(
                    "/feed.xml",
                    get(move || {
                        hits.fetch_add(1, Ordering::SeqCst);
                        async { ([(header::CONTENT_TYPE, "application/rss+xml; charset=gbk")], "<rss><channel/></rss>") }
                    }),
                )
                .route(
                    "/",
                    get(|| async {
                        axum::response::Html(
                            r#"<!doctype html><html><head><link rel="alternate" type="application/rss+xml" href="/feed.xml"></head></html>"#,
                        )
                    }),
                )
                .route("/text", get(|| async { "just text" }))
                .route(
                    "/busy",
                    get(|| async { (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "120")], "") }),
                )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, site).await.unwrap() });

        let open = router(
            crate::db::connect_memory().await,
            FeedProxy::build("test", true).unwrap(),
            None,
        );
        let get = |app: Router, path: String| async move {
            let uri = format!("/api/fetch?url={}", urlencode(&format!("http://{addr}{path}")));
            call(&app, "GET", &uri, &[], b"").await
        };

        let r = get(open.clone(), "/feed.xml".into()).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["x-kind"], "feed");
        assert_eq!(r.headers()["x-feed-type"], "application/rss+xml; charset=gbk");
        assert_eq!(r.headers()[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(r.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(bytes(r).await, b"<rss><channel/></rss>");
        // Kept a while: the site is asked once.
        get(open.clone(), "/feed.xml".into()).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let r = get(open.clone(), "/".into()).await;
        assert_eq!(r.headers()["x-kind"], "page");
        let v: serde_json::Value = serde_json::from_slice(&bytes(r).await).unwrap();
        assert_eq!(v["feeds"], json!([format!("http://{addr}/feed.xml")]));

        assert_eq!(
            get(open.clone(), "/text".into()).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            get(open.clone(), "/missing".into()).await.status(),
            StatusCode::BAD_GATEWAY
        );
        let r = get(open.clone(), "/busy".into()).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(r.headers()[header::RETRY_AFTER], "120");

        // The real thing will not reach this machine.
        let strict = router(crate::db::connect_memory().await, FeedProxy::new("test").unwrap(), None);
        assert_eq!(
            get(strict.clone(), "/feed.xml".into()).await.status(),
            StatusCode::FORBIDDEN
        );
        let r = call(&strict, "GET", "/api/fetch?url=not%20a%20url", &[], b"").await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let r = call(&strict, "GET", "/api/capabilities", &[], b"").await;
        assert_eq!(
            bytes(r).await,
            br#"{"fetch":true,"readers":false,"refresh":false,"stats":true,"sync":true}"#
        );
    }

    fn urlencode(s: &str) -> String {
        url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }
}
