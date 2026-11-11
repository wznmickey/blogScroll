//! What readers do on the site, counted without them in it, and what it adds
//! up to: how much readers take to each post, blog, tag, category, search.
//!
//! - Browsers that did not turn it off (设置 → 统计) send what happened
//!   (`POST /api/events`, readers.rs; on the static site the Worker): a visit
//!   a day, time on the site and on posts, opening and reading posts, likes,
//!   saves, follows, searches, the settings in use. With a random id of the
//!   browser's own, unrelated to sync, hashed here: only to count visitors
//!   (and readers of each own feed) once.
//! - Kept as counts per day (`stats`), keyed by addresses (a post's, a blog's
//!   `site_key`) rather than database ids, so they survive a database built
//!   anew. The Worker commits its counts to the backup repository every hour
//!   (stats/<day>.json, stats/subscribers.json); restoring takes them in as
//!   source `site`, beside this server's own (`local`).
//! - `refresh_heat` turns the last HEAT_DAYS into `post_heat` (each post's
//!   standing, 0-1, shown as 热度 and weighed by 推荐 when the reader wants
//!   others' reading to count) and `blog_heat`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::util::now;

/// Days of counts that make up heat and what's trending.
pub const HEAT_DAYS: i64 = 30;
/// Visitors are counted per day; their ids go after this.
const VISITOR_DAYS: i64 = 2;

#[derive(Debug, Deserialize)]
pub struct Event {
    /// visit | dwell | dwell_post | open | read | like | save | follow | search | setting
    pub t: String,
    /// A post's address, a blog's `site_key`, a search, `name=value`.
    #[serde(default)]
    pub k: Option<String>,
    /// Seconds for dwell; +1 / -1 for like, save, follow (undone).
    #[serde(default)]
    pub n: Option<f64>,
}

/// A browser's random id, as kept here: hashed, never as sent.
pub fn who(id: &str) -> Option<String> {
    let id = id.trim();
    (id.len() >= 16 && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| hex::encode(Sha256::digest(format!("blogscroll reader {id}").as_bytes())))
}

pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn day_before(days: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

/// One interaction, checked: what it counts as (metric, key, amount), or None.
pub fn count_of(e: &Event) -> Option<(&'static str, String, f64)> {
    let key = e.k.as_deref().map(str::trim).unwrap_or_default();
    let n = e.n.filter(|n| n.is_finite());
    let addr = |k: &str| (k.starts_with("https://") || k.starts_with("http://")) && k.len() <= 500;
    let sign = || n.map_or(1.0, |n| if n < 0.0 { -1.0 } else { 1.0 });
    match e.t.as_str() {
        "visit" => Some(("visit", String::new(), 1.0)),
        "dwell" => Some(("dwell", String::new(), n?.clamp(0.0, 3600.0))),
        "dwell_post" if addr(key) => Some(("dwell_post", key.into(), n?.clamp(0.0, 3600.0))),
        "open" if addr(key) => Some(("open", key.into(), 1.0)),
        "read" if addr(key) => Some(("read", key.into(), 1.0)),
        "like" if addr(key) => Some(("like", key.into(), sign())),
        "save" if addr(key) => Some(("save", key.into(), sign())),
        "follow" if !key.is_empty() && key.len() <= 300 => Some(("follow", key.into(), sign())),
        "search" => {
            let term: String = key.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
            let term: String = term.chars().take(40).collect();
            (!term.is_empty()).then_some(("search", term, 1.0))
        }
        "setting" if !key.is_empty() && key.len() <= 80 => Some(("setting", key.into(), 1.0)),
        _ => None,
    }
}

/// This server's own visitors' interactions (`who`: their browser's id, hashed).
pub async fn record(pool: &SqlitePool, who: Option<&str>, events: &[Event]) -> sqlx::Result<usize> {
    let day = today();
    let mut tx = pool.begin().await?;
    let mut counted = 0;
    for e in events {
        let Some((metric, key, n)) = count_of(e) else {
            continue;
        };
        if metric == "visit" {
            // Once a day per browser.
            let Some(who) = who else { continue };
            let new = sqlx::query("INSERT OR IGNORE INTO visitors (day, who) VALUES (?, ?)")
                .bind(&day)
                .bind(who)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if new == 0 {
                continue;
            }
        }
        sqlx::query(
            "INSERT INTO stats (source, day, metric, key, n) VALUES ('local', ?, ?, ?, ?)
             ON CONFLICT (source, day, metric, key) DO UPDATE SET n = n + excluded.n",
        )
        .bind(&day)
        .bind(metric)
        .bind(&key)
        .bind(n)
        .execute(&mut *tx)
        .await?;
        counted += 1;
    }
    sqlx::query("DELETE FROM visitors WHERE day < ?")
        .bind(day_before(VISITOR_DAYS))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(counted)
}

/// A day as the Worker commits it (stats/<day>.json).
#[derive(Debug, Serialize, Deserialize)]
pub struct DayFile {
    pub day: String,
    /// metric → key → amount
    pub metrics: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
}

/// stats/subscribers.json: how many readers have each own feed.
#[derive(Debug, Serialize, Deserialize)]
pub struct Subscribers {
    pub feeds: std::collections::BTreeMap<String, i64>,
}

/// Takes in the static site's counts from the backup repository (stats/):
/// each day there replaces what was taken in for it before.
pub async fn import(pool: &SqlitePool, dir: &Path) -> anyhow::Result<usize> {
    let stats = dir.join("stats");
    let Ok(read) = std::fs::read_dir(&stats) else {
        return Ok(0);
    };
    let mut days = 0;
    let mut tx = pool.begin().await?;
    for entry in read.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if !name.ends_with(".json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        if name == "subscribers.json" {
            let Ok(s) = serde_json::from_str::<Subscribers>(&text) else {
                tracing::warn!(file = %path.display(), "unreadable subscribers file");
                continue;
            };
            sqlx::query("DELETE FROM site_subscribers").execute(&mut *tx).await?;
            for (url, n) in s.feeds {
                sqlx::query("INSERT INTO site_subscribers (url, n) VALUES (?, ?)")
                    .bind(url)
                    .bind(n)
                    .execute(&mut *tx)
                    .await?;
            }
            continue;
        }
        let Ok(file) = serde_json::from_str::<DayFile>(&text) else {
            tracing::warn!(file = %path.display(), "unreadable stats file");
            continue;
        };
        sqlx::query("DELETE FROM stats WHERE source = 'site' AND day = ?")
            .bind(&file.day)
            .execute(&mut *tx)
            .await?;
        for (metric, keys) in &file.metrics {
            for (key, n) in keys {
                sqlx::query("INSERT INTO stats (source, day, metric, key, n) VALUES ('site', ?, ?, ?, ?)")
                    .bind(&file.day)
                    .bind(metric)
                    .bind(key)
                    .bind(n)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        days += 1;
    }
    tx.commit().await?;
    Ok(days)
}

/// Recomputes `post_heat` and `blog_heat` from the last HEAT_DAYS of counts.
pub async fn refresh_heat(pool: &SqlitePool) -> sqlx::Result<()> {
    let since = day_before(HEAT_DAYS);
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM post_heat").execute(&mut *tx).await?;
    // What one interaction is worth: opening 1, reading the original 2, a
    // like 4, a save 5, a minute on it 1.
    sqlx::query(
        "INSERT INTO post_heat (post_id, score, heat)
         SELECT post_id, score, cume_dist() OVER (ORDER BY score) FROM (
            SELECT p.id AS post_id,
                   SUM(s.n * CASE s.metric WHEN 'open' THEN 1 WHEN 'read' THEN 2 WHEN 'like' THEN 4
                                  WHEN 'save' THEN 5 WHEN 'dwell_post' THEN 1.0 / 60 ELSE 0 END) AS score
            FROM stats s JOIN posts p ON p.url = s.key
            WHERE s.day >= ? AND s.metric IN ('open', 'read', 'like', 'save', 'dwell_post')
            GROUP BY p.id
            HAVING score > 0)",
    )
    .bind(&since)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM blog_heat").execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO blog_heat (blog_id, followers, score)
         SELECT b.id, MAX(COALESCE(f.n, 0), 0), MAX(COALESCE(f.n, 0), 0) * 5 + COALESCE(h.s, 0)
         FROM blogs b
         LEFT JOIN (SELECT key, SUM(n) AS n FROM stats WHERE metric = 'follow' GROUP BY key) f ON f.key = b.site_key
         LEFT JOIN (SELECT p.blog_id, SUM(ph.score) AS s FROM post_heat ph JOIN posts p ON p.id = ph.post_id
                    GROUP BY p.blog_id) h ON h.blog_id = b.id
         WHERE COALESCE(f.n, 0) > 0 OR h.s > 0",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Readers of each own feed: this server's, and the static site's.
pub async fn subscribers(pool: &SqlitePool) -> sqlx::Result<std::collections::HashMap<String, i64>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT url, SUM(n) FROM (
            SELECT url, COUNT(*) AS n FROM reader_subs WHERE last_at >= ? GROUP BY url
            UNION ALL SELECT url, n FROM site_subscribers)
         GROUP BY url",
    )
    .bind(now() - crate::sources::readers::KEEP_DAYS * 86400)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// A reader's own feeds, for counting them (`who`: their browser's id, hashed);
/// `counted` false (they turned statistics off) forgets them.
pub async fn note_subscriptions(pool: &SqlitePool, who: &str, feeds: &[String], counted: bool) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM reader_subs WHERE who = ? AND url NOT IN (SELECT value FROM json_each(?))")
        .bind(who)
        .bind(if counted {
            serde_json::to_string(feeds).unwrap_or_default()
        } else {
            "[]".into()
        })
        .execute(&mut *tx)
        .await?;
    if counted {
        for url in feeds {
            sqlx::query(
                "INSERT INTO reader_subs (url, who, last_at) VALUES (?, ?, ?)
                 ON CONFLICT (url, who) DO UPDATE SET last_at = excluded.last_at",
            )
            .bind(url)
            .bind(who)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    async fn get(state: &crate::api::AppState, uri: &str) -> serde_json::Value {
        let resp = crate::api::router(state.clone())
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    fn ev(t: &str, k: &str, n: Option<f64>) -> Event {
        Event {
            t: t.into(),
            k: Some(k.into()),
            n,
        }
    }

    #[tokio::test]
    async fn what_readers_do_adds_up_to_heat_and_trending() {
        let state = crate::api::tests::seeded().await;
        let pool = &state.pool;
        let a = who("0123456789abcdef0123").unwrap();
        let b = who("fedcba98765432100123").unwrap();
        assert_ne!(a, b);
        assert!(who("short").is_none() && who("has spaces in it 123").is_none());

        let post = "https://b1.example/p/2";
        let events = [
            ev("visit", "", None),
            ev("open", post, None),
            ev("read", post, None),
            ev("like", post, Some(1.0)),
            ev("dwell_post", post, Some(120.0)),
            ev("open", "https://b2.example/p/1", None),
            ev("follow", "b2.example", Some(1.0)),
            ev("search", "  Rust   编程 ", None),
            ev("setting", "columns=4", None),
            ev("open", "not an address", None),
            ev("nonsense", "x", None),
        ];
        assert_eq!(record(pool, Some(&a), &events).await.unwrap(), 9);
        // The same browser again today: one visitor still; another one: two.
        record(pool, Some(&a), &[ev("visit", "", None)]).await.unwrap();
        record(
            pool,
            Some(&b),
            &[ev("visit", "", None), ev("search", "rust 编程", None)],
        )
        .await
        .unwrap();
        refresh_heat(pool).await.unwrap();

        let t = get(&state, "/api/trending").await;
        assert_eq!(t["visitors"]["today"], 2);
        // b1's post: opened, read, liked, two minutes: 1 + 2 + 4 + 2.
        assert_eq!(t["posts"][0]["url"], post);
        assert_eq!(t["posts"][0]["heat"], 1.0);
        assert_eq!(t["posts"][1]["url"], "https://b2.example/p/1");
        assert_eq!(t["searches"][0], serde_json::json!({ "term": "rust 编程", "n": 2.0 }));
        // b1: its post's 9; b2: a follow (5) and an opening.
        assert_eq!(t["blogs"][0]["key"], "b1.example");
        assert_eq!(
            (t["blogs"][1]["key"].as_str(), t["blogs"][1]["followers"].as_i64()),
            (Some("b2.example"), Some(1))
        );
        assert_eq!(t["blogs"][1]["score"], 6.0);
        assert!(
            t["tags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["term"] == "日记" || c["term"] == "旅行")
        );

        // Cards carry it; 推荐 weighs it unless the reader set it to 0.
        let latest = get(&state, "/api/posts?mode=latest&blog=1&limit=50").await;
        let hot = latest["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["url"] == post)
            .unwrap();
        assert_eq!(hot["heat"], 1.0);
        let mixed = get(&state, "/api/posts?limit=50&seed=3").await;
        let with = mixed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["url"] == post)
            .unwrap();
        assert_eq!(with["why"]["heat"], 10.0);
        let off = get(&state, "/api/posts?limit=50&seed=3&w=40,12,5,60,10,14,5,8,0").await;
        let without = off["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["url"] == post)
            .unwrap();
        assert_eq!(without["why"]["heat"], 0.0);
    }

    #[tokio::test]
    async fn the_static_sites_counts_come_in_from_the_backup() {
        let state = crate::api::tests::seeded().await;
        let pool = &state.pool;
        record(pool, None, &[ev("open", "https://b1.example/p/1", None)])
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!("stats-import-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("stats")).unwrap();
        let file = DayFile {
            day: today(),
            metrics: [(
                "open".to_string(),
                [("https://b1.example/p/3".to_string(), 5.0)].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
        };
        std::fs::write(
            dir.join(format!("stats/{}.json", today())),
            serde_json::to_vec(&file).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("stats/subscribers.json"),
            br#"{"feeds": {"https://own.example/feed.xml": 3}}"#,
        )
        .unwrap();
        assert_eq!(import(pool, &dir).await.unwrap(), 1);
        // Taken in twice: the same, not double.
        import(pool, &dir).await.unwrap();
        refresh_heat(pool).await.unwrap();
        let t = get(&state, "/api/trending").await;
        assert_eq!(t["posts"][0]["url"], "https://b1.example/p/3");
        assert_eq!(
            t["posts"][1]["url"], "https://b1.example/p/1",
            "this server's own counts stay"
        );
        assert_eq!(subscribers(pool).await.unwrap()["https://own.example/feed.xml"], 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn readers_of_own_feeds_are_counted_until_they_say_no() {
        let pool = crate::db::connect_memory().await;
        let feeds = [
            "https://a.example/feed".to_string(),
            "https://b.example/feed".to_string(),
        ];
        note_subscriptions(&pool, "x", &feeds, true).await.unwrap();
        note_subscriptions(&pool, "y", &feeds[..1], true).await.unwrap();
        let n = subscribers(&pool).await.unwrap();
        assert_eq!((n["https://a.example/feed"], n["https://b.example/feed"]), (2, 1));
        // y dropped it; x turned statistics off.
        note_subscriptions(&pool, "y", &[], true).await.unwrap();
        note_subscriptions(&pool, "x", &feeds, false).await.unwrap();
        assert!(subscribers(&pool).await.unwrap().is_empty());
    }
}
