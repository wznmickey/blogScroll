use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{FromRow, SqlitePool};

use crate::config::Config;
use crate::feed::ParsedItem;
use crate::http::Validators;
use crate::sources::DirectoryEntry;
use crate::util::site_key;

pub async fn connect(path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let opts = SqliteConnectOptions::from_str("sqlite:")?
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
pub async fn connect_memory() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .foreign_keys(true);
    // A single connection: every in-memory connection would be its own database.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

#[derive(Debug, Clone, FromRow)]
pub struct BlogRow {
    pub id: i64,
    pub name: String,
    pub homepage_url: String,
    pub feed_url: Option<String>,
    pub feed_url_origin: Option<String>,
    pub content_policy: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub refresh_secs: i64,
    pub discovery_at: Option<i64>,
    pub failures: i64,
    pub content_hash: Option<String>,
    pub profile_at: Option<i64>,
    pub comment_system: Option<String>,
    pub comments_next_at: i64,
}

impl BlogRow {
    pub fn full_text_allowed(&self) -> bool {
        self.content_policy == "full_text"
    }
}

macro_rules! blog_columns {
    () => {
        "id, name, homepage_url, feed_url, feed_url_origin, content_policy, \
         etag, last_modified, refresh_secs, discovery_at, failures, content_hash, \
         profile_at, comment_system, comments_next_at"
    };
}

pub async fn kv_get(pool: &SqlitePool, key: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM kv WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
}

pub async fn kv_set(pool: &SqlitePool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO kv (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

/// Stores one directory's full member list. Members that disappeared from
/// the list are marked inactive, and each blog's `active` flag is recomputed
/// from all directories it belongs to.
pub async fn save_directory(
    pool: &SqlitePool,
    directory: &str,
    entries: &[DirectoryEntry],
    refresh_secs: i64,
    now: i64,
) -> sqlx::Result<usize> {
    let mut tx = pool.begin().await?;
    let mut saved = 0;
    for e in entries {
        let Some(key) = site_key(e.homepage.as_str()) else {
            continue;
        };
        let feed = e.feed_url.as_ref().map(|u| u.to_string());
        let blog_id: i64 = sqlx::query_scalar(
            "INSERT INTO blogs (site_key, name, homepage_url, feed_url, feed_url_origin, refresh_secs, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, CASE WHEN ?4 IS NULL THEN NULL ELSE 'directory' END, ?5, ?6, ?6)
             ON CONFLICT(site_key) DO UPDATE SET
                name = excluded.name,
                homepage_url = excluded.homepage_url,
                feed_url = CASE WHEN excluded.feed_url IS NOT NULL AND blogs.feed_url_origin IS NOT 'manual'
                                THEN excluded.feed_url ELSE blogs.feed_url END,
                feed_url_origin = CASE WHEN excluded.feed_url IS NOT NULL AND blogs.feed_url_origin IS NOT 'manual'
                                       THEN 'directory' ELSE blogs.feed_url_origin END,
                updated_at = excluded.updated_at
             RETURNING id",
        )
        .bind(&key)
        .bind(&e.name)
        .bind(e.homepage.as_str())
        .bind(&feed)
        .bind(refresh_secs)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO directory_entries (directory, external_id, blog_id, status, tags, active, last_seen_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(directory, external_id) DO UPDATE SET
                blog_id = excluded.blog_id, status = excluded.status, tags = excluded.tags,
                active = excluded.active, last_seen_at = excluded.last_seen_at",
        )
        .bind(directory)
        .bind(&e.external_id)
        .bind(blog_id)
        .bind(&e.status)
        .bind(serde_json::to_string(&e.tags).unwrap_or_else(|_| "[]".into()))
        .bind(e.alive)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM blog_tags WHERE blog_id = ? AND directory = ?")
            .bind(blog_id)
            .bind(directory)
            .execute(&mut *tx)
            .await?;
        for tag in &e.tags {
            sqlx::query("INSERT OR IGNORE INTO blog_tags (blog_id, directory, tag) VALUES (?, ?, ?)")
                .bind(blog_id)
                .bind(directory)
                .bind(tag)
                .execute(&mut *tx)
                .await?;
        }
        saved += 1;
    }

    sqlx::query("UPDATE directory_entries SET active = 0 WHERE directory = ? AND last_seen_at < ?")
        .bind(directory)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE blogs SET active = EXISTS (
            SELECT 1 FROM directory_entries d WHERE d.blog_id = blogs.id AND d.active = 1)",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(saved)
}

/// Applies `[policy]` from the config: opt-outs, full-text grants and feed overrides.
pub async fn apply_policy(pool: &SqlitePool, cfg: &Config) -> sqlx::Result<()> {
    let keys = |sites: Vec<&String>| -> Vec<String> {
        sites
            .into_iter()
            .filter_map(|s| {
                let k = site_key(s);
                if k.is_none() {
                    tracing::warn!(site = %s, "ignoring unparsable site in [policy]");
                }
                k
            })
            .collect()
    };
    let excluded = keys(cfg.policy.exclude.iter().collect());
    let full_text = keys(cfg.policy.full_text.iter().map(|g| &g.site).collect());
    let embed = keys(cfg.policy.embed_comments.iter().map(|g| &g.site).collect());
    for g in &cfg.policy.full_text {
        tracing::info!(site = %g.site, note = %g.note, "full text allowed");
    }

    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE blogs SET excluded = 0, content_policy = 'summary', embed_comments = 0")
        .execute(&mut *tx)
        .await?;
    for k in &excluded {
        sqlx::query("UPDATE blogs SET excluded = 1 WHERE site_key = ?")
            .bind(k)
            .execute(&mut *tx)
            .await?;
    }
    for k in &full_text {
        sqlx::query("UPDATE blogs SET content_policy = 'full_text' WHERE site_key = ?")
            .bind(k)
            .execute(&mut *tx)
            .await?;
    }
    for k in &embed {
        sqlx::query("UPDATE blogs SET embed_comments = 1 WHERE site_key = ?")
            .bind(k)
            .execute(&mut *tx)
            .await?;
    }
    // Withdrawn consent means the stored copies go too.
    sqlx::query("DELETE FROM posts WHERE blog_id IN (SELECT id FROM blogs WHERE excluded = 1)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE posts SET content_html = NULL, content_source = NULL
         WHERE content_source IS NOT NULL
           AND blog_id IN (SELECT id FROM blogs WHERE content_policy = 'summary')",
    )
    .execute(&mut *tx)
    .await?;

    let override_keys: Vec<(String, &str)> = cfg
        .policy
        .feed_overrides
        .iter()
        .filter_map(|o| Some((site_key(&o.site)?, o.feed.as_str())))
        .collect();
    let manual: Vec<(i64, String)> = sqlx::query_as("SELECT id, site_key FROM blogs WHERE feed_url_origin = 'manual'")
        .fetch_all(&mut *tx)
        .await?;
    for (id, key) in manual {
        if !override_keys.iter().any(|(k, _)| *k == key) {
            reset_feed(&mut tx, id).await?;
        }
    }
    for (key, feed) in &override_keys {
        sqlx::query(
            "UPDATE blogs SET feed_url = ?1, feed_url_origin = 'manual', etag = NULL, last_modified = NULL,
                    next_check_at = 0
             WHERE site_key = ?2 AND feed_url IS NOT ?1",
        )
        .bind(feed)
        .bind(key)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

async fn reset_feed(tx: &mut sqlx::SqliteConnection, id: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE blogs SET feed_url = NULL, feed_url_origin = NULL, etag = NULL, last_modified = NULL,
                discovery_at = NULL, next_check_at = 0
         WHERE id = ?",
    )
    .bind(id)
    .execute(tx)
    .await?;
    Ok(())
}

/// Blogs whose profile or comment counts are due. Only blogs we reached
/// successfully last time; failing sites are left to the feed backoff.
pub async fn due_for_enrich(
    pool: &SqlitePool,
    now: i64,
    profile_every: i64,
    count_every: i64,
    limit: i64,
) -> sqlx::Result<Vec<BlogRow>> {
    sqlx::query_as(concat!(
        "SELECT ",
        blog_columns!(),
        " FROM blogs
         WHERE active = 1 AND excluded = 0 AND failures = 0 AND last_checked_at IS NOT NULL
           AND ((?2 > 0 AND (profile_at IS NULL OR profile_at <= ?1 - ?2))
             OR (?3 > 0 AND comment_system IS NOT NULL AND comments_next_at <= ?1))
         ORDER BY profile_at IS NOT NULL, profile_at, comments_next_at
         LIMIT ?4"
    ))
    .bind(now)
    .bind(profile_every)
    .bind(count_every)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn due_blogs(pool: &SqlitePool, now: i64, limit: i64) -> sqlx::Result<Vec<BlogRow>> {
    sqlx::query_as(concat!(
        "SELECT ",
        blog_columns!(),
        " FROM blogs
         WHERE active = 1 AND excluded = 0 AND next_check_at <= ?
         ORDER BY next_check_at LIMIT ?"
    ))
    .bind(now)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn blog_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<BlogRow>> {
    sqlx::query_as(concat!(
        "SELECT ",
        blog_columns!(),
        " FROM blogs WHERE id = ? AND excluded = 0"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn set_discovered_feed(pool: &SqlitePool, id: i64, feed_url: &str, now: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE blogs SET feed_url = ?, feed_url_origin = 'discovered', discovery_at = ?,
                etag = NULL, last_modified = NULL
         WHERE id = ?",
    )
    .bind(feed_url)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_no_feed(pool: &SqlitePool, id: i64, now: i64, next_check_at: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE blogs SET discovery_at = ?1, last_checked_at = ?1, next_check_at = ?2,
                last_error = 'no feed found', updated_at = ?1
         WHERE id = ?3",
    )
    .bind(now)
    .bind(next_check_at)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub struct Success<'a> {
    pub validators: Option<&'a Validators>,
    pub refresh_secs: i64,
    pub next_check_at: i64,
    pub had_new_posts: bool,
    pub icon_url: Option<&'a str>,
    /// `None` keeps the stored hash.
    pub content_hash: Option<&'a str>,
}

pub async fn record_success(pool: &SqlitePool, id: i64, now: i64, s: Success<'_>) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE blogs SET
            etag = CASE WHEN ?1 THEN ?2 ELSE etag END,
            last_modified = CASE WHEN ?1 THEN ?3 ELSE last_modified END,
            refresh_secs = ?4, next_check_at = ?5,
            last_checked_at = ?6, last_success_at = ?6,
            last_new_post_at = CASE WHEN ?7 THEN ?6 ELSE last_new_post_at END,
            icon_url = COALESCE(?8, icon_url),
            content_hash = COALESCE(?9, content_hash),
            failures = 0, last_error = NULL, updated_at = ?6
         WHERE id = ?10",
    )
    .bind(s.validators.is_some())
    .bind(s.validators.and_then(|v| v.etag.as_deref()))
    .bind(s.validators.and_then(|v| v.last_modified.as_deref()))
    .bind(s.refresh_secs)
    .bind(s.next_check_at)
    .bind(now)
    .bind(s.had_new_posts)
    .bind(s.icon_url)
    .bind(s.content_hash)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_failure(
    pool: &SqlitePool,
    id: i64,
    now: i64,
    error: &str,
    next_check_at: i64,
    forget_feed: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE blogs SET failures = failures + 1, last_error = ?1, last_checked_at = ?2,
                next_check_at = ?3, updated_at = ?2,
                feed_url = CASE WHEN ?4 THEN NULL ELSE feed_url END,
                feed_url_origin = CASE WHEN ?4 THEN NULL ELSE feed_url_origin END,
                etag = CASE WHEN ?4 THEN NULL ELSE etag END,
                last_modified = CASE WHEN ?4 THEN NULL ELSE last_modified END
         WHERE id = ?5",
    )
    .bind(error)
    .bind(now)
    .bind(next_check_at)
    .bind(forget_feed)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Inserts or refreshes a post. Returns `true` when the post is new.
pub async fn upsert_post(
    pool: &SqlitePool,
    blog_id: i64,
    item: &ParsedItem,
    now: i64,
    undated_sort_at: i64,
    content: Option<(&str, &str)>,
) -> sqlx::Result<bool> {
    let published = item.published.map(|d| d.timestamp());
    let sort_at = published.unwrap_or(undated_sort_at).min(now);
    let first_seen: i64 = sqlx::query_scalar(
        "INSERT INTO posts (blog_id, guid, url, title, excerpt, cover_url, author, tags,
                            published_at, first_seen_at, sort_at, content_html, content_source,
                            comments_url, comment_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(blog_id, guid) DO UPDATE SET
            url = excluded.url, title = excluded.title, excerpt = excluded.excerpt,
            cover_url = excluded.cover_url, author = excluded.author, tags = excluded.tags,
            published_at = excluded.published_at,
            sort_at = MIN(COALESCE(excluded.published_at, posts.sort_at), posts.first_seen_at),
            content_html = COALESCE(excluded.content_html, posts.content_html),
            content_source = COALESCE(excluded.content_source, posts.content_source),
            comments_url = COALESCE(excluded.comments_url, posts.comments_url),
            comment_count = COALESCE(excluded.comment_count, posts.comment_count)
         RETURNING first_seen_at",
    )
    .bind(blog_id)
    .bind(&item.guid)
    .bind(item.url.as_str())
    .bind(&item.title)
    .bind(&item.excerpt)
    .bind(item.cover.as_ref().map(|u| u.as_str()))
    .bind(&item.author)
    .bind(serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".into()))
    .bind(published)
    .bind(now)
    .bind(sort_at)
    .bind(content.map(|c| c.0))
    .bind(content.map(|c| c.1))
    .bind(item.comments_url.as_ref().map(|u| u.as_str()))
    .bind(item.comment_count)
    .fetch_one(pool)
    .await?;
    Ok(first_seen == now)
}

pub struct Profile<'a> {
    pub description: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub about_url: Option<&'a str>,
    pub about_text: Option<&'a str>,
    pub social_json: &'a str,
    pub generator: Option<&'a str>,
    /// `None` keeps a previously detected system (pages can change themes).
    pub comment_system_json: Option<&'a str>,
}

pub async fn save_profile(pool: &SqlitePool, blog_id: i64, now: i64, p: &Profile<'_>) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE blogs SET description = ?1, avatar_url = ?2, about_url = ?3, about_text = ?4,
                social_links = ?5, generator = ?6,
                comment_system = COALESCE(?7, comment_system), profile_at = ?8
         WHERE id = ?9",
    )
    .bind(p.description)
    .bind(p.avatar_url)
    .bind(p.about_url)
    .bind(p.about_text)
    .bind(p.social_json)
    .bind(p.generator)
    .bind(p.comment_system_json)
    .bind(now)
    .bind(blog_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Records a profile attempt that failed, so it is not retried before `profile_at` + interval.
pub async fn touch_profile(pool: &SqlitePool, blog_id: i64, now: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE blogs SET profile_at = ? WHERE id = ?")
        .bind(now)
        .bind(blog_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The newest posts of a blog, for a batched comment-count request.
pub async fn recent_post_urls(pool: &SqlitePool, blog_id: i64, limit: i64) -> sqlx::Result<Vec<(i64, String)>> {
    sqlx::query_as("SELECT id, url FROM posts WHERE blog_id = ? ORDER BY sort_at DESC LIMIT ?")
        .bind(blog_id)
        .bind(limit)
        .fetch_all(pool)
        .await
}

pub async fn save_comment_counts(
    pool: &SqlitePool,
    blog_id: i64,
    counts: &[(i64, i64)],
    next_at: i64,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    for (post_id, count) in counts {
        sqlx::query("UPDATE posts SET comment_count = ? WHERE id = ? AND blog_id = ?")
            .bind(count)
            .bind(post_id)
            .bind(blog_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE blogs SET comments_next_at = ? WHERE id = ?")
        .bind(next_at)
        .bind(blog_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Keeps only the newest `keep` posts of a blog, so storage stays bounded
/// no matter how long a blog has been followed.
pub async fn prune_posts(pool: &SqlitePool, blog_id: i64, keep: i64) -> sqlx::Result<u64> {
    let res = sqlx::query(
        "DELETE FROM posts WHERE blog_id = ?1 AND id NOT IN (
            SELECT id FROM posts WHERE blog_id = ?1 ORDER BY sort_at DESC, id DESC LIMIT ?2)",
    )
    .bind(blog_id)
    .bind(keep)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Posts of a full-text blog that still have no stored body.
pub async fn posts_missing_content(pool: &SqlitePool, blog_id: i64, limit: i64) -> sqlx::Result<Vec<(i64, String)>> {
    sqlx::query_as(
        "SELECT id, url FROM posts WHERE blog_id = ? AND content_source IS NULL
         ORDER BY sort_at DESC LIMIT ?",
    )
    .bind(blog_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// `content = None` records that extraction found nothing, so it is not retried.
pub async fn set_post_content(pool: &SqlitePool, post_id: i64, content: Option<&str>) -> sqlx::Result<()> {
    sqlx::query("UPDATE posts SET content_html = ?, content_source = ? WHERE id = ?")
        .bind(content)
        .bind(if content.is_some() { "page" } else { "none" })
        .bind(post_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn entry(id: &str, tags: &[&str]) -> DirectoryEntry {
        DirectoryEntry {
            external_id: id.into(),
            name: format!("Blog {id}"),
            homepage: Url::parse(&format!("https://{id}.example/")).unwrap(),
            feed_url: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            status: "RUN".into(),
            alive: true,
        }
    }

    #[tokio::test]
    async fn resync_replaces_tags_and_deactivates_missing_blogs() {
        let pool = connect_memory().await;
        save_directory(
            &pool,
            "d",
            &[entry("a", &["tech", "blog"]), entry("b", &["life"])],
            3600,
            100,
        )
        .await
        .unwrap();
        save_directory(&pool, "d", &[entry("a", &["life"])], 3600, 200)
            .await
            .unwrap();

        let tags: Vec<(String, String)> =
            sqlx::query_as("SELECT b.site_key, t.tag FROM blog_tags t JOIN blogs b ON b.id = t.blog_id ORDER BY 1, 2")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            tags,
            [("a.example".into(), "life".into()), ("b.example".into(), "life".into())]
        );
        let active: Vec<(String, bool)> = sqlx::query_as("SELECT site_key, active FROM blogs ORDER BY 1")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(active, [("a.example".into(), true), ("b.example".into(), false)]);
    }

    #[tokio::test]
    async fn enrichment_selects_due_reachable_blogs() {
        let pool = connect_memory().await;
        let entries: Vec<DirectoryEntry> = ["a", "b", "c", "d"].iter().map(|id| entry(id, &[])).collect();
        save_directory(&pool, "d", &entries, 3600, 100).await.unwrap();
        // a: never checked; b: reachable, profile due; c: reachable but failing;
        // d: profile fresh, comment counts due.
        sqlx::query(
            "UPDATE blogs SET last_checked_at = 1000, failures = CASE site_key WHEN 'c.example' THEN 2 ELSE 0 END
             WHERE site_key <> 'a.example'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"UPDATE blogs SET profile_at = 9000, comment_system = '{"kind":"halo"}', comments_next_at = 500
               WHERE site_key = 'd.example'"#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let names = |rows: Vec<BlogRow>| rows.into_iter().map(|b| b.name).collect::<Vec<_>>();
        let due = due_for_enrich(&pool, 10_000, 30 * 86400, 86400, 10).await.unwrap();
        assert_eq!(names(due), ["Blog b", "Blog d"]);
        let profiles_only = due_for_enrich(&pool, 10_000, 30 * 86400, 0, 10).await.unwrap();
        assert_eq!(names(profiles_only), ["Blog b"]);
        let nothing = due_for_enrich(&pool, 10_000, 0, 0, 10).await.unwrap();
        assert!(nothing.is_empty());
    }

    #[tokio::test]
    async fn prune_keeps_newest_posts() {
        let pool = connect_memory().await;
        save_directory(&pool, "d", &[entry("a", &[])], 3600, 100).await.unwrap();
        for n in 0..10 {
            let item = crate::feed::ParsedItem {
                guid: n.to_string(),
                url: Url::parse(&format!("https://a.example/{n}")).unwrap(),
                title: format!("p{n}"),
                excerpt: String::new(),
                cover: None,
                author: None,
                tags: vec![],
                published: chrono::DateTime::from_timestamp(1_000_000 + n * 60, 0),
                body_html: None,
                comments_url: None,
                comment_count: None,
            };
            upsert_post(&pool, 1, &item, 2_000_000, 0, None).await.unwrap();
        }
        assert_eq!(prune_posts(&pool, 1, 4).await.unwrap(), 6);
        let left: Vec<String> = sqlx::query_scalar("SELECT guid FROM posts ORDER BY sort_at DESC")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, ["9", "8", "7", "6"]);
    }
}
