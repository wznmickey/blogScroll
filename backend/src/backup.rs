//! Backup of what was crawled, as text committed to a git repository (for
//! example a private one on GitHub), and restore from it.
//!
//! ```text
//! README.md          what this is and how to restore
//! meta.json          format, version, counts
//! blogs/{id}.json    { blog, directories, tags, posts }
//! ```
//!
//! One pretty-printed file per blog with posts in id order, so a commit only
//! touches the blogs that changed and its diff reads line by line. Crawl
//! bookkeeping that changes on every check (ETags, schedules, errors) and
//! everything derived (categories, term vectors, comment counts) is left
//! out; a restore rebuilds it. git itself (credentials, remote) is the
//! user's: we run the `git` command in their working copy.
//!
//! History is only ever added to: what was pushed from elsewhere is taken in
//! by fast-forward before each backup, a parted history stops the backup
//! instead of being overwritten, and the push is never a forced one.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tokio::process::Command;

use crate::config::BackupConfig;
use crate::util::now;

const FORMAT: &str = "blogscroll-data";
const VERSION: u32 = 1;
const GIT_TIMEOUT: Duration = Duration::from_secs(300);
const LAST_BACKUP_KEY: &str = "last_backup";
/// After a failed scheduled backup, try again this much later.
const RETRY_SECS: i64 = 3600;
const COMMITTER: [&str; 4] = [
    "-c",
    "user.name=blogScroll",
    "-c",
    "user.email=backup@blogscroll.invalid",
];

/// Columns holding JSON text are written as JSON, not as a string of JSON.
mod json_text {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(text: &str, s: S) -> Result<S::Ok, S::Error> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(serde::ser::Error::custom)?;
        v.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        Ok(serde_json::Value::deserialize(d)?.to_string())
    }

    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(text: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
            match text {
                Some(t) => super::serialize(t, s),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
            let v = serde_json::Value::deserialize(d)?;
            Ok((!v.is_null()).then(|| v.to_string()))
        }
    }
}

#[derive(Debug, Serialize, Deserialize, FromRow, PartialEq)]
struct BlogRecord {
    id: i64,
    site_key: String,
    name: String,
    homepage_url: String,
    icon_url: Option<String>,
    feed_url: Option<String>,
    feed_url_origin: Option<String>,
    active: bool,
    excluded: bool,
    content_policy: String,
    created_at: i64,
    description: Option<String>,
    avatar_url: Option<String>,
    about_url: Option<String>,
    about_text: Option<String>,
    #[serde(with = "json_text")]
    social_links: String,
    generator: Option<String>,
    #[serde(with = "json_text::option")]
    comment_system: Option<String>,
    embed_comments: bool,
    kind: Option<String>,
    detected_kind: Option<String>,
    flagged: Option<String>,
    from_feed_meta: bool,
}

#[derive(Debug, Serialize, Deserialize, FromRow, PartialEq)]
struct DirectoryRecord {
    #[serde(skip)]
    blog_id: i64,
    directory: String,
    external_id: String,
    status: Option<String>,
    #[serde(with = "json_text")]
    tags: String,
    active: bool,
}

#[derive(Debug, Serialize, Deserialize, FromRow, PartialEq)]
struct TagRecord {
    #[serde(skip)]
    blog_id: i64,
    directory: String,
    tag: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow, PartialEq)]
