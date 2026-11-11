//! Backup of what was crawled, as text committed to a git repository (for
//! example a private one on GitHub), and merging it back in.
//!
//! ```text
//! README.md            what this is and how to restore
//! meta.json            format, version, counts
//! sites/<site>.json    { blog, directories, tags, posts }, named after the blog's address
//! ```
//!
//! One pretty-printed file per blog, posts in the order they were first
//! seen, so a commit only touches the blogs that changed and its diff reads
//! line by line. Nothing in it is a database id: several crawlers (a server
//! at home, GitHub Actions) back up into the same repository, each with its
//! own database, and a blog is known by its address, a post by its guid.
//! Crawl bookkeeping (ETags, schedules, errors) and everything derived
//! (categories, term vectors, comment counts) is left out; a restore
//! rebuilds it. git itself (credentials, remote) is the user's: we run the
//! `git` command in their working copy.
//!
//! Only ever added to:
//! - each backup first merges the repository into the database (blogs and
//!   posts it lacks, a newer profile, a feed found elsewhere), then writes
//!   the database together with everything the repository already had. A
//!   post leaves the snapshot only when its blogger withdrew consent
//!   (`policy.exclude`), and a stored full text only when that permission
//!   was withdrawn;
//! - history: what was pushed from elsewhere is taken in first, and the push
//!   is never a forced one. When another crawler pushed in between, it waits
//!   a random 5–10 minutes (`conflict_wait_minutes`, so two crawlers do not
//!   keep colliding), then takes the newer snapshot in, commits on top of it
//!   and pushes again.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tokio::process::Command;

use crate::config::BackupConfig;
use crate::util::now;

const FORMAT: &str = "blogscroll-data";
/// 1: `blogs/<id>.json`, with one database's ids (still read).
/// 2: `sites/<site>.json`, no ids.
const VERSION: u32 = 2;
const SITES: &str = "sites";
const LEGACY_BLOGS: &str = "blogs";
const GIT_TIMEOUT: Duration = Duration::from_secs(300);
const LAST_BACKUP_KEY: &str = "last_backup";
/// After a failed scheduled backup, try again this much later.
const RETRY_SECS: i64 = 3600;
/// Another crawler pushing between our fetch and our push: start again on
/// top of theirs, this many times.
const PUSH_ATTEMPTS: usize = 3;
/// A snapshot with this much less than the last one is not committed without
/// `allow_shrink`. Merging only adds, so it would take a bug or many
/// withdrawn consents.
const MAX_SHRINK: f64 = 0.1;
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

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
struct BlogRecord {
    /// This database's; not written (each crawler has its own).
    #[serde(skip)]
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
    /// When the profile fields above were read: the newer one wins a merge.
    #[serde(default)]
    profile_at: Option<i64>,
    embed_comments: bool,
    kind: Option<String>,
    detected_kind: Option<String>,
    flagged: Option<String>,
    from_feed_meta: bool,
}

