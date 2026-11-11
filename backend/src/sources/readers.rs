//! 读者订阅: feeds readers added themselves (设置 → 来源 → 我添加的来源),
//! crawled like every other blog: whole history, categories, tags,
//! recommendations. A blog no other directory lists is private (`blogs.private`,
//! see db.rs): it appears only for readers who ask for it by id.
//!
//! The list is every feed some reader asked for lately: those registered with
//! this server (`reader_feeds`, readers.rs), plus, for the static site, a
//! file the crawl workflow writes from the Worker's database (a line per
//! feed: its address, and when it was first and last asked for, tab
//! separated), taken into `reader_feeds` too. A feed this server already
//! crawls (an old 自选订阅, say) keeps its blog and history: it is matched by
//! its feed address. Which feeds there are, and how crawling them goes, is in
//! [`report`] (readers.md in the backup, `feed-server readers`).

use std::path::PathBuf;

use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::SqlitePool;
use url::Url;

use super::{Directory, DirectoryEntry, DirectoryFetch};
use crate::http::{PoliteClient, Validators};
use crate::util::{now, parse_http_url, site_key};

/// Feeds nobody asked for in this long are no longer crawled.
pub const KEEP_DAYS: i64 = 90;
/// At most this many (registrations are capped too, see readers.rs).
const MAX_FEEDS: usize = 5000;

pub struct Readers {
    pool: SqlitePool,
    file: Option<PathBuf>,
}

impl Readers {
    pub fn new(pool: SqlitePool, file: Option<PathBuf>) -> Self {
        Self { pool, file }
    }

    /// None when the list is not known here: the file it comes from is
    /// missing (a run that did not fetch it), so nothing changes.
    async fn entries(&self) -> anyhow::Result<Option<Vec<DirectoryEntry>>> {
        let mut urls: Vec<String> = sqlx::query_scalar("SELECT url FROM reader_feeds WHERE last_at >= ? ORDER BY url")
            .bind(now() - KEEP_DAYS * 86400)
            .fetch_all(&self.pool)
            .await?;
        if let Some(path) = &self.file {
            let text = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => anyhow::bail!("reading {}: {e}", path.display()),
            };
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let mut cols = line.split('\t');
                let url = cols.next().unwrap_or_default().trim().to_string();
                let mut time = || cols.next().and_then(|t| t.trim().parse::<i64>().ok()).unwrap_or(now());
                let (first, last) = (time(), time());
                // Here too, for the report.
                sqlx::query(
                    "INSERT INTO reader_feeds (url, first_at, last_at) VALUES (?, ?, ?)
                     ON CONFLICT (url) DO UPDATE SET first_at = MIN(first_at, excluded.first_at),
                         last_at = MAX(last_at, excluded.last_at)",
                )
                .bind(&url)
                .bind(first)
                .bind(last)
                .execute(&self.pool)
                .await?;
                urls.push(url);
            }
        }
        urls.sort();
        urls.dedup();
        let mut out = Vec::new();
        for raw in urls.into_iter().take(MAX_FEEDS) {
            let Some(feed) = parse_http_url(&raw) else {
                continue;
            };
            // Known already (crawled before, from here or another directory):
            // the same blog as it is, so its history stays with it.
            let known: Option<(String, String, String, bool)> = sqlx::query_as(
                "SELECT site_key, homepage_url, name, from_feed_meta FROM blogs WHERE feed_url = ? LIMIT 1",
            )
            .bind(feed.as_str())
            .fetch_optional(&self.pool)
            .await?;
            // Otherwise the feed's address names it (its title and site link
            // come with the first fetch); never an address a reader merely
            // claims, which could take over another blog.
            let (homepage, name, name_from_feed) = match known {
                Some((key, home, name, from_feed)) => {
                    // An address that names that same blog (blogs are known by `site_key`).
                    let same = [home, feed.to_string(), format!("https://{key}")]
                        .into_iter()
                        .filter_map(|u| Url::parse(&u).ok())
                        .find(|u| site_key(u.as_str()).as_deref() == Some(key.as_str()));
                    (same.unwrap_or_else(|| feed.clone()), name, from_feed)
                }
                None => (feed.clone(), feed.host_str().unwrap_or_default().to_string(), true),
            };
            out.push(DirectoryEntry {
                // As the reader's browser sent it: the export names the file
                // they read after it (export.rs `own_file`).
                external_id: raw,
                name,
                homepage,
                feed_url: Some(feed),
                tags: Vec::new(),
                status: "READER".into(),
                alive: true,
                kind: None,
                name_from_feed,
            });
        }
        Ok(Some(out))
    }
}

