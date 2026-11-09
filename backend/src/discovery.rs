//! Finds a blog's feed when the directory does not tell us.
//!
//! Cost per blog: one homepage request, then at most a handful of feed
//! candidates, stopping at the first that parses. The per-host gap in
//! [`PoliteClient`] applies to every one of these. A blog where nothing is
//! found is not retried for `discovery_retry_secs` (a week by default).

use std::collections::HashSet;
use std::time::Duration;

use bytes::Bytes;
use url::Url;

use crate::feed;
use crate::html;
use crate::http::{ACCEPT_FEED, ACCEPT_HTML, FetchError, Fetched, PoliteClient, Validators};

/// Feed paths used by common blog engines, most likely first:
/// Hexo, WordPress/Typecho, Halo/Hexo, Hugo, Jekyll, Ghost.
const COMMON_PATHS: &[&str] = &["atom.xml", "feed", "rss.xml", "index.xml", "feed.xml", "rss"];
const MAX_ADVERTISED: usize = 3;

pub struct Discovered {
    pub feed_url: Url,
    pub validators: Validators,
    pub body: Bytes,
    pub fresh_for: Option<Duration>,
}

/// A fetched HTML page; discovery hands the homepage on so the profile step
/// does not request it a second time.
pub struct Page {
    pub url: Url,
    pub html: String,
}

pub async fn fetch_page(http: &PoliteClient, url: &Url) -> Result<Page, FetchError> {
    Ok(match http.get(url, ACCEPT_HTML, None).await? {
        Fetched::Ok { final_url, body, .. } => Page {
            url: final_url,
            html: String::from_utf8_lossy(&body).into_owned(),
        },
        Fetched::NotModified { .. } => Page {
            url: url.clone(),
            html: String::new(),
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
            Probe::NotAFeed => {}
            // The server is struggling or refusing us; guessing more URLs would only add load.
            Probe::ServerTrouble => break,
        }
    }
    None
}

enum Probe {
    Found(Discovered),
    NotAFeed,
    ServerTrouble,
}

async fn try_feed(http: &PoliteClient, url: Url) -> Probe {
    match http.get(&url, ACCEPT_FEED, None).await {
        Ok(Fetched::Ok {
            body,
            validators,
            fresh_for,
            ..
        }) => match feed::parse(&body, &url) {
            Ok(_) => Probe::Found(Discovered {
                feed_url: url,
                validators,
                body,
                fresh_for,
            }),
            Err(_) => Probe::NotAFeed,
        },
        Ok(Fetched::NotModified { .. }) => Probe::NotAFeed,
        Err(e) => {
            tracing::debug!(%url, error = %e, "feed candidate rejected");
            // Only "nothing here" answers justify guessing the next path;
            // 403/429/5xx/timeouts mean stop.
            let missing = e
                .status()
                .is_some_and(|s| matches!(s.as_u16(), 400 | 404 | 405 | 406 | 410));
            if missing { Probe::NotAFeed } else { Probe::ServerTrouble }
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

    #[test]
    fn dir_url_variants() {
        let f = |s: &str| dir_url(&Url::parse(s).unwrap()).to_string();
        assert_eq!(f("https://a.com"), "https://a.com/");
        assert_eq!(f("https://a.com/blog"), "https://a.com/blog/");
        assert_eq!(f("https://a.com/blog/index.html?x=1"), "https://a.com/blog/");
        assert_eq!(f("https://a.com/blog/"), "https://a.com/blog/");
    }
}