impl BlogRecord {
    fn has_profile(&self) -> bool {
        self.description.is_some()
            || self.avatar_url.is_some()
            || self.about_text.is_some()
            || self.social_links != "[]"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
struct TagRecord {
    #[serde(skip)]
    blog_id: i64,
    directory: String,
    tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
struct PostRecord {
    #[serde(skip)]
    blog_id: i64,
    guid: String,
    url: String,
    title: String,
    excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    excerpt_html: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BlogFile {
    blog: BlogRecord,
    directories: Vec<DirectoryRecord>,
    tags: Vec<TagRecord>,
    posts: Vec<PostRecord>,
}

impl BlogFile {
    /// What the blog's own settings in this database allow to be kept.
    fn apply_policy(&mut self) {
        // Withdrawn consent means the stored copies go too.
        if self.blog.excluded {
            self.posts.clear();
        }
        if self.blog.content_policy != "full_text" {
            for p in &mut self.posts {
                p.content_html = None;
                p.content_source = None;
            }
        }
    }

    /// Adds what only `other` has: posts, directory entries, tags, a feed
    /// found elsewhere, a newer profile. Where both have a post, this one's
    /// stands, except that the post was first seen at the earlier of the two
    /// times (so the copies agree instead of taking turns in the history).
    fn absorb(&mut self, other: BlogFile) {
        let b = &mut self.blog;
        b.created_at = b.created_at.min(other.blog.created_at);
        if b.feed_url.is_none() && other.blog.feed_url.is_some() {
            b.feed_url = other.blog.feed_url.clone();
            b.feed_url_origin = other.blog.feed_url_origin.clone();
        }
        // A failed refresh moves profile_at on without reading anything, so
        // an empty profile never replaces one with content.
        if other.blog.has_profile() && (!b.has_profile() || other.blog.profile_at > b.profile_at) {
            let o = other.blog;
            b.description = o.description;
            b.avatar_url = o.avatar_url;
            b.about_url = o.about_url;
            b.about_text = o.about_text;
            b.social_links = o.social_links;
            b.generator = o.generator;
            b.comment_system = o.comment_system;
            b.profile_at = o.profile_at;
        }
        for d in other.directories {
            if !self
                .directories
                .iter()
                .any(|x| x.directory == d.directory && x.external_id == d.external_id)
            {
                self.directories.push(d);
            }
        }
        for t in other.tags {
            if !self.tags.iter().any(|x| x.directory == t.directory && x.tag == t.tag) {
                self.tags.push(t);
            }
        }
        let mut at: HashMap<String, usize> = self
            .posts
            .iter()
            .enumerate()
            .map(|(i, p)| (p.guid.clone(), i))
            .collect();
        for o in other.posts {
            let Some(&i) = at.get(&o.guid) else {
                at.insert(o.guid.clone(), self.posts.len());
                self.posts.push(o);
                continue;
            };
            let p = &mut self.posts[i];
            p.first_seen_at = p.first_seen_at.min(o.first_seen_at);
            if o.sort_at < p.sort_at {
                p.sort_at = o.sort_at;
                p.undated = o.undated;
            }
            if p.cover_url.is_none() {
                p.cover_url = o.cover_url;
            }
            if p.excerpt_html.is_none() {
                p.excerpt_html = o.excerpt_html;
            }
            if p.comments_url.is_none() {
                p.comments_url = o.comments_url;
            }
            if p.content_html.is_none() {
                p.content_html = o.content_html;
                p.content_source = o.content_source;
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    format: String,
    version: u32,
    blogs: usize,
    posts: usize,
}

const README: &str = "# blogScroll 数据备份

由 `feed-server backup` 自动生成：每个博客一个文件（`sites/<博客地址>.json`），包含博客信息、
所属目录和全部已抓取文章。抓取过程中的临时状态（ETag、刷新时间、错误）和可以重新计算的内容
（分类、标签向量、评论数）不在这里。

可以有多个抓取端（家里的服务器、GitHub Actions）往这里备份：每次备份先把这里已有的内容合并进
自己的数据库，再把合并后的结果提交回来，只增加不删除，也从不强制推送。

合并到一个数据库（空的或已有数据的都可以）：

```sh
feed-server -c config.toml restore --from <这个目录>
```
";

/// The first README, replaced while nobody has edited it.
const README_V1: &str = "# blogScroll 数据备份

由 `feed-server backup` 自动生成：每个博客一个文件（`blogs/<id>.json`），包含博客信息、
所属目录和全部已抓取文章。抓取过程中的临时状态（ETag、刷新时间、错误）和可以重新计算的内容
（分类、标签向量、评论数）不在这里。

恢复到一个新的空数据库：

```sh
feed-server -c config.toml restore --from <这个目录>
```
";

/// A blog's file name: its address, with anything but letters, digits, `.`,
/// `-` and `_` escaped (`example.com/blog` → `example.com%2Fblog.json`).
fn file_name(site_key: &str) -> String {
    let mut name = String::with_capacity(site_key.len() + 5);
    for b in site_key.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
            name.push(b as char);
        } else {
            name.push_str(&format!("%{b:02X}"));
        }
    }
    // File systems allow 255 bytes; keep long addresses apart by a hash.
    if name.len() > 200 {
        let hash = site_key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        name.truncate(180);
        while !name.is_char_boundary(name.len()) || name.ends_with('%') {
            name.pop();
        }
        name.push_str(&format!("-{hash:016x}"));
    }
    name + ".json"
}

fn json_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    paths.sort();
    Ok(paths)
}

/// Every blog in a snapshot directory, by address; both layouts are read.
fn read_snapshot(dir: &Path) -> anyhow::Result<BTreeMap<String, BlogFile>> {
    if let Ok(text) = std::fs::read_to_string(dir.join("meta.json")) {
        let meta: Meta = serde_json::from_str(&text).context("reading meta.json")?;
        if meta.format != FORMAT || meta.version > VERSION {
            bail!("{} is not a blogScroll backup this version can read", dir.display());
        }
    }
    let mut out: BTreeMap<String, BlogFile> = BTreeMap::new();
    for sub in [SITES, LEGACY_BLOGS] {
        for path in json_files(&dir.join(sub))? {
            let file: BlogFile = serde_json::from_str(&std::fs::read_to_string(&path)?)
                .with_context(|| format!("reading {}", path.display()))?;
            match out.get_mut(&file.blog.site_key) {
                Some(have) => have.absorb(file),
                None => {
                    out.insert(file.blog.site_key.clone(), file);
                }
            }
        }
    }
    Ok(out)
}

/// The database as snapshot files, by address (with this database's ids).
async fn read_database(pool: &SqlitePool) -> anyhow::Result<BTreeMap<String, BlogFile>> {
    let blogs: Vec<BlogRecord> = sqlx::query_as(
        "SELECT id, site_key, name, homepage_url, icon_url, feed_url, feed_url_origin, active, excluded,
                content_policy, created_at, description, avatar_url, about_url, about_text, social_links,
                generator, comment_system, profile_at, embed_comments, kind, detected_kind, flagged,
                from_feed_meta
         FROM blogs ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let mut by_id: BTreeMap<i64, BlogFile> = blogs
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
        if let Some(f) = by_id.get_mut(&d.blog_id) {
            f.directories.push(d);
        }
    }
    let tags: Vec<TagRecord> =
        sqlx::query_as("SELECT blog_id, directory, tag FROM blog_tags ORDER BY blog_id, directory, tag")
            .fetch_all(pool)
            .await?;
    for t in tags {
        if let Some(f) = by_id.get_mut(&t.blog_id) {
            f.tags.push(t);
        }
    }
    let posts: Vec<PostRecord> = sqlx::query_as(
        "SELECT blog_id, guid, url, title, excerpt, excerpt_html, cover_url, author, tags, published_at, first_seen_at,
                sort_at, undated, keywords, content_html, content_source, comments_url
         FROM posts ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    for p in posts {
        if let Some(f) = by_id.get_mut(&p.blog_id) {
            f.posts.push(p);
        }
    }
    Ok(by_id.into_values().map(|f| (f.blog.site_key.clone(), f)).collect())
}

#[derive(Debug, Default)]
pub struct SnapshotStats {
    pub blogs: usize,
    pub posts: usize,
    /// Files written or removed because their content changed.
    pub changed: usize,
}

/// Writes the database and what the snapshot in `dir` already had, touching
/// only files whose content changed. Files in the old layout are folded in
/// and removed.
pub async fn write_snapshot(pool: &SqlitePool, dir: &Path, allow_shrink: bool) -> anyhow::Result<SnapshotStats> {
    let mut files = read_snapshot(dir)?;
    for (key, mut mine) in read_database(pool).await? {
        if let Some(theirs) = files.remove(&key) {
            mine.absorb(theirs);
        }
        files.insert(key, mine);
    }
    let mut stats = SnapshotStats {
        blogs: files.len(),
        ..Default::default()
    };
    for f in files.values_mut() {
        f.apply_policy();
        f.posts
            .sort_by(|a, b| (a.first_seen_at, &a.guid).cmp(&(b.first_seen_at, &b.guid)));
        f.directories
            .sort_by(|a, b| (&a.directory, &a.external_id).cmp(&(&b.directory, &b.external_id)));
        f.tags
            .sort_by(|a, b| (&a.directory, &a.tag).cmp(&(&b.directory, &b.tag)));
        stats.posts += f.posts.len();
    }
    check_shrink(dir, &stats, allow_shrink)?;

    let sites = dir.join(SITES);
    std::fs::create_dir_all(&sites)?;
    for (key, file) in &files {
        stats.changed += usize::from(write_if_changed(&sites.join(file_name(key)), &pretty(file)?)?);
    }
    // Everything in the old layout is in the new one now.
    for path in json_files(&dir.join(LEGACY_BLOGS))? {
        std::fs::remove_file(path)?;
        stats.changed += 1;
    }
    let _ = std::fs::remove_dir(dir.join(LEGACY_BLOGS));
    let meta = Meta {
        format: FORMAT.into(),
        version: VERSION,
        blogs: stats.blogs,
        posts: stats.posts,
    };
    stats.changed += usize::from(write_if_changed(&dir.join("meta.json"), &pretty(&meta)?)?);
    // Written once; after that the README is the repository owner's to edit.
    let readme = dir.join("README.md");
    if std::fs::read_to_string(&readme).map_or(true, |t| t == README_V1) {
        stats.changed += usize::from(write_if_changed(&readme, README)?);
    }
    Ok(stats)
}

/// Refuses to replace the last snapshot with one far smaller; history would
/// keep the old one, but the latest snapshot is what a restore uses.
fn check_shrink(dir: &Path, new: &SnapshotStats, allow: bool) -> anyhow::Result<()> {
    let Ok(text) = std::fs::read_to_string(dir.join("meta.json")) else {
        return Ok(());
    };
    let Ok(last) = serde_json::from_str::<Meta>(&text) else {
        return Ok(());
    };
    let shrinks = |now: usize, before: usize| (now as f64) < before as f64 * (1.0 - MAX_SHRINK);
    if !allow && (shrinks(new.blogs, last.blogs) || shrinks(new.posts, last.posts)) {
        bail!(
            "the last backup has {} blogs and {} posts, this one would have {} and {}; not committing a \
             snapshot that much smaller. Pass --allow-shrink if the drop is intended",
            last.blogs,
            last.posts,
            new.blogs,
            new.posts
        );
    }
    Ok(())
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

/// How blogs new to this database are scheduled.
#[derive(Debug, Clone, Copy)]
pub struct MergeOptions {
    pub refresh_secs: i64,
    pub discovery_retry_secs: i64,
}

#[derive(Debug, Default)]
pub struct RestoreStats {
    /// In the snapshot.
    pub blogs: usize,
    pub posts: usize,
    /// New to the database.
    pub blogs_added: usize,
    pub posts_added: usize,
    /// Blogs whose stored data changed otherwise (a newer profile, a feed
    /// found elsewhere, an earlier first-seen time).
    pub blogs_updated: usize,
}

/// Merges a snapshot into the database, empty or not: blogs and posts it
/// lacks, newer profiles, feeds found elsewhere. Nothing is removed, and the
/// blog's settings here decide (an excluded blog takes no posts, a blog
/// without full-text permission no stored bodies). New blogs with a feed are
/// fetched soon; ones without are not searched again before
/// `discovery_retry_secs`.
pub async fn restore(pool: &SqlitePool, dir: &Path, opts: &MergeOptions) -> anyhow::Result<RestoreStats> {
    if !dir.join("meta.json").exists() {
        bail!("no meta.json in {}: not a blogScroll backup", dir.display());
    }
    let snapshot = read_snapshot(dir)?;
    let mut database = read_database(pool).await?;
    let t = now();
    let mut stats = RestoreStats {
        blogs: snapshot.len(),
        posts: snapshot.values().map(|f| f.posts.len()).sum(),
        ..Default::default()
    };
    // All or nothing: a merge that fails halfway leaves the database as it was.
    let mut tx = pool.begin().await?;
    for (key, theirs) in snapshot {
        match database.remove(&key) {
            None => {
                let mut file = theirs;
                file.apply_policy();
                insert_blog(&mut tx, &file, opts, t).await?;
                stats.blogs_added += 1;
                stats.posts_added += file.posts.len();
            }
            Some(mine) => {
                let mut merged = mine.clone();
                merged.absorb(theirs);
                merged.apply_policy();
                let added = update_blog(&mut tx, &mine, &merged).await?;
                stats.posts_added += added.0;
                stats.blogs_updated += usize::from(added.1);
            }
        }
    }
    // Which blogs are listed (and which only for readers) follows the
    // directory entries just taken in.
    crate::db::refresh_listing(&mut tx).await?;
    tx.commit().await?;
    if stats.blogs_added + stats.posts_added > 0 {
        crate::analyze::retag_all(pool).await?;
    }
    Ok(stats)
}

async fn insert_blog(
    tx: &mut sqlx::SqliteConnection,
    file: &BlogFile,
    opts: &MergeOptions,
    t: i64,
) -> anyhow::Result<()> {
    let b = &file.blog;
    let next_check = if b.feed_url.is_some() {
        0
    } else {
        t + opts.discovery_retry_secs
    };
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO blogs (site_key, name, homepage_url, icon_url, feed_url, feed_url_origin, active,
            excluded, content_policy, refresh_secs, next_check_at, discovery_at, created_at, updated_at,
            description, avatar_url, about_url, about_text, social_links, generator, profile_at,
            comment_system, embed_comments, kind, detected_kind, flagged, from_feed_meta)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&b.site_key)
    .bind(&b.name)
    .bind(&b.homepage_url)
    .bind(&b.icon_url)
    .bind(&b.feed_url)
    .bind(&b.feed_url_origin)
    .bind(b.active)
    .bind(b.excluded)
    .bind(&b.content_policy)
    .bind(opts.refresh_secs)
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
    // The first layout had no profile_at: a profile it has is not read again at once.
    .bind(b.profile_at.or(b.has_profile().then_some(t)))
    .bind(&b.comment_system)
    .bind(b.embed_comments)
    .bind(&b.kind)
    .bind(&b.detected_kind)
    .bind(&b.flagged)
    .bind(b.from_feed_meta)
    .fetch_one(&mut *tx)
    .await?;
    insert_entries(tx, id, file, &[], &[]).await?;
    for p in &file.posts {
        insert_post(tx, id, p).await?;
    }
    Ok(())
}

/// Stores what merging added to a blog already here; returns the posts
/// added and whether anything else changed.
async fn update_blog(
    tx: &mut sqlx::SqliteConnection,
    mine: &BlogFile,
    merged: &BlogFile,
) -> anyhow::Result<(usize, bool)> {
    let id = mine.blog.id;
    let (a, b) = (&mine.blog, &merged.blog);
    let mut changed = false;
    if a != b {
        changed = true;
        // A feed found elsewhere is fetched soon.
        let found_feed = a.feed_url.is_none() && b.feed_url.is_some();
        sqlx::query(
            "UPDATE blogs SET created_at = ?, feed_url = ?, feed_url_origin = ?,
                description = ?, avatar_url = ?, about_url = ?, about_text = ?, social_links = ?,
                generator = ?, comment_system = ?,
                -- A refresh asked for here (NULL) still happens.
                profile_at = CASE WHEN profile_at IS NULL THEN NULL ELSE ? END,
                next_check_at = CASE WHEN ? THEN 0 ELSE next_check_at END,
                failures = CASE WHEN ? THEN 0 ELSE failures END
             WHERE id = ?",
        )
        .bind(b.created_at)
        .bind(&b.feed_url)
        .bind(&b.feed_url_origin)
        .bind(&b.description)
        .bind(&b.avatar_url)
        .bind(&b.about_url)
        .bind(&b.about_text)
        .bind(&b.social_links)
        .bind(&b.generator)
        .bind(&b.comment_system)
        .bind(b.profile_at)
        .bind(found_feed)
        .bind(found_feed)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    insert_entries(tx, id, merged, &mine.directories, &mine.tags).await?;
    // A feed that turned into spam had its posts removed here; the snapshot
    // keeps the old ones, but they are not shown again.
    if mine.blog.flagged.as_deref() == Some("spam") {
        return Ok((0, changed));
    }
    let have: HashMap<&str, &PostRecord> = mine.posts.iter().map(|p| (p.guid.as_str(), p)).collect();
    let mut added = 0;
    for p in &merged.posts {
        match have.get(p.guid.as_str()) {
            None => {
                insert_post(tx, id, p).await?;
                added += 1;
            }
            Some(old) if *old != p => {
                changed = true;
                sqlx::query(
                    "UPDATE posts SET first_seen_at = ?, sort_at = ?, undated = ?, cover_url = ?, excerpt_html = ?,
                        comments_url = ?, content_html = ?, content_source = ?
                     WHERE blog_id = ? AND guid = ?",
                )
                .bind(p.first_seen_at)
                .bind(p.sort_at)
                .bind(p.undated)
                .bind(&p.cover_url)
                .bind(&p.excerpt_html)
                .bind(&p.comments_url)
                .bind(&p.content_html)
                .bind(&p.content_source)
                .bind(id)
                .bind(&p.guid)
                .execute(&mut *tx)
                .await?;
            }
            Some(_) => {}
        }
    }
    Ok((added, changed))
}

/// Directory entries and tags of `file` not among `have_*` (an entry listed
/// here for another blog stays with that one).
async fn insert_entries(
    tx: &mut sqlx::SqliteConnection,
    blog_id: i64,
    file: &BlogFile,
    have_dirs: &[DirectoryRecord],
    have_tags: &[TagRecord],
) -> anyhow::Result<()> {
    let t = now();
    for d in &file.directories {
        if have_dirs
            .iter()
            .any(|x| x.directory == d.directory && x.external_id == d.external_id)
        {
            continue;
        }
        sqlx::query(
            "INSERT OR IGNORE INTO directory_entries
                (directory, external_id, blog_id, status, tags, active, last_seen_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&d.directory)
        .bind(&d.external_id)
        .bind(blog_id)
        .bind(&d.status)
        .bind(&d.tags)
        .bind(d.active)
        .bind(t)
        .execute(&mut *tx)
        .await?;
    }
    for tag in &file.tags {
        if have_tags
            .iter()
            .any(|x| x.directory == tag.directory && x.tag == tag.tag)
        {
            continue;
        }
        sqlx::query("INSERT OR IGNORE INTO blog_tags (blog_id, directory, tag) VALUES (?, ?, ?)")
            .bind(blog_id)
            .bind(&tag.directory)
            .bind(&tag.tag)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

async fn insert_post(tx: &mut sqlx::SqliteConnection, blog_id: i64, p: &PostRecord) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO posts (blog_id, guid, url, title, excerpt, excerpt_html, cover_url, author, tags, published_at,
            first_seen_at, sort_at, undated, keywords, content_html, content_source, comments_url)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(blog_id)
    .bind(&p.guid)
    .bind(&p.url)
    .bind(&p.title)
    .bind(&p.excerpt)
    .bind(&p.excerpt_html)
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
    Ok(())
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
    pub merged: RestoreStats,
    pub snapshot: SnapshotStats,
    pub committed: bool,
    pub pushed: bool,
}

/// Takes in what was pushed from elsewhere, merges it into the database,
/// writes the snapshot, commits it if anything changed, and pushes.
pub async fn backup(pool: &SqlitePool, cfg: &BackupConfig, opts: &MergeOptions) -> anyhow::Result<BackupOutcome> {
    let dir = cfg.git_dir.as_deref().context("backup.git_dir is not set")?;
    // git runs in more than one directory (the clone in the parent), so a
    // relative path like "data/backup" must not be read from each of them.
    let dir = &std::path::absolute(dir)?;
    let b = use_batch_ssh(dir).await;
    ensure_repo(cfg, dir, b).await?;
    let has_origin = git(dir, &["remote"], b).await?.lines().any(|r| r == "origin");
    let push = cfg.push && has_origin;
    let mut attempt = 0;
    loop {
        attempt += 1;
        if push {
            catch_up(dir, b).await?;
        }
        let merged = if dir.join("meta.json").exists() {
            restore(pool, dir, opts).await?
        } else {
            RestoreStats::default()
        };
        let snapshot = write_snapshot(pool, dir, cfg.allow_shrink).await?;
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
        // Also pushes commits a failed push left behind. The refspec is
        // explicit and has no `+`, so this is never a forced push, whatever
        // the config says.
        if push && let Err(e) = git(dir, &["push", "--quiet", "--set-upstream", "origin", "HEAD"], b).await {
            if attempt < PUSH_ATTEMPTS {
                let [lo, hi] = cfg.conflict_wait_minutes;
                let wait = crate::util::random_between(lo * 60, hi * 60);
                tracing::info!(error = %e, wait_secs = wait, "backup push refused; waiting, then taking the newer snapshot in");
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }
            return Err(e);
        }
        crate::db::kv_set(pool, LAST_BACKUP_KEY, &now().to_string()).await?;
        return Ok(BackupOutcome {
            merged,
            snapshot,
            committed,
            pushed: push,
        });
    }
}

/// Brings the working copy up to the remote. Our own commits that were never
/// pushed are set aside (on the branch `unpushed`) rather than combined:
/// everything in them is in the database, so the next snapshot, made on top
/// of the remote's, has it again.
async fn catch_up(dir: &Path, b: bool) -> anyhow::Result<()> {
    // A shallow clone of an empty repository (the first backup's) has no
    // fetch refspec, so fetching would never update origin/<branch>.
    if git(dir, &["config", "--get-all", "remote.origin.fetch"], b)
        .await
        .map_or(true, |s| s.trim().is_empty())
    {
        git(
            dir,
            &["config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"],
            b,
        )
        .await?;
    }
    git(dir, &["fetch", "--quiet", "origin"], b).await?;
    if git(dir, &["rev-parse", "--verify", "--quiet", "@{upstream}"], b)
        .await
        .is_err()
    {
        return Ok(());
    }
    if git(dir, &["merge", "--ff-only", "--quiet", "@{upstream}"], b)
        .await
        .is_err()
    {
        git(dir, &["branch", "--force", "unpushed", "HEAD"], b).await?;
        git(dir, &["reset", "--hard", "--quiet", "@{upstream}"], b).await?;
    }
    Ok(())
}

/// While serving: a backup every `every_hours`, counted from the last one
/// (also across restarts).
pub async fn run_periodically(pool: SqlitePool, cfg: BackupConfig, opts: MergeOptions) {
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
        match backup(&pool, &cfg, &opts).await {
            Ok(o) => {
                tracing::info!(
                    blogs = o.snapshot.blogs,
                    posts = o.snapshot.posts,
                    blogs_added = o.merged.blogs_added,
                    posts_added = o.merged.posts_added,
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

#[cfg(test)]
mod tests {
    use super::*;

    const OPTS: MergeOptions = MergeOptions {
        refresh_secs: 3600,
        discovery_retry_secs: 86400,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blogscroll-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(COMMITTER)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn file_names_are_addresses() {
        assert_eq!(file_name("wznmickey.com"), "wznmickey.com.json");
        assert_eq!(file_name("example.com/blog"), "example.com%2Fblog.json");
        assert_ne!(file_name("a.com/b"), file_name("a.com%2Fb"));
        let long = format!("a.com/{}", "x".repeat(400));
        assert!(file_name(&long).len() < 220);
        assert_ne!(file_name(&long), file_name(&format!("{long}y")));
    }

    #[tokio::test]
    async fn snapshot_round_trips_and_only_rewrites_what_changed() {
        let st = crate::api::tests::seeded().await;
        let dir = temp_dir("backup-test");

        let first = write_snapshot(&st.pool, &dir, false).await.unwrap();
        assert_eq!((first.blogs, first.posts), (3, 15));
        assert_eq!(first.changed, 3 + 2, "three blogs, meta.json and README.md");
        let again = write_snapshot(&st.pool, &dir, false).await.unwrap();
        assert_eq!(again.changed, 0, "nothing changed, nothing rewritten");

        let text = std::fs::read_to_string(dir.join("sites/b1.example.json")).unwrap();
        assert!(
            text.contains("\"tags\": [\n"),
            "JSON columns are written as JSON:\n{text}"
        );
        assert!(!text.contains("etag") && !text.contains("next_check_at"));
        assert!(!text.contains("\"id\""), "no database ids:\n{text}");

        sqlx::query("UPDATE posts SET title = 'changed' WHERE id = 1")
            .execute(&st.pool)
            .await
            .unwrap();
        assert_eq!(write_snapshot(&st.pool, &dir, false).await.unwrap().changed, 1);

        let fresh = crate::db::connect_memory().await;
        let restored = restore(&fresh, &dir, &OPTS).await.unwrap();
        assert_eq!((restored.blogs_added, restored.posts_added), (3, 15));
        let dir2 = dir.join("again");
        write_snapshot(&fresh, &dir2, false).await.unwrap();
        for site in ["b1.example", "b2.example", "b3.example"] {
            let a = std::fs::read_to_string(dir.join(format!("sites/{site}.json"))).unwrap();
            let b = std::fs::read_to_string(dir2.join(format!("sites/{site}.json"))).unwrap();
            assert_eq!(a, b, "{site} survives a round trip");
        }
        let category: String = sqlx::query_scalar("SELECT category FROM posts WHERE guid = '1-0'")
            .fetch_one(&fresh)
            .await
            .unwrap();
        assert_eq!(category, "travel", "derived data is recomputed");
        let again = restore(&fresh, &dir, &OPTS).await.unwrap();
        assert_eq!(
            (again.blogs_added, again.posts_added, again.blogs_updated),
            (0, 0, 0),
            "merging the same snapshot twice changes nothing"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two crawlers, each with its own database (and ids), back up into the
    /// same snapshot: each ends up with both's posts, and nothing is lost.
    #[tokio::test]
    async fn two_databases_merge_and_the_snapshot_only_grows() {
        let home = crate::api::tests::seeded().await.pool;
        let actions = crate::db::connect_memory().await;
        let dir = temp_dir("backup-merge");

        // Actions knows blog 2 (under another id) with a post home lacks, and
        // has found a feed for blog 3, which home has not.
        write_snapshot(&home, &dir, false).await.unwrap();
        restore(&actions, &dir, &OPTS).await.unwrap();
        sqlx::query("DELETE FROM posts WHERE guid = '2-0'")
            .execute(&home)
            .await
            .unwrap();
        sqlx::query("UPDATE blogs SET feed_url = NULL WHERE site_key = 'b3.example'")
            .execute(&home)
            .await
            .unwrap();
        sqlx::query("UPDATE blogs SET feed_url = 'https://b3.example/atom.xml', feed_url_origin = 'discovered' WHERE site_key = 'b3.example'")
            .execute(&actions)
            .await
            .unwrap();
        let b2: i64 = count(&actions, "SELECT id FROM blogs WHERE site_key = 'b2.example'").await;
        sqlx::query(
            "INSERT INTO posts (blog_id, guid, url, title, first_seen_at, sort_at)
             VALUES (?, 'only-actions', 'https://b2.example/new', 'Only on Actions', 5, 5)",
        )
        .bind(b2)
        .execute(&actions)
        .await
        .unwrap();
        sqlx::query("UPDATE posts SET first_seen_at = 1 WHERE guid = '1-1'")
            .execute(&actions)
            .await
            .unwrap();
        // Actions backs up, then home does.
        write_snapshot(&actions, &dir, false).await.unwrap();
        let merged = restore(&home, &dir, &OPTS).await.unwrap();
        assert_eq!(merged.posts_added, 2, "2-0 and only-actions: {merged:?}");
        assert!(
            merged.blogs_updated >= 2,
            "the feed for b3, 1-1's first sighting: {merged:?}"
        );
        assert_eq!(
            count(
                &home,
                "SELECT COUNT(*) FROM blogs WHERE feed_url = 'https://b3.example/atom.xml' AND next_check_at = 0"
            )
            .await,
            1,
            "a feed found elsewhere is fetched soon"
        );
        assert_eq!(
            count(&home, "SELECT first_seen_at FROM posts WHERE guid = '1-1'").await,
            1
        );
        let s = write_snapshot(&home, &dir, false).await.unwrap();
        assert_eq!(s.posts, 16);

        // Gone from a database (pruned, lost), still in the snapshot.
        sqlx::query("DELETE FROM posts WHERE guid LIKE '1-%'")
            .execute(&home)
            .await
            .unwrap();
        assert_eq!(write_snapshot(&home, &dir, false).await.unwrap().posts, 16);
        // But a blogger who withdrew consent leaves it.
        sqlx::query("UPDATE blogs SET excluded = 1 WHERE site_key = 'b2.example'")
            .execute(&home)
            .await
            .unwrap();
        let err = write_snapshot(&home, &dir, false).await.unwrap_err().to_string();
        assert!(err.contains("--allow-shrink"), "{err}");
        let s = write_snapshot(&home, &dir, true).await.unwrap();
        assert_eq!(s.posts, 16 - 6);
        let text = std::fs::read_to_string(dir.join("sites/b2.example.json")).unwrap();
        assert!(text.contains("\"posts\": []"), "{text}");
        // ...and does not come back into a database that excludes them.
        let again = restore(&home, &dir, &OPTS).await.unwrap();
        assert_eq!(again.posts_added, 5, "the 1-* posts return, not b2's");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn the_first_layout_is_read_and_replaced() {
        let st = crate::api::tests::seeded().await;
        let dir = temp_dir("backup-legacy");
        std::fs::create_dir_all(dir.join("blogs")).unwrap();
        // Written by the first version: ids, numbered files, no profile_at.
        let legacy = serde_json::json!({
            "blog": {
                "id": 7, "site_key": "old.example", "name": "Old", "homepage_url": "https://old.example/",
                "icon_url": null, "feed_url": "https://old.example/feed", "feed_url_origin": "discovered",
                "active": true, "excluded": false, "content_policy": "summary", "created_at": 10,
                "description": null, "avatar_url": null, "about_url": null, "about_text": null,
                "social_links": [], "generator": null, "comment_system": null, "embed_comments": false,
                "kind": null, "detected_kind": null, "flagged": null, "from_feed_meta": false
            },
            "directories": [],
            "tags": [],
            "posts": [{
                "id": 99, "guid": "g", "url": "https://old.example/p", "title": "Old post", "excerpt": "",
                "cover_url": null, "author": null, "tags": [], "published_at": 20, "first_seen_at": 20,
                "sort_at": 20, "undated": false, "keywords": [], "content_html": null,
                "content_source": null, "comments_url": null
            }]
        });
        std::fs::write(dir.join("blogs/7.json"), legacy.to_string()).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            r#"{"format":"blogscroll-data","version":1,"blogs":1,"posts":1}"#,
        )
        .unwrap();
        std::fs::write(dir.join("README.md"), README_V1).unwrap();

        let s = write_snapshot(&st.pool, &dir, false).await.unwrap();
        assert_eq!((s.blogs, s.posts), (4, 16));
        assert!(!dir.join("blogs").exists(), "folded into sites/");
        assert!(dir.join("sites/old.example.json").exists());
        assert_eq!(std::fs::read_to_string(dir.join("README.md")).unwrap(), README);
        let fresh = crate::db::connect_memory().await;
        assert_eq!(restore(&fresh, &dir, &OPTS).await.unwrap().posts_added, 16);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two crawlers with their own working copies push to one repository,
    /// the second without having seen the first's commit: both commits land,
    /// one after the other, and the remote's history is never rewritten.
    #[tokio::test]
    async fn concurrent_backups_both_land_without_force() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let base = PathBuf::from(format!("target/backup-race-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let remote = std::path::absolute(base.join("remote.git")).unwrap();
        run_git(
            Path::new("."),
            &["init", "--quiet", "--bare", "-b", "main", &remote.to_string_lossy()],
        );
        let cfg = |name: &str| BackupConfig {
            // Relative, as in config.toml: cloned where it says.
            git_dir: Some(base.join(name)),
            remote: Some(format!("file://{}", remote.display())),
            every_hours: 0,
            push: true,
            allow_shrink: false,
            conflict_wait_minutes: [0, 0],
        };
        let home = crate::api::tests::seeded().await.pool;
        let actions = crate::db::connect_memory().await;

        let first = backup(&home, &cfg("home"), &OPTS).await.unwrap();
        assert!(first.committed && first.pushed);
        assert!(base.join("home/.git").exists(), "cloned where configured");
        assert!(!base.join("data").exists());
        // Actions clones now, then home commits again before Actions pushes.
        let a = backup(&actions, &cfg("actions"), &OPTS).await.unwrap();
        assert_eq!(a.merged.posts_added, 15, "took in home's posts");
        sqlx::query(
            "INSERT INTO posts (blog_id, guid, url, title, first_seen_at, sort_at)
             VALUES (1, 'from-home', 'https://b1.example/h', 'From home', 6, 6)",
        )
        .execute(&home)
        .await
        .unwrap();
        let ours = backup(&home, &cfg("home"), &OPTS).await.unwrap();
        assert!(ours.committed && ours.pushed);
        let b1: i64 = count(&actions, "SELECT id FROM blogs WHERE site_key = 'b1.example'").await;
        sqlx::query(
            "INSERT INTO posts (blog_id, guid, url, title, first_seen_at, sort_at)
             VALUES (?, 'from-actions', 'https://b1.example/x', 'From Actions', 5, 5)",
        )
        .bind(b1)
        .execute(&actions)
        .await
        .unwrap();
        // Simulate a push race: Actions committed locally before home's push.
        write_snapshot(&actions, &base.join("actions"), false).await.unwrap();
        run_git(&base.join("actions"), &["add", "--all"]);
        run_git(&base.join("actions"), &["commit", "--quiet", "-m", "unpushed"]);
        let theirs = backup(&actions, &cfg("actions"), &OPTS).await.unwrap();
        assert!(theirs.pushed);

        let log = run_git(&remote, &["log", "--format=%s", "main"]);
        assert_eq!(log.lines().count(), 3, "three backups, none rewritten:\n{log}");
        let clone = base.join("check");
        run_git(
            &base,
            &["clone", "--quiet", &format!("file://{}", remote.display()), "check"],
        );
        let text = std::fs::read_to_string(clone.join("sites/b1.example.json")).unwrap();
        assert!(text.contains("From home") && text.contains("From Actions"), "{text}");
        assert!(
            run_git(&base.join("actions"), &["branch", "--list", "unpushed"]).contains("unpushed"),
            "the commit that could not be pushed is set aside, not lost"
        );

        // A database that lost most of its posts still leaves the backup whole.
        sqlx::query("DELETE FROM posts").execute(&home).await.unwrap();
        let after = backup(&home, &cfg("home"), &OPTS).await.unwrap();
        assert_eq!(after.snapshot.posts, 17);
        assert_eq!(after.merged.posts_added, 17, "and fills it again");
        std::fs::remove_dir_all(&base).unwrap();
    }
}