struct PostRecord {
    #[serde(skip)]
    blog_id: i64,
    id: i64,
    guid: String,
    url: String,
    title: String,
    excerpt: String,
    cover_url: Option<String>,
    author: Option<String>,
    #[serde(with = "json_text")]
    tags: String,
    published_at: Option<i64>,
    first_seen_at: i64,
    sort_at: i64,
    undated: bool,
    /// The analysis keeps these (they were extracted from the feed's body,
    /// which is not stored); everything else derived is recomputed.
    #[serde(with = "json_text")]
    keywords: String,
    content_html: Option<String>,
    content_source: Option<String>,
    comments_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct BlogFile {
    blog: BlogRecord,
    directories: Vec<DirectoryRecord>,
    tags: Vec<TagRecord>,
    posts: Vec<PostRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    format: String,
    version: u32,
    blogs: usize,
    posts: usize,
}

const README: &str = "# blogScroll 数据备份

由 `feed-server backup` 自动生成：每个博客一个文件（`blogs/<id>.json`），包含博客信息、
所属目录和全部已抓取文章。抓取过程中的临时状态（ETag、刷新时间、错误）和可以重新计算的内容
（分类、标签向量、评论数）不在这里。

恢复到一个新的空数据库：

```sh
feed-server -c config.toml restore --from <这个目录>
```
";

#[derive(Debug, Default)]
pub struct SnapshotStats {
    pub blogs: usize,
    pub posts: usize,
    /// Files written or removed because their content changed.
    pub changed: usize,
}

/// Writes the snapshot into `dir`, touching only files whose content changed.
pub async fn write_snapshot(pool: &SqlitePool, dir: &Path) -> anyhow::Result<SnapshotStats> {
    let blogs: Vec<BlogRecord> = sqlx::query_as(
        "SELECT id, site_key, name, homepage_url, icon_url, feed_url, feed_url_origin, active, excluded,
                content_policy, created_at, description, avatar_url, about_url, about_text, social_links,
                generator, comment_system, embed_comments, kind, detected_kind, flagged, from_feed_meta
         FROM blogs ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let mut files: BTreeMap<i64, BlogFile> = blogs
        .into_iter()
        .map(|blog| {
            let file = BlogFile {
                blog,
                directories: Vec::new(),
                tags: Vec::new(),
                posts: Vec::new(),
            };
            (file.blog.id, file)
        })
        .collect();
    let directories: Vec<DirectoryRecord> = sqlx::query_as(
        "SELECT blog_id, directory, external_id, status, tags, active FROM directory_entries
         ORDER BY directory, external_id",
    )
    .fetch_all(pool)
    .await?;
    for d in directories {
        if let Some(f) = files.get_mut(&d.blog_id) {
            f.directories.push(d);
        }
    }
    let tags: Vec<TagRecord> =
        sqlx::query_as("SELECT blog_id, directory, tag FROM blog_tags ORDER BY blog_id, directory, tag")
            .fetch_all(pool)
            .await?;
    for t in tags {
        if let Some(f) = files.get_mut(&t.blog_id) {
            f.tags.push(t);
        }
    }
    let posts: Vec<PostRecord> = sqlx::query_as(
        "SELECT blog_id, id, guid, url, title, excerpt, cover_url, author, tags, published_at, first_seen_at,
                sort_at, undated, keywords, content_html, content_source, comments_url
         FROM posts ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let mut stats = SnapshotStats {
        blogs: files.len(),
        posts: posts.len(),
        changed: 0,
    };
    for p in posts {
        if let Some(f) = files.get_mut(&p.blog_id) {
            f.posts.push(p);
        }
    }

    let blog_dir = dir.join("blogs");
    std::fs::create_dir_all(&blog_dir)?;
    let mut keep = HashSet::new();
    for (id, file) in &files {
        let name = format!("{id}.json");
        stats.changed += usize::from(write_if_changed(&blog_dir.join(&name), &pretty(file)?)?);
        keep.insert(name);
    }
    for entry in std::fs::read_dir(&blog_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") && !keep.contains(&name) {
            std::fs::remove_file(entry.path())?;
            stats.changed += 1;
        }
    }
    let meta = Meta {
        format: FORMAT.into(),
        version: VERSION,
        blogs: stats.blogs,
        posts: stats.posts,
    };
    stats.changed += usize::from(write_if_changed(&dir.join("meta.json"), &pretty(&meta)?)?);
    // Written once; after that the README is the repository owner's to edit.
    let readme = dir.join("README.md");
    if !readme.exists() {
        std::fs::write(&readme, README)?;
        stats.changed += 1;
    }
    Ok(stats)
}

fn pretty<T: Serialize>(v: &T) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(v)? + "\n")
}

