//! 中文独立博客列表 (https://github.com/timqian/chinese-independent-blogs),
//! from its CSV: `Introduction, Address, RSS feed, tags` (tags split by `;`).
//!
//! The list has no status of its own: every listed blog counts as alive, and
//! the crawler's failures and backoff deal with the ones that are gone. Many
//! of its blogs are 开往 members too; for those, 开往's name and the feed we
//! already read stay (`fills_in`).

use futures::FutureExt;
use futures::future::BoxFuture;
use url::Url;

use super::{Directory, DirectoryEntry, DirectoryFetch};
use crate::config::IndependentConfig;
use crate::http::{Fetched, PoliteClient, Validators};
use crate::util::{parse_http_url, site_key};

const ACCEPT_CSV: &str = "text/csv, text/plain;q=0.9, */*;q=0.5";

pub struct Independent {
    cfg: IndependentConfig,
}

impl Independent {
    pub fn new(cfg: IndependentConfig) -> Self {
        Self { cfg }
    }

    async fn fetch_list(
        &self,
        http: &PoliteClient,
        url: &str,
        validators: Option<&Validators>,
    ) -> anyhow::Result<DirectoryFetch> {
        let url = Url::parse(url)?;
        match http.get(&url, ACCEPT_CSV, validators).await? {
            Fetched::Ok { body, validators, .. } => Ok(DirectoryFetch::Fetched {
                entries: parse(&body)?,
                validators,
            }),
            Fetched::NotModified { .. } => Ok(DirectoryFetch::NotModified),
        }
    }
}

/// Columns are found by their header, so a reordered file still reads; a
/// file without addresses is an error, not an empty list (which would take
/// every blog of the list off the site).
fn parse(body: &[u8]) -> anyhow::Result<Vec<DirectoryEntry>> {
    let mut rows = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(true)
        .from_reader(body);
    let headers = rows.headers()?.clone();
    let column = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
    let Some(address) = column("Address") else {
        anyhow::bail!("chinese-independent-blogs: no Address column in {headers:?}");
    };
    let (intro, feed, tags) = (column("Introduction"), column("RSS feed"), column("tags"));

    let mut out = Vec::new();
    for row in rows.records() {
        let row = row?;
        let field = |i: Option<usize>| i.and_then(|i| row.get(i)).unwrap_or("").trim();
        let Some(homepage) = parse_http_url(field(Some(address))) else {
            continue;
        };
        let Some(external_id) = site_key(homepage.as_str()) else {
            continue;
        };
        let name = field(intro);
        out.push(DirectoryEntry {
            external_id,
            name: if name.is_empty() {
                homepage.host_str().unwrap_or_default().to_string()
            } else {
                name.to_string()
            },
            name_from_feed: name.is_empty(),
            feed_url: parse_http_url(field(feed)),
            tags: field(tags)
                .split(';')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect(),
            homepage,
            status: "LISTED".into(),
            alive: true,
            kind: None,
        });
    }
    if out.is_empty() {
        anyhow::bail!("chinese-independent-blogs: the list has no blogs");
    }
    Ok(out)
}

impl Directory for Independent {
    fn id(&self) -> &'static str {
        "independent"
    }

    fn name(&self) -> &'static str {
        "中文独立博客列表"
    }

    fn homepage(&self) -> &'static str {
        "https://github.com/timqian/chinese-independent-blogs"
    }

    fn fills_in(&self) -> bool {
        true
    }

    fn fetch<'a>(
        &'a self,
        http: &'a PoliteClient,
        validators: Option<&'a Validators>,
    ) -> BoxFuture<'a, anyhow::Result<DirectoryFetch>> {
        async move {
            match self.fetch_list(http, &self.cfg.url, validators).await {
                Ok(list) => Ok(list),
                Err(e) => {
                    let Some(fallback) = &self.cfg.fallback_url else {
                        return Err(e);
                    };
                    tracing::warn!(error = %e, "chinese-independent-blogs list failed, trying mirror");
                    // The mirror has its own validators; fetch it in full.
                    self.fetch_list(http, fallback, None).await
                }
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_list() {
        let list = parse(include_bytes!("../../tests/fixtures/independent_blogs.csv")).unwrap();
        let ids: Vec<&str> = list.iter().map(|e| e.external_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "blog.example.com",
                "nofeed.example",
                "spaces.example",
                "comma.example",
                "noname.example"
            ],
            "the row without a usable address is left out"
        );

        let first = &list[0];
        assert_eq!(first.name, "示例博客");
        assert_eq!(first.homepage.as_str(), "https://blog.example.com/");
        assert_eq!(
            first.feed_url.as_ref().unwrap().as_str(),
            "https://blog.example.com/feed.xml"
        );
        assert_eq!(first.tags, ["编程", "前端"]);
        assert!(first.alive && !first.name_from_feed);

        assert_eq!(list[1].feed_url, None, "an empty feed column: discovered later");
        assert_eq!(list[2].tags, ["随笔", "思考"]);
        assert_eq!(list[3].name, "名字, 带逗号");
        assert!(list[4].name_from_feed);
        assert_eq!(list[4].name, "noname.example");
    }

    #[test]
    fn columns_by_header() {
        let list =
            parse("tags, RSS feed, Address\n生活, https://a.example/rss, https://a.example/\n".as_bytes()).unwrap();
        assert_eq!(list[0].external_id, "a.example");
        assert_eq!(list[0].feed_url.as_ref().unwrap().as_str(), "https://a.example/rss");
        assert_eq!(list[0].name, "a.example");
    }

    #[test]
    fn a_list_without_blogs_is_an_error() {
        assert!(parse(b"Introduction, Address, RSS feed, tags\n").is_err());
        assert!(parse(b"<html>rate limited</html>\n").is_err());
    }
}
