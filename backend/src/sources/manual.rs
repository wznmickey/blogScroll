//! Feeds you add yourself, not only blogs: news sites, podcasts, newsletters.
//! Listed in the config (`[[sources.feeds]]`) or in OPML files exported from
//! a feed reader (`[[sources.opml]]`). Nothing is fetched to build the list;
//! each feed is then crawled like any other, with the same politeness rules.

use std::collections::HashSet;
use std::sync::LazyLock;

use futures::FutureExt;
use futures::future::BoxFuture;
use regex::Regex;
use url::Url;

use super::{Directory, DirectoryEntry, DirectoryFetch};
use crate::analyze::KINDS;
use crate::config::{FeedSource, OpmlSource};
use crate::http::{PoliteClient, Validators};
use crate::util::parse_http_url;

pub struct Manual {
    feeds: Vec<FeedSource>,
    opml: Vec<OpmlSource>,
}

impl Manual {
    pub fn new(feeds: Vec<FeedSource>, opml: Vec<OpmlSource>) -> Self {
        Self { feeds, opml }
    }

    fn entries(&self) -> anyhow::Result<Vec<DirectoryEntry>> {
        let mut all: Vec<FeedSource> = self.feeds.clone();
        for o in &self.opml {
            let text =
                std::fs::read_to_string(&o.path).map_err(|e| anyhow::anyhow!("reading {}: {e}", o.path.display()))?;
            for mut f in parse_opml(&text) {
                f.kind = f.kind.or_else(|| o.kind.clone());
                f.tags.extend(o.tags.iter().cloned());
                all.push(f);
            }
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for f in all {
            let Some(feed) = parse_http_url(&f.url) else {
                tracing::warn!(url = %f.url, "ignoring feed with an unusable URL");
                continue;
            };
            if !seen.insert(feed.clone()) {
                continue;
            }
            if let Some(k) = f.kind.as_deref().filter(|k| !KINDS.iter().any(|(id, _)| id == k)) {
                tracing::warn!(url = %f.url, kind = k, "unknown kind, using `other`");
            }
            let kind = f.kind.map(|k| {
                if KINDS.iter().any(|(id, _)| *id == k) {
                    k
                } else {
                    "other".into()
                }
            });
            // Without a homepage the feed URL identifies the source, so two
            // feeds from one site stay apart. The site link from the feed
            // replaces it for display after the first fetch.
            let homepage = f
                .homepage
                .as_deref()
                .and_then(parse_http_url)
                .unwrap_or_else(|| feed.clone());
            let name = f.name.clone().filter(|n| !n.trim().is_empty());
            out.push(DirectoryEntry {
                external_id: feed.to_string(),
                name: name
                    .clone()
                    .unwrap_or_else(|| feed.host_str().unwrap_or_default().to_string()),
                homepage,
                feed_url: Some(feed),
                tags: f.tags,
                status: "CONFIG".into(),
                alive: true,
                kind,
                name_from_feed: name.is_none(),
            });
        }
        Ok(out)
    }
}

impl Directory for Manual {
    fn id(&self) -> &'static str {
        "feeds"
    }

    fn name(&self) -> &'static str {
        "自选订阅"
    }

    fn homepage(&self) -> &'static str {
        ""
    }

    fn is_local(&self) -> bool {
        true
    }

    fn fetch<'a>(
        &'a self,
        _http: &'a PoliteClient,
        _validators: Option<&'a Validators>,
    ) -> BoxFuture<'a, anyhow::Result<DirectoryFetch>> {
        async move {
            Ok(DirectoryFetch::Fetched {
                entries: self.entries()?,
                validators: Validators::default(),
            })
        }
        .boxed()
    }
}

static OUTLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<outline\b([^>]*)>").unwrap());
static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"([\w:]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());

/// Every `<outline xmlUrl=...>` in an OPML document; folders are skipped.
pub fn parse_opml(text: &str) -> Vec<FeedSource> {
    OUTLINE
        .captures_iter(text)
        .filter_map(|c| {
            let attrs = c.get(1)?.as_str();
            let get = |name: &str| {
                ATTR.captures_iter(attrs)
                    .find(|a| a[1].eq_ignore_ascii_case(name))
                    .and_then(|a| a.get(2).or(a.get(3)))
                    .map(|v| unescape(v.as_str()))
                    .filter(|v| !v.trim().is_empty())
            };
            Some(FeedSource {
                url: get("xmlUrl")?,
                name: get("title").or_else(|| get("text")),
                homepage: get("htmlUrl").filter(|h| Url::parse(h).is_ok()),
                kind: None,
                tags: Vec::new(),
            })
        })
        .collect()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head><title>Subscriptions</title></head>
  <body>
    <outline text="科技">
      <outline type="rss" text="少数派" title="少数派" xmlUrl="https://sspai.com/feed" htmlUrl="https://sspai.com"/>
      <outline type="rss" text="A &amp; B" xmlUrl='https://ab.example/rss?x=1&amp;y=2'/>
    </outline>
  </body>
</opml>"#;

    #[test]
    fn opml_outlines_become_feeds() {
        let feeds = parse_opml(OPML);
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].url, "https://sspai.com/feed");
        assert_eq!(feeds[0].name.as_deref(), Some("少数派"));
        assert_eq!(feeds[0].homepage.as_deref(), Some("https://sspai.com"));
        assert_eq!(feeds[1].url, "https://ab.example/rss?x=1&y=2");
        assert_eq!(feeds[1].name.as_deref(), Some("A & B"));
    }

    #[test]
    fn config_feeds_become_entries() {
        let m = Manual::new(
            vec![
                FeedSource {
                    url: "https://news.example/rss".into(),
                    name: None,
                    homepage: None,
                    kind: Some("news".into()),
                    tags: vec!["科技".into()],
                },
                FeedSource {
                    url: "https://news.example/rss".into(),
                    name: Some("duplicate".into()),
                    homepage: None,
                    kind: None,
                    tags: vec![],
                },
                FeedSource {
                    url: "https://pod.example/feed.xml".into(),
                    name: Some("播客".into()),
                    homepage: Some("https://pod.example/".into()),
                    kind: Some("radio".into()),
                    tags: vec![],
                },
            ],
            vec![],
        );
        let e = m.entries().unwrap();
        assert_eq!(e.len(), 2, "the same feed twice is one source");
        assert_eq!(e[0].homepage.as_str(), "https://news.example/rss");
        assert!(e[0].name_from_feed);
        assert_eq!(e[0].kind.as_deref(), Some("news"));
        assert_eq!(e[1].name, "播客");
        assert!(!e[1].name_from_feed);
        assert_eq!(e[1].kind.as_deref(), Some("other"), "unknown kinds fall back");
    }
}