/// Every feed readers asked for lately, and how crawling it goes, as a
/// Markdown table: for the site's owner (readers.md in the backup,
/// `feed-server readers`). Who asked is not known here.
pub async fn report(pool: &SqlitePool) -> anyhow::Result<String> {
    #[derive(sqlx::FromRow)]
    struct Row {
        url: String,
        first_at: i64,
        last_at: i64,
        name: Option<String>,
        private: Option<bool>,
        posts: Option<i64>,
        newest: Option<i64>,
        last_success_at: Option<i64>,
        failures: Option<i64>,
        last_error: Option<String>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT f.url, f.first_at, f.last_at, b.name, b.private,
                (SELECT COUNT(*) FROM posts p WHERE p.blog_id = b.id) AS posts,
                (SELECT MAX(p.sort_at) FROM posts p WHERE p.blog_id = b.id) AS newest,
                b.last_success_at, b.failures, b.last_error
         FROM reader_feeds f
         LEFT JOIN directory_entries d ON d.directory = ? AND d.external_id = f.url
         LEFT JOIN blogs b ON b.id = d.blog_id
         WHERE f.last_at >= ?
         ORDER BY f.first_at DESC, f.url",
    )
    .bind(crate::db::READERS)
    .bind(now() - KEEP_DAYS * 86400)
    .fetch_all(pool)
    .await?;
    let rows: Vec<Row> = rows.into_iter().filter(|r| parse_http_url(&r.url).is_some()).collect();
    let day =
        |t: i64| chrono::DateTime::from_timestamp(t, 0).map_or_else(String::new, |d| d.format("%Y-%m-%d").to_string());
    let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
    let only = rows.iter().filter(|r| r.private == Some(true)).count();
    let mut out = format!(
        "# 读者订阅\n\n\
         读者在「设置 → 来源 → 我添加的来源」里添加的 RSS。本站像收录的博客一样抓取、分析、备份，\
         只给添加的人看（只被这里列着的博客不进任何公开的列表、搜索和导出）。\
         这里只有地址：不知道是谁、有几个人添加的。90 天没人再登记的不再抓，也不再列在这里\
         （已抓的文章留在 sites/ 里）。这个文件由备份自动生成，每次覆盖。\n\n\
         共 {} 个，其中 {} 个只有读者订阅，其余本站本来就收录。\n\n\
         | RSS | 博客 | 文章 | 最近一篇 | 抓取 | 首次登记 | 最近登记 |\n|---|---|---|---|---|---|---|\n",
        rows.len(),
        only
    );
    for r in &rows {
        let blog = match (&r.name, r.private) {
            (Some(n), Some(true)) => format!("{}（只给订阅者）", cell(n)),
            (Some(n), _) => cell(n),
            (None, _) => "—".into(),
        };
        let crawl = match (r.name.is_some(), r.last_success_at, r.failures.unwrap_or(0)) {
            (false, ..) => "还没抓".to_string(),
            (true, _, f) if f > 0 => format!("出错 {f} 次：{}", cell(r.last_error.as_deref().unwrap_or(""))),
            (true, Some(t), _) => format!("{} 读过", day(t)),
            (true, None, _) => "还没读到".to_string(),
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            cell(&r.url),
            blog,
            r.posts.unwrap_or(0),
            r.newest.map(day).unwrap_or_default(),
            crawl,
            day(r.first_at),
            day(r.last_at),
        ));
    }
    Ok(out)
}