fn write_if_changed(path: &Path, content: &str) -> anyhow::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(false);
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Runs git in `dir` without ever waiting for input: no password prompt, and
/// with `batch_ssh` ssh fails on an unknown host key or a passphrase instead
/// of asking in the terminal the server runs in.
async fn git(dir: &Path, args: &[&str], batch_ssh: bool) -> anyhow::Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(COMMITTER)
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    if batch_ssh {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let run = cmd.output();
    let out = tokio::time::timeout(GIT_TIMEOUT, run)
        .await
        .with_context(|| format!("git {} timed out", args.join(" ")))?
        .context("running git (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Batch-mode ssh, unless the user set up their own ssh command (which may
/// pick the deploy key), in the environment or in git's config.
async fn use_batch_ssh(dir: &Path) -> bool {
    if std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some() {
        return false;
    }
    let probe = if dir.join(".git").exists() { dir } else { Path::new(".") };
    git(probe, &["config", "--get", "core.sshCommand"], false)
        .await
        .map_or(true, |s| s.trim().is_empty())
}

async fn ensure_repo(cfg: &BackupConfig, dir: &Path, batch_ssh: bool) -> anyhow::Result<()> {
    if dir.join(".git").exists() {
        return Ok(());
    }
    let Some(remote) = cfg.remote.as_deref() else {
        bail!(
            "{} is not a git repository: clone your backup repository there, or set backup.remote",
            dir.display()
        );
    };
    let parent = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let target = dir.to_string_lossy();
    // Only the latest snapshot is needed to add the next one.
    git(
        parent,
        &["clone", "--quiet", "--depth", "1", remote, &target],
        batch_ssh,
    )
    .await?;
    Ok(())
}

#[derive(Debug)]
pub struct BackupOutcome {
    pub snapshot: SnapshotStats,
    pub committed: bool,
    pub pushed: bool,
}

/// Writes the snapshot, commits it if anything changed, and pushes.
pub async fn backup(pool: &SqlitePool, cfg: &BackupConfig) -> anyhow::Result<BackupOutcome> {
    let dir = cfg.git_dir.as_deref().context("backup.git_dir is not set")?;
    let b = use_batch_ssh(dir).await;
    ensure_repo(cfg, dir, b).await?;
    let has_origin = git(dir, &["remote"], b).await?.lines().any(|r| r == "origin");
    let push = cfg.push && has_origin;
    if push {
        // Take in what was pushed from elsewhere (an edited README, another
        // machine), by fast-forward only. If the histories have parted, stop:
        // a backup adds commits, it never overwrites any.
        git(dir, &["fetch", "--quiet", "origin"], b).await?;
        if git(dir, &["rev-parse", "--verify", "--quiet", "@{upstream}"], b)
            .await
            .is_ok()
        {
            git(dir, &["merge", "--ff-only", "--quiet", "@{upstream}"], b)
                .await
                .context(
                    "the backup repository has commits that do not follow this copy's; nothing was committed \
                     or pushed. Combine them by hand (git pull --rebase), never with --force",
                )?;
        }
    }
    let snapshot = write_snapshot(pool, dir).await?;
    git(dir, &["add", "--all"], b).await?;
    let changed = git(dir, &["status", "--porcelain"], b).await?.lines().count();
    let committed = changed > 0;
    if committed {
        let message = format!(
            "备份 {}：{} 个博客，{} 篇文章，{} 个文件有变化",
            chrono::Local::now().format("%Y-%m-%d %H:%M"),
            snapshot.blogs,
            snapshot.posts,
            changed
        );
        git(dir, &["commit", "--quiet", "-m", &message], b).await?;
    }
    // Also pushes commits a failed push left behind. The refspec is explicit
    // and has no `+`, so this is never a forced push, whatever the config says.
    if push {
        git(dir, &["push", "--quiet", "--set-upstream", "origin", "HEAD"], b).await?;
    }
    let pushed = push;
    crate::db::kv_set(pool, LAST_BACKUP_KEY, &now().to_string()).await?;
    Ok(BackupOutcome {
        snapshot,
        committed,
        pushed,
    })
}

/// While serving: a backup every `every_hours`, counted from the last one
/// (also across restarts).
pub async fn run_periodically(pool: SqlitePool, cfg: BackupConfig) {
    let every = i64::from(cfg.every_hours) * 3600;
    let mut next = crate::db::kv_get(&pool, LAST_BACKUP_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .map_or(0, |last| last + every);
    loop {
        // Not straight away at startup: the first analysis runs then.
        let wait = (next - now()).max(120);
        tokio::time::sleep(Duration::from_secs(wait as u64)).await;
        match backup(&pool, &cfg).await {
            Ok(o) => {
                tracing::info!(
                    blogs = o.snapshot.blogs,
                    posts = o.snapshot.posts,
                    changed = o.snapshot.changed,
                    committed = o.committed,
                    pushed = o.pushed,
                    "backup finished"
                );
                next = now() + every;
            }
            Err(e) => {
                tracing::warn!(error = %e, "backup failed");
                next = now() + RETRY_SECS;
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct RestoreStats {
    pub blogs: usize,
    pub posts: usize,
}

/// Fills an empty database from a snapshot. Feeds are fetched again soon
/// after (without validators); profiles are kept, and blogs that had no feed
/// are not searched again before `discovery_retry_secs`.
pub async fn restore(
    pool: &SqlitePool,
    dir: &Path,
    refresh_secs: i64,
    discovery_retry_secs: i64,
) -> anyhow::Result<RestoreStats> {
    let meta: Meta = serde_json::from_str(
        &std::fs::read_to_string(dir.join("meta.json"))
            .with_context(|| format!("no meta.json in {}", dir.display()))?,
    )?;
    if meta.format != FORMAT || meta.version > VERSION {
        bail!("{} is not a blogScroll backup this version can read", dir.display());
    }
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blogs").fetch_one(pool).await?;
    if existing > 0 {
        bail!("the database already has {existing} blogs; restore into a new, empty one ([database] path)");
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir.join("blogs"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();

    let t = now();
    let mut stats = RestoreStats::default();
    for chunk in paths.chunks(100) {
        let mut tx = pool.begin().await?;
        for path in chunk {
            let file: BlogFile = serde_json::from_str(&std::fs::read_to_string(path)?)
                .with_context(|| format!("reading {}", path.display()))?;
            let b = &file.blog;
            let next_check = if b.feed_url.is_some() {
                0
            } else {
                t + discovery_retry_secs
            };
            sqlx::query(
                "INSERT INTO blogs (id, site_key, name, homepage_url, icon_url, feed_url, feed_url_origin, active,
                    excluded, content_policy, refresh_secs, next_check_at, discovery_at, created_at, updated_at,
                    description, avatar_url, about_url, about_text, social_links, generator, profile_at,
                    comment_system, embed_comments, kind, detected_kind, flagged, from_feed_meta)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(b.id)
            .bind(&b.site_key)
            .bind(&b.name)
            .bind(&b.homepage_url)
            .bind(&b.icon_url)
            .bind(&b.feed_url)
            .bind(&b.feed_url_origin)
            .bind(b.active)
            .bind(b.excluded)
            .bind(&b.content_policy)
            .bind(refresh_secs)
            .bind(next_check)
            .bind(b.feed_url.is_none().then_some(t))
            .bind(b.created_at)
            .bind(t)
            .bind(&b.description)
            .bind(&b.avatar_url)
            .bind(&b.about_url)
            .bind(&b.about_text)
            .bind(&b.social_links)
            .bind(&b.generator)
            .bind(t)
            .bind(&b.comment_system)
            .bind(b.embed_comments)
            .bind(&b.kind)
            .bind(&b.detected_kind)
            .bind(&b.flagged)
            .bind(b.from_feed_meta)
            .execute(&mut *tx)
            .await?;
            for d in &file.directories {
                sqlx::query(
                    "INSERT INTO directory_entries (directory, external_id, blog_id, status, tags, active, last_seen_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&d.directory)
                .bind(&d.external_id)
                .bind(b.id)
                .bind(&d.status)
                .bind(&d.tags)
                .bind(d.active)
                .bind(t)
                .execute(&mut *tx)
                .await?;
            }
            for tag in &file.tags {
                sqlx::query("INSERT INTO blog_tags (blog_id, directory, tag) VALUES (?, ?, ?)")
                    .bind(b.id)
                    .bind(&tag.directory)
                    .bind(&tag.tag)
                    .execute(&mut *tx)
                    .await?;
            }
            for p in &file.posts {
                sqlx::query(
                    "INSERT INTO posts (id, blog_id, guid, url, title, excerpt, cover_url, author, tags, published_at,
                        first_seen_at, sort_at, undated, keywords, content_html, content_source, comments_url)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(p.id)
                .bind(b.id)
                .bind(&p.guid)
                .bind(&p.url)
                .bind(&p.title)
                .bind(&p.excerpt)
                .bind(&p.cover_url)
                .bind(&p.author)
                .bind(&p.tags)
                .bind(p.published_at)
                .bind(p.first_seen_at)
                .bind(p.sort_at)
                .bind(p.undated)
                .bind(&p.keywords)
                .bind(&p.content_html)
                .bind(&p.content_source)
                .bind(&p.comments_url)
                .execute(&mut *tx)
                .await?;
            }
            stats.blogs += 1;
            stats.posts += file.posts.len();
        }
        tx.commit().await?;
    }
    crate::analyze::retag_all(pool).await?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_round_trips_and_only_rewrites_what_changed() {
        let st = crate::api::tests::seeded().await;
        let dir = std::env::temp_dir().join(format!("blogscroll-backup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let first = write_snapshot(&st.pool, &dir).await.unwrap();
        assert_eq!((first.blogs, first.posts), (3, 15));
        assert_eq!(first.changed, 3 + 2, "three blogs, meta.json and README.md");
        let again = write_snapshot(&st.pool, &dir).await.unwrap();
        assert_eq!(again.changed, 0, "nothing changed, nothing rewritten");

        let text = std::fs::read_to_string(dir.join("blogs/1.json")).unwrap();
        assert!(
            text.contains("\"tags\": [\n"),
            "JSON columns are written as JSON:\n{text}"
        );
        assert!(!text.contains("etag") && !text.contains("next_check_at"));

        sqlx::query("UPDATE posts SET title = 'changed' WHERE id = 1")
            .execute(&st.pool)
            .await
            .unwrap();
        assert_eq!(write_snapshot(&st.pool, &dir).await.unwrap().changed, 1);

        let fresh = crate::db::connect_memory().await;
        let restored = restore(&fresh, &dir, 3600, 86400).await.unwrap();
        assert_eq!((restored.blogs, restored.posts), (3, 15));
        let dir2 = dir.join("again");
        write_snapshot(&fresh, &dir2).await.unwrap();
        for id in 1..=3 {
            let a = std::fs::read_to_string(dir.join(format!("blogs/{id}.json"))).unwrap();
            let b = std::fs::read_to_string(dir2.join(format!("blogs/{id}.json"))).unwrap();
            assert_eq!(a, b, "blog {id} survives a round trip");
        }
        let category: String = sqlx::query_scalar("SELECT category FROM posts WHERE id = 1")
            .fetch_one(&fresh)
            .await
            .unwrap();
        assert_eq!(category, "travel", "derived data is recomputed");
        assert!(
            restore(&fresh, &dir, 3600, 86400).await.is_err(),
            "never restores over existing data"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
