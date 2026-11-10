//! Finds a blog's feed when the directory does not tell us.
//!
//! Cost per blog: one homepage request, then the feeds the page advertises
//! and the common paths below, stopping at the first that parses. An error on
//! one candidate does not stop the search, but the per-host gap in
//! [`PoliteClient`] applies to every request, and a `Retry-After` is waited
//! out (briefly) before the next. A blog where nothing is found is not
//! retried for `discovery_retry_secs` (a week by default).

use std::collections::HashSet;
use std::time::Duration;

use bytes::Bytes;
use url::Url;

use crate::feed;
use crate::html;
use crate::http::{ACCEPT_FEED, ACCEPT_HTML, FetchError, Fetched, PoliteClient, Validators};

/// Bumped when where or how feeds are looked for changes (more paths, say):
/// blogs where none was found are then searched again straight away (see
/// `Crawler::catch_up_on_new_rules`).
pub const VERSION: u32 = 2;

/// Feed paths used by common blog engines, most likely first: Hexo,
/// WordPress/Typecho, Halo/Hexo, Hugo, Jekyll, Ghost; then less common ones
/// seen on blogs that do advertise their feed (Hexo's `rss2.xml`, PHP
/// engines, per-language feeds).
const COMMON_PATHS: &[&str] = &[
    "atom.xml",
    "feed",
    "rss.xml",
    "index.xml",
    "feed.xml",
    "rss",
    "rss2.xml",
    "atom",
    "feed.php",
    "rss.php",
    "posts.rss",
    "blog.xml",
    "rss.en.xml",
];
const MAX_ADVERTISED: usize = 3;
/// Longest `Retry-After` waited out before trying the next candidate.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

pub struct Discovered {
    pub feed_url: Url,
    pub validators: Validators,
    pub body: Bytes,
    pub fresh_for: Option<Duration>,
    pub frame_blocked: bool,
}

/// A fetched HTML page; discovery hands the homepage on so the profile step
/// does not request it a second time.
pub struct Page {
    pub url: Url,
    pub html: String,
    /// Whether the site forbids showing it in a frame; `None` if not fetched.
    pub frame_blocked: Option<bool>,
}

pub async fn fetch_page(http: &PoliteClient, url: &Url) -> Result<Page, FetchError> {
    Ok(match http.get(url, ACCEPT_HTML, None).await? {
        Fetched::Ok {
            final_url,
            body,
            frame_blocked,
            ..
        } => Page {
            url: final_url,
            html: String::from_utf8_lossy(&body).into_owned(),
            frame_blocked: Some(frame_blocked),
        },
        Fetched::NotModified { .. } => Page {
            url: url.clone(),
            html: String::new(),
            frame_blocked: None,
        },
    })
}

/// `Ok((None, _))` means the site answered but has no feed we could find.
/// `Err` means the homepage itself could not be fetched.
pub async fn discover(http: &PoliteClient, homepage: &Url) -> Result<(Option<Discovered>, Page), FetchError> {
    let page = fetch_page(http, homepage).await?;
    let found = find_feed(http, &page).await;
    Ok((found, page))
}

async fn find_feed(http: &PoliteClient, page: &Page) -> Option<Discovered> {
    let page_url = &page.url;
    let advertised = html::feed_links(&page.html, page_url);
    let base = dir_url(page_url);
    let guessed = COMMON_PATHS.iter().filter_map(|p| base.join(p).ok());

    let mut tried = HashSet::new();
    for candidate in advertised.into_iter().take(MAX_ADVERTISED).chain(guessed) {
        if !tried.insert(candidate.clone()) {
            continue;
        }
        match try_feed(http, candidate).await {
            Probe::Found(found) => return Some(found),
            Probe::NotAFeed { retry_after: None } => {}
            Probe::NotAFeed {
                retry_after: Some(wait),
            } => tokio::time::sleep(wait.min(MAX_RETRY_WAIT)).await,
        }
    }
    None
}

enum Probe {
    Found(Discovered),
    /// Not a feed, or an error; the next candidate is tried after `retry_after`.
    NotAFeed {
        retry_after: Option<Duration>,
    },
}

async fn try_feed(http: &PoliteClient, url: Url) -> Probe {
    match http.get(&url, ACCEPT_FEED, None).await {
        Ok(Fetched::Ok {
            body,
            validators,
            fresh_for,
            frame_blocked,
            ..
        }) => match feed::parse(&body, &url) {
            Ok(_) => Probe::Found(Discovered {
                feed_url: url,
                validators,
                body,
                fresh_for,
                frame_blocked,
            }),
            Err(_) => Probe::NotAFeed { retry_after: None },
        },
        Ok(Fetched::NotModified { .. }) => Probe::NotAFeed { retry_after: None },
        Err(e) => {
            tracing::debug!(%url, error = %e, "feed candidate rejected");
            Probe::NotAFeed {
                retry_after: e.retry_after(),
            }
        }
    }
}

/// `https://a.com/blog` -> `https://a.com/blog/`, `https://a.com/index.html` -> `https://a.com/`
pub fn dir_url(url: &Url) -> Url {
    let mut u = url.clone();
    u.set_query(None);
    u.set_fragment(None);
    let path = u.path().to_string();
    if !path.ends_with('/') {
        let last = path.rsplit('/').next().unwrap_or_default();
        if last.contains('.') {
            let _ = u.path_segments_mut().map(|mut s| {
                s.pop().push("");
            });
        } else {
            u.set_path(&format!("{path}/"));
        }
    }
    u
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A site whose first guesses fail in every way but whose `rss2.xml` works.
    #[tokio::test]
    async fn keeps_trying_past_errors_and_finds_less_common_paths() {
        use axum::Router;
        use axum::http::StatusCode;
        use axum::routing::get;
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let app = Router::new()
            .route(
                "/",
                get(|| async { "<html><head><title>no feed link</title></head></html>" }),
            )
            .route("/atom.xml", get(|| async { StatusCode::FORBIDDEN }))
            .route("/feed", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
            .route("/rss.xml", get(|| async { StatusCode::TOO_MANY_REQUESTS }))
            .route(
                "/rss2.xml",
                get(|| async {
                    r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
                    <item><title>a</title><link>https://x.example/a</link></item></channel></rss>"#
                }),
            )
            .layer(axum::middleware::from_fn(
                move |req: axum::extract::Request, next: axum::middleware::Next| {
                    log.lock().unwrap().push(req.uri().path().to_string());
                    next.run(req)
                },
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let cfg = crate::config::CrawlerConfig {
            per_host_interval_secs: 0,
            ..Default::default()
        };
        let http = PoliteClient::new(&cfg).unwrap();
        let home = Url::parse(&format!("http://{addr}/")).unwrap();
        let (found, _) = discover(&http, &home).await.unwrap();
        assert_eq!(found.unwrap().feed_url.path(), "/rss2.xml");
        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            [
                "/",
                "/atom.xml",
                "/feed",
                "/rss.xml",
                "/index.xml",
                "/feed.xml",
                "/rss",
                "/rss2.xml"
            ]
        );
    }

    #[test]
    fn dir_url_variants() {
        let f = |s: &str| dir_url(&Url::parse(s).unwrap()).to_string();
        assert_eq!(f("https://a.com"), "https://a.com/");
        assert_eq!(f("https://a.com/blog"), "https://a.com/blog/");
        assert_eq!(f("https://a.com/blog/index.html?x=1"), "https://a.com/blog/");
        assert_eq!(f("https://a.com/blog/"), "https://a.com/blog/");
    }
}