impl Directory for Readers {
    fn id(&self) -> &'static str {
        crate::db::READERS
    }

    fn name(&self) -> &'static str {
        "读者订阅"
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
            Ok(match self.entries().await? {
                Some(entries) => DirectoryFetch::Fetched {
                    entries,
                    validators: Validators::default(),
                },
                None => DirectoryFetch::NotModified,
            })
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    use url::Url;

    use super::*;
    use crate::feed::ParsedItem;

    async fn get(state: &crate::api::AppState, uri: &str) -> serde_json::Value {
        let resp = crate::api::router(state.clone())
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn ids(v: &serde_json::Value) -> Vec<i64> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_i64().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn readers_feeds_are_crawled_but_shown_only_to_them() {
        let state = crate::api::tests::seeded().await;
        let pool = &state.pool;
        let now = now();
        // Blog 2 is on the site already; own.example is nobody else's.
        sqlx::query("UPDATE blogs SET feed_url = 'https://b2.example/feed.xml' WHERE id = 2")
            .execute(pool)
            .await
            .unwrap();
        for url in [
            "https://own.example/feed.xml",
            "https://b2.example/feed.xml",
            "not a feed",
        ] {
            sqlx::query("INSERT INTO reader_feeds (url, first_at, last_at) VALUES (?, ?, ?)")
                .bind(url)
                .bind(now)
                .bind(now)
                .execute(pool)
                .await
                .unwrap();
        }
        // And one nobody asked for in months.
        sqlx::query("INSERT INTO reader_feeds (url, first_at, last_at) VALUES ('https://old.example/rss', 1, 1)")
            .execute(pool)
            .await
            .unwrap();

        let readers = Readers::new(pool.clone(), None);
        let entries = readers.entries().await.unwrap().unwrap();
        assert_eq!(
            entries.iter().map(|e| e.external_id.as_str()).collect::<Vec<_>>(),
            ["https://b2.example/feed.xml", "https://own.example/feed.xml"]
        );
        // The known feed keeps its blog (and its history).
        assert_eq!(entries[0].homepage.as_str(), "https://b2.example/");
        crate::db::save_directory(pool, crate::db::READERS, &entries, 3600, now)
            .await
            .unwrap();
        let own: i64 = sqlx::query_scalar("SELECT id FROM blogs WHERE feed_url = 'https://own.example/feed.xml'")
            .fetch_one(pool)
            .await
            .unwrap();
        let private = |id: i64| async move {
            sqlx::query_scalar::<_, bool>("SELECT private FROM blogs WHERE id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap()
        };
        assert!(private(own).await, "listed only by readers");
        assert!(!private(2).await, "still the site's");

        // The owner's list: every feed, which blog it became, nobody's name.
        let report = report(pool).await.unwrap();
        assert!(report.contains("共 2 个，其中 1 个只有读者订阅"), "{report}");
        assert!(
            report.contains("| https://own.example/feed.xml | own.example（只给订阅者） | 0 |"),
            "{report}"
        );
        assert!(
            report.contains("| https://b2.example/feed.xml | Blog 2 | 5 |"),
            "{report}"
        );

        // Crawled like any other blog.
        let item = ParsedItem {
            guid: "own-1".into(),
            url: Url::parse("https://own.example/p/1").unwrap(),
            title: "Own post".into(),
            excerpt: "摘要".into(),
            excerpt_html: None,
            cover: None,
            author: None,
            tags: vec!["旅行".into()],
            published: chrono::DateTime::from_timestamp(now - 60, 0),
            body_html: None,
            comments_url: None,
            comment_count: None,
        };
        let e = crate::db::Entry {
            position: 0,
            backlog_at: None,
            keywords_json: "[]",
            content: None,
        };
        let mut conn = pool.acquire().await.unwrap();
        crate::db::upsert_post(&mut conn, own, &item, now, &e).await.unwrap();
        drop(conn);
        crate::analyze::retag_all(pool).await.unwrap();
        let post: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE guid = 'own-1'")
            .fetch_one(pool)
            .await
            .unwrap();

        // Not for everyone: not in the feeds, the blog list or the counts.
        let latest = get(&state, "/api/posts?mode=latest&limit=50").await;
        assert!(!ids(&latest).contains(&post));
        assert!(!ids(&get(&state, "/api/posts?limit=50&seed=1").await).contains(&post));
        let blogs = get(&state, "/api/blogs?limit=50").await;
        assert!(!blogs["items"].as_array().unwrap().iter().any(|b| b["id"] == own));
        assert_eq!(get(&state, "/api/meta").await["stats"]["blogs"], 2);
        // For a reader who has it: everywhere, filters and all.
        let mine = get(&state, &format!("/api/posts?mode=latest&limit=50&own={own}")).await;
        assert_eq!(ids(&mine)[0], post);
        let mine = get(
            &state,
            &format!("/api/posts?limit=50&seed=1&own={own}&directories=travellings"),
        )
        .await;
        assert!(ids(&mine).contains(&post));
        // Its own page and the post itself open for anyone with the link.
        assert_eq!(
            ids(&get(&state, &format!("/api/posts?mode=latest&blog={own}")).await),
            [post]
        );
        assert_eq!(get(&state, &format!("/api/posts/{post}")).await["id"], post);

        // Asked for by nobody any more: off the list, then off the site.
        sqlx::query("UPDATE reader_feeds SET last_at = 1 WHERE url = 'https://own.example/feed.xml'")
            .execute(pool)
            .await
            .unwrap();
        let entries = readers.entries().await.unwrap().unwrap();
        crate::db::save_directory(pool, crate::db::READERS, &entries, 3600, now + 1)
            .await
            .unwrap();
        let active: bool = sqlx::query_scalar("SELECT active FROM blogs WHERE id = ?")
            .bind(own)
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(!active);

        // The crawl workflow's file: address, first and last asked for.
        let file = std::env::temp_dir().join(format!("readers-test-{}.txt", std::process::id()));
        std::fs::write(&file, format!("https://file.example/rss\t100\t{now}\n\n")).unwrap();
        let from_file = Readers::new(pool.clone(), Some(file.clone()));
        let urls: Vec<String> = from_file
            .entries()
            .await
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|e| e.external_id)
            .collect();
        assert!(urls.contains(&"https://file.example/rss".to_string()));
        let first: i64 = sqlx::query_scalar("SELECT first_at FROM reader_feeds WHERE url = 'https://file.example/rss'")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(first, 100);
        std::fs::remove_file(file).unwrap();

        // A list file that is not there (a run that did not fetch it): no change.
        let missing = Readers::new(pool.clone(), Some("/nonexistent/readers.txt".into()));
        assert!(missing.entries().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn directories_taken_out_of_the_config_list_nothing() {
        let state = crate::api::tests::seeded().await;
        let pool = &state.pool;
        assert_eq!(get(&state, "/api/meta").await["stats"]["blogs"], 2);
        let n = crate::db::retire_directories(pool, &["feeds", crate::db::READERS])
            .await
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(get(&state, "/api/meta").await["stats"]["blogs"], 0);
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(kept, 15, "posts stay (and stay in the backup)");
    }
}
