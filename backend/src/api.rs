use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::comments::{CommentSystem, EmbedConfig};
use crate::profile::SocialLink;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub site_name: String,
    pub directories: Arc<Vec<DirectoryInfo>>,
}

impl AppState {
    fn directory_name(&self, id: &str) -> String {
        self.directories
            .iter()
            .find(|d| d.id == id)
            .map_or_else(|| id.to_string(), |d| d.name.to_string())
    }

    fn tag_label(&self, directory: &str, tag: &str) -> String {
        self.directories
            .iter()
            .find(|d| d.id == directory)
            .and_then(|d| d.tag_labels.iter().find(|(t, _)| *t == tag))
            .map_or_else(|| tag.to_string(), |(_, label)| label.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
    #[serde(skip)]
    pub tag_labels: &'static [(&'static str, &'static str)],
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/meta", get(meta))
        .route("/api/tags", get(tags))
        .route("/api/posts", get(list_posts))
        .route("/api/posts/{id}", get(get_post))
        .route("/api/blogs", get(list_blogs))
        .route("/api/blogs/{id}", get(get_blog))
        .with_state(state)
}

pub struct ApiError(StatusCode, String);

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = %e, "database error");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "not found".into())
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn truthy(s: &Option<String>) -> bool {
    matches!(non_empty(s), Some("1" | "true" | "yes"))
}

fn like_pattern(s: &str) -> String {
    let escaped: String = s
        .chars()
        .flat_map(|c| match c {
            '%' | '_' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    format!("%{escaped}%")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TagRef {
    directory: String,
    tag: String,
    #[serde(default)]
    label: String,
}

/// Parses `json_group_array(json_object('directory', .., 'tag', ..))` and fills in labels.
fn tag_refs(st: &AppState, json: &str) -> Vec<TagRef> {
    let mut tags: Vec<TagRef> = serde_json::from_str(json).unwrap_or_default();
    for t in &mut tags {
        t.label = st.tag_label(&t.directory, &t.tag);
    }
    tags
}

// ---------- meta ----------

#[derive(Serialize)]
struct Meta {
    site_name: String,
    directories: Vec<DirectoryInfo>,
    stats: Stats,
    /// When this answer was produced; tells static exports' readers how fresh they are.
    generated_at: i64,
}

#[derive(Serialize, FromRow)]
struct Stats {
    blogs: i64,
    blogs_with_feed: i64,
    posts: i64,
    last_directory_sync: Option<i64>,
}

async fn meta(State(st): State<AppState>) -> ApiResult<Meta> {
    let stats: Stats = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM blogs WHERE active = 1 AND excluded = 0) AS blogs,
            (SELECT COUNT(*) FROM blogs WHERE active = 1 AND excluded = 0 AND last_success_at IS NOT NULL) AS blogs_with_feed,
            (SELECT COUNT(*) FROM posts p JOIN blogs b ON b.id = p.blog_id WHERE b.active = 1 AND b.excluded = 0) AS posts,
            (SELECT CAST(value AS INTEGER) FROM kv WHERE key = 'last_directory_sync') AS last_directory_sync",
    )
    .fetch_one(&st.pool)
    .await?;
    Ok(Json(Meta {
        site_name: st.site_name.clone(),
        directories: st.directories.to_vec(),
        stats,
        generated_at: crate::util::now(),
    }))
}

// ---------- tags ----------

#[derive(Serialize)]
struct TagsResponse {
    /// Directory tags (开往: 技术 / 生活 / ...), used as channels.
    directories: Vec<DirectoryTags>,
    /// Popular categories from the posts themselves (RSS `<category>`), last 180 days.
    categories: Vec<CategoryCount>,
}

#[derive(Serialize)]
struct DirectoryTags {
    id: &'static str,
    name: &'static str,
    tags: Vec<TagCount>,
}

#[derive(Serialize)]
struct TagCount {
    tag: String,
    label: String,
    blogs: i64,
    posts: i64,
}

#[derive(Serialize, FromRow)]
struct CategoryCount {
    name: String,
    blogs: i64,
    posts: i64,
}

async fn tags(State(st): State<AppState>) -> ApiResult<TagsResponse> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT bt.directory, bt.tag, COUNT(DISTINCT b.id) AS blogs, COUNT(p.id) AS posts
         FROM blog_tags bt
         JOIN blogs b ON b.id = bt.blog_id
         LEFT JOIN posts p ON p.blog_id = b.id
         WHERE b.active = 1 AND b.excluded = 0
         GROUP BY bt.directory, bt.tag
         ORDER BY blogs DESC, bt.tag",
    )
    .fetch_all(&st.pool)
    .await?;
    let directories = st
        .directories
        .iter()
        .map(|d| DirectoryTags {
            id: d.id,
            name: d.name,
            tags: rows
                .iter()
                .filter(|(dir, ..)| dir == d.id)
                .map(|(dir, tag, blogs, posts)| TagCount {
                    label: st.tag_label(dir, tag),
                    tag: tag.clone(),
                    blogs: *blogs,
                    posts: *posts,
                })
                .collect(),
        })
        .collect();

    let since = crate::util::now() - 180 * 86400;
    let categories: Vec<CategoryCount> = sqlx::query_as(
        "SELECT MIN(j.value) AS name, COUNT(DISTINCT p.blog_id) AS blogs, COUNT(*) AS posts
         FROM posts p JOIN blogs b ON b.id = p.blog_id, json_each(p.tags) j
         WHERE b.active = 1 AND b.excluded = 0 AND p.sort_at >= ?
         GROUP BY lower(j.value)
         HAVING blogs >= 2
         ORDER BY blogs DESC, posts DESC, name
         LIMIT 40",
    )
    .bind(since)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(TagsResponse {
        directories,
        categories,
    }))
}

// ---------- posts ----------

#[derive(Debug, Deserialize)]
pub struct PostQuery {
    /// `discover` (default): recent posts, lightly shuffled, at most a few per blog.
    /// `latest`: strictly newest first.
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    /// Opaque: `sort_at.id` for latest, an offset for discover.
    #[serde(default)]
    cursor: Option<String>,
    /// Shuffle seed for discover, picked by the client per session.
    #[serde(default)]
    seed: Option<i64>,
    #[serde(default)]
    blog: Option<i64>,
    #[serde(default)]
    q: Option<String>,
    /// Directory tag of the blog, e.g. `tech`.
    #[serde(default)]
    tag: Option<String>,
    /// Only blogs listed in this directory, e.g. `travellings`.
    #[serde(default)]
    directory: Option<String>,
    /// The post's own category from its feed, e.g. `旅行`.
    #[serde(default)]
    category: Option<String>,
    /// `1`: only posts with a cover image.
    #[serde(default)]
    cover: Option<String>,
}

#[derive(Debug, FromRow)]
struct PostRow {
    id: i64,
    title: String,
    url: String,
    excerpt: String,
    cover_url: Option<String>,
    author: Option<String>,
    tags: String,
    published_at: Option<i64>,
    sort_at: i64,
    has_full_text: bool,
    comment_count: Option<i64>,
    comments_url: Option<String>,
    blog_id: i64,
    blog_name: String,
    blog_homepage: String,
    blog_icon: Option<String>,
    blog_tags: String,
}

#[derive(Debug, Serialize)]
pub struct BlogRef {
    id: i64,
    name: String,
    homepage: String,
    icon: Option<String>,
    /// Directory tags of the blog (e.g. `tech`), so clients can filter cards.
    tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PostCard {
    id: i64,
    title: String,
    url: String,
    excerpt: String,
    cover: Option<String>,
    author: Option<String>,
    tags: Vec<String>,
    published_at: Option<i64>,
    sort_at: i64,
    has_full_text: bool,
    /// From the feed (`slash:comments`) or the blog's comment service.
    comment_count: Option<i64>,
    comments_url: Option<String>,
    blog: BlogRef,
}

impl From<PostRow> for PostCard {
    fn from(r: PostRow) -> Self {
        PostCard {
            id: r.id,
            title: r.title,
            url: r.url,
            excerpt: r.excerpt,
            cover: r.cover_url,
            author: r.author,
            tags: serde_json::from_str(&r.tags).unwrap_or_default(),
            published_at: r.published_at,
            sort_at: r.sort_at,
            has_full_text: r.has_full_text,
            comment_count: r.comment_count,
            comments_url: r.comments_url,
            blog: BlogRef {
                id: r.blog_id,
                name: r.blog_name,
                homepage: r.blog_homepage,
                icon: r.blog_icon,
                tags: serde_json::from_str(&r.blog_tags).unwrap_or_default(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    items: Vec<T>,
    next_cursor: Option<String>,
}

macro_rules! post_columns {
    () => {
        "p.id, p.title, p.url, p.excerpt, p.cover_url, p.author, p.tags, p.published_at, p.sort_at,
     (p.content_html IS NOT NULL) AS has_full_text, p.comment_count, p.comments_url,
     b.id AS blog_id, b.name AS blog_name, b.homepage_url AS blog_homepage,
     COALESCE(b.avatar_url, b.icon_url) AS blog_icon,
     (SELECT json_group_array(DISTINCT bt.tag) FROM blog_tags bt WHERE bt.blog_id = b.id) AS blog_tags"
    };
}

/// Blog-level filters shared by both feed modes. Arguments are the SQL
/// parameter numbers for (tag, directory, cover-only).
macro_rules! blog_filters {
    ($tag:literal, $dir:literal, $cover:literal) => {
        concat!(
            " AND (",
            $tag,
            " IS NULL OR EXISTS (SELECT 1 FROM blog_tags bt WHERE bt.blog_id = b.id AND bt.tag = ",
            $tag,
            "))",
            " AND (",
            $dir,
            " IS NULL OR EXISTS (SELECT 1 FROM directory_entries de
                   WHERE de.blog_id = b.id AND de.active = 1 AND de.directory = ",
            $dir,
            "))",
            " AND (",
            $cover,
            " = 0 OR p.cover_url IS NOT NULL)"
        )
    };
}

struct Filters<'a> {
    tag: Option<&'a str>,
    directory: Option<&'a str>,
    category: Option<&'a str>,
    cover: bool,
}

async fn list_posts(State(st): State<AppState>, Query(q): Query<PostQuery>) -> ApiResult<Page<PostCard>> {
    let limit = q.limit.unwrap_or(30).clamp(1, 100);
    let search = non_empty(&q.q).map(like_pattern);
    let f = Filters {
        tag: non_empty(&q.tag),
        directory: non_empty(&q.directory),
        category: non_empty(&q.category),
        cover: truthy(&q.cover),
    };
    let discover = q.mode.as_deref() != Some("latest") && q.blog.is_none() && search.is_none() && f.category.is_none();
    if discover {
        discover_posts(&st.pool, limit, q.cursor.as_deref(), q.seed.unwrap_or(0), &f).await
    } else {
        latest_posts(&st.pool, limit, q.cursor.as_deref(), q.blog, search.as_deref(), &f).await
    }
}

async fn latest_posts(
    pool: &SqlitePool,
    limit: i64,
    cursor: Option<&str>,
    blog: Option<i64>,
    search: Option<&str>,
    f: &Filters<'_>,
) -> ApiResult<Page<PostCard>> {
    let (c_sort, c_id) = cursor
        .and_then(|c| c.split_once('.'))
        .and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?)))
        .map_or((None, None), |(a, b)| (Some(a), Some(b)));
    let rows: Vec<PostRow> = sqlx::query_as(concat!(
        "SELECT ",
        post_columns!(),
        "
         FROM posts p JOIN blogs b ON b.id = p.blog_id
         WHERE b.excluded = 0
           AND (?1 IS NOT NULL OR b.active = 1)
           AND (?1 IS NULL OR p.blog_id = ?1)
           AND (?2 IS NULL OR (p.sort_at, p.id) < (?2, ?3))
           AND (?4 IS NULL OR p.title LIKE ?4 ESCAPE '\\' OR p.excerpt LIKE ?4 ESCAPE '\\' OR b.name LIKE ?4 ESCAPE '\\')
           AND (?9 IS NULL OR EXISTS (SELECT 1 FROM json_each(p.tags) j WHERE lower(j.value) = lower(?9)))",
        blog_filters!("?6", "?7", "?8"),
        "
         ORDER BY p.sort_at DESC, p.id DESC
         LIMIT ?5"
    ))
    .bind(blog)
    .bind(c_sort)
    .bind(c_id)
    .bind(search)
    .bind(limit)
    .bind(f.tag)
    .bind(f.directory)
    .bind(f.cover)
    .bind(f.category)
    .fetch_all(pool)
    .await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last().map(|r| format!("{}.{}", r.sort_at, r.id)))
        .flatten();
    Ok(Json(Page {
        items: rows.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

/// Score = age in days, plus up to two weeks of seeded noise, minus five days
/// for each newer post of the same blog. Keeps the feed fresh, mixed, and
/// stops one prolific blog from filling a screen.
async fn discover_posts(
    pool: &SqlitePool,
    limit: i64,
    cursor: Option<&str>,
    seed: i64,
    f: &Filters<'_>,
) -> ApiResult<Page<PostCard>> {
    let offset: i64 = cursor.and_then(|c| c.parse().ok()).unwrap_or(0).max(0);
    let seed = seed.rem_euclid(1 << 31);
    let rows: Vec<PostRow> = sqlx::query_as(concat!(
        "WITH ranked AS (
            SELECT p.id AS post_id, ROW_NUMBER() OVER (PARTITION BY p.blog_id ORDER BY p.sort_at DESC) AS rn
            FROM posts p JOIN blogs b ON b.id = p.blog_id
            WHERE b.active = 1 AND b.excluded = 0",
        blog_filters!("?4", "?5", "?6"),
        "
         )
         SELECT ",
        post_columns!(),
        "
         FROM ranked r JOIN posts p ON p.id = r.post_id JOIN blogs b ON b.id = p.blog_id
         WHERE r.rn <= 8
         ORDER BY p.sort_at / 86400.0
                  + ((p.id * 2654435761 + ?1) % 1000003) / 1000003.0 * 14.0
                  - (r.rn - 1) * 5.0 DESC,
                  p.id DESC
         LIMIT ?2 OFFSET ?3"
    ))
    .bind(seed)
    .bind(limit)
    .bind(offset)
    .bind(f.tag)
    .bind(f.directory)
    .bind(f.cover)
    .fetch_all(pool)
    .await?;
    let next_cursor = (rows.len() as i64 == limit).then(|| (offset + limit).to_string());
    Ok(Json(Page {
        items: rows.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

#[derive(Debug, Serialize)]
pub struct PostDetail {
    #[serde(flatten)]
    card: PostCard,
    /// Only present for blogs whose author allowed full text.
    content_html: Option<String>,
    comments: Option<PostComments>,
}

#[derive(Debug, Serialize)]
pub struct PostComments {
    /// Display name of the blog's comment system, e.g. "Waline".
    system: String,
    /// Widget configuration; only for blogs whose author allowed embedding.
    embed: Option<EmbedConfig>,
}

async fn get_post(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<PostDetail> {
    #[derive(FromRow)]
    struct Row {
        #[sqlx(flatten)]
        post: PostRow,
        content_html: Option<String>,
        comment_system: Option<String>,
        embed_comments: bool,
    }
    let row: Option<Row> = sqlx::query_as(concat!(
        "SELECT ",
        post_columns!(),
        ", CASE WHEN b.content_policy = 'full_text' THEN p.content_html END AS content_html,
         b.comment_system, b.embed_comments
         FROM posts p JOIN blogs b ON b.id = p.blog_id
         WHERE p.id = ? AND b.excluded = 0"
    ))
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let row = row.ok_or_else(not_found)?;
    let system: Option<CommentSystem> = row.comment_system.as_deref().and_then(|s| serde_json::from_str(s).ok());
    let card: PostCard = row.post.into();
    let comments = system.map(|sys| PostComments {
        system: sys.label(),
        embed: if row.embed_comments {
            url::Url::parse(&card.url)
                .ok()
                .and_then(|u| crate::comments::embed_config(&sys, &u, &card.title))
        } else {
            None
        },
    });
    Ok(Json(PostDetail {
        card,
        content_html: row.content_html,
        comments,
    }))
}

// ---------- blogs ----------

#[derive(Debug, Deserialize)]
pub struct BlogQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    /// `recent` (default) | `posts` | `name`
    #[serde(default)]
    sort: Option<String>,
}

#[derive(Debug, FromRow)]
struct BlogSummaryRow {
    id: i64,
    name: String,
    homepage: String,
    icon: Option<String>,
    post_count: i64,
    last_post_at: Option<i64>,
    has_feed: bool,
    description: Option<String>,
    tags: String,
}

#[derive(Debug, Serialize)]
pub struct BlogSummary {
    id: i64,
    name: String,
    homepage: String,
    icon: Option<String>,
    post_count: i64,
    last_post_at: Option<i64>,
    has_feed: bool,
    /// From the blog's own homepage (meta description).
    description: Option<String>,
    tags: Vec<TagRef>,
}

impl BlogSummary {
    fn from_row(st: &AppState, r: BlogSummaryRow) -> Self {
        BlogSummary {
            tags: tag_refs(st, &r.tags),
            id: r.id,
            name: r.name,
            homepage: r.homepage,
            icon: r.icon,
            post_count: r.post_count,
            last_post_at: r.last_post_at,
            has_feed: r.has_feed,
            description: r.description,
        }
    }
}

macro_rules! blog_summary_sql {
    () => {
        "SELECT b.id, b.name, b.homepage_url AS homepage, COALESCE(b.avatar_url, b.icon_url) AS icon,
            b.description,
            (SELECT COUNT(*) FROM posts p WHERE p.blog_id = b.id) AS post_count,
            (SELECT MAX(p.sort_at) FROM posts p WHERE p.blog_id = b.id) AS last_post_at,
            (b.last_success_at IS NOT NULL) AS has_feed,
            (SELECT json_group_array(json_object('directory', bt.directory, 'tag', bt.tag))
             FROM blog_tags bt WHERE bt.blog_id = b.id) AS tags
         FROM blogs b"
    };
}

async fn list_blogs(State(st): State<AppState>, Query(q): Query<BlogQuery>) -> ApiResult<Page<BlogSummary>> {
    let limit = q.limit.unwrap_or(48).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let search = non_empty(&q.q).map(like_pattern);
    let rows: Vec<BlogSummaryRow> = sqlx::query_as(concat!(
        blog_summary_sql!(),
        "
         WHERE b.active = 1 AND b.excluded = 0
           AND (?1 IS NULL OR b.name LIKE ?1 ESCAPE '\\' OR b.homepage_url LIKE ?1 ESCAPE '\\')
           AND (?4 IS NULL OR EXISTS (SELECT 1 FROM blog_tags bt WHERE bt.blog_id = b.id AND bt.tag = ?4))
         ORDER BY
           CASE WHEN ?5 = 'name' THEN b.name END COLLATE NOCASE,
           CASE WHEN ?5 = 'posts' THEN post_count END DESC,
           last_post_at IS NULL, last_post_at DESC, b.id
         LIMIT ?2 OFFSET ?3"
    ))
    .bind(search)
    .bind(limit)
    .bind(offset)
    .bind(non_empty(&q.tag))
    .bind(non_empty(&q.sort).unwrap_or("recent"))
    .fetch_all(&st.pool)
    .await?;
    let next_cursor = (rows.len() as i64 == limit).then(|| (offset + limit).to_string());
    Ok(Json(Page {
        items: rows.into_iter().map(|r| BlogSummary::from_row(&st, r)).collect(),
        next_cursor,
    }))
}

#[derive(Debug, Serialize)]
pub struct BlogDetail {
    #[serde(flatten)]
    summary: BlogSummary,
    full_text: bool,
    directories: Vec<BlogDirectory>,
    about_url: Option<String>,
    /// Short excerpt of the about page.
    about_text: Option<String>,
    social: Vec<SocialLink>,
    /// Blog engine from `<meta name="generator">`, e.g. "Hexo 7.3.0".
    generator: Option<String>,
    /// Display name of the detected comment system.
    comment_system: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BlogDirectory {
    directory: String,
    name: String,
}

async fn get_blog(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<BlogDetail> {
    let row: Option<BlogSummaryRow> =
        sqlx::query_as(concat!(blog_summary_sql!(), " WHERE b.id = ? AND b.excluded = 0"))
            .bind(id)
            .fetch_optional(&st.pool)
            .await?;
    let summary = BlogSummary::from_row(&st, row.ok_or_else(not_found)?);
    #[derive(FromRow)]
    struct Extra {
        full_text: bool,
        about_url: Option<String>,
        about_text: Option<String>,
        social_links: String,
        generator: Option<String>,
        comment_system: Option<String>,
    }
    let extra: Extra = sqlx::query_as(
        "SELECT content_policy = 'full_text' AS full_text, about_url, about_text, social_links,
                generator, comment_system
         FROM blogs WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&st.pool)
    .await?;
    let dirs: Vec<String> =
        sqlx::query_scalar("SELECT directory FROM directory_entries WHERE blog_id = ? AND active = 1")
            .bind(id)
            .fetch_all(&st.pool)
            .await?;
    let directories = dirs
        .into_iter()
        .map(|directory| BlogDirectory {
            name: st.directory_name(&directory),
            directory,
        })
        .collect();
    Ok(Json(BlogDetail {
        summary,
        full_text: extra.full_text,
        directories,
        about_url: extra.about_url,
        about_text: extra.about_text,
        social: serde_json::from_str(&extra.social_links).unwrap_or_default(),
        generator: extra.generator,
        comment_system: extra
            .comment_system
            .as_deref()
            .and_then(|s| serde_json::from_str::<CommentSystem>(s).ok())
            .map(|s| s.label()),
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::feed::ParsedItem;
    use crate::sources::DirectoryEntry;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    use url::Url;

    /// Blogs 1–3; blog 3 is not alive. Tags: 1 = blog+tech, 2 = life, 3 = tech.
    /// Five posts each; even posts have a cover and the category 旅行.
    pub(crate) async fn seeded() -> AppState {
        let pool = crate::db::connect_memory().await;
        let tags = |i: i64| -> Vec<String> {
            match i {
                1 => vec!["blog".into(), "tech".into()],
                2 => vec!["life".into()],
                _ => vec!["tech".into()],
            }
        };
        let entries: Vec<DirectoryEntry> = (1..=3)
            .map(|i| DirectoryEntry {
                external_id: i.to_string(),
                name: format!("Blog {i}"),
                homepage: Url::parse(&format!("https://b{i}.example/")).unwrap(),
                feed_url: None,
                tags: tags(i),
                status: "RUN".into(),
                alive: i != 3,
            })
            .collect();
        crate::db::save_directory(&pool, "travellings", &entries, 3600, 1000)
            .await
            .unwrap();
        let now = crate::util::now();
        for blog in 1..=3 {
            for n in 0..5 {
                let item = ParsedItem {
                    guid: format!("{blog}-{n}"),
                    url: Url::parse(&format!("https://b{blog}.example/p/{n}")).unwrap(),
                    title: format!("Post {blog}-{n} 100%"),
                    excerpt: "摘要".into(),
                    cover: (n % 2 == 0).then(|| Url::parse("https://img.example/a.jpg").unwrap()),
                    author: None,
                    tags: if n % 2 == 0 {
                        vec!["旅行".into()]
                    } else {
                        vec!["随笔".into()]
                    },
                    published: chrono::DateTime::from_timestamp(now - 86400 * 10 + (blog * 10 + n) * 3600, 0),
                    body_html: Some("<p>secret full text</p>".into()),
                    comments_url: None,
                    comment_count: (blog == 1 && n == 4).then_some(9),
                };
                crate::db::upsert_post(&pool, blog, &item, now, 0, None).await.unwrap();
            }
        }
        AppState {
            pool,
            site_name: "test".into(),
            directories: Arc::new(vec![DirectoryInfo {
                id: "travellings",
                name: "开往",
                homepage: "x",
                tag_labels: &[("tech", "技术"), ("life", "生活")],
            }]),
        }
    }

    async fn get_json(state: &AppState, uri: &str) -> (StatusCode, serde_json::Value) {
        let resp = router(state.clone())
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn ids(v: &serde_json::Value, key: &str) -> Vec<i64> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| it.pointer(key).unwrap().as_i64().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn latest_paginates_without_gaps_and_hides_inactive() {
        let st = seeded().await;
        let mut seen = Vec::new();
        let mut uri = "/api/posts?mode=latest&limit=4".to_string();
        loop {
            let (status, v) = get_json(&st, &uri).await;
            assert_eq!(status, StatusCode::OK);
            for it in v["items"].as_array().unwrap() {
                assert_ne!(it["blog"]["id"], 3, "inactive blog must be hidden");
                seen.push(it["sort_at"].as_i64().unwrap());
            }
            match v["next_cursor"].as_str() {
                Some(c) => uri = format!("/api/posts?mode=latest&limit=4&cursor={c}"),
                None => break,
            }
        }
        assert_eq!(seen.len(), 10);
        assert!(seen.windows(2).all(|w| w[0] >= w[1]));
    }

    #[tokio::test]
    async fn discover_is_stable_per_seed_and_covers_everything() {
        let st = seeded().await;
        let (_, a) = get_json(&st, "/api/posts?seed=42&limit=100").await;
        let (_, b) = get_json(&st, "/api/posts?seed=42&limit=100").await;
        assert_eq!(a, b);
        assert_eq!(a["items"].as_array().unwrap().len(), 10);
    }

    #[tokio::test]
    async fn filters_by_directory_tag_in_both_modes() {
        let st = seeded().await;
        for mode in ["latest", "discover"] {
            let (_, v) = get_json(&st, &format!("/api/posts?mode={mode}&tag=tech&limit=100")).await;
            let blogs = ids(&v, "/blog/id");
            assert_eq!(blogs.len(), 5, "{mode}: only blog 1 is alive and tagged tech");
            assert!(blogs.iter().all(|b| *b == 1));
        }
        let (_, v) = get_json(&st, "/api/posts?tag=life&directory=travellings&limit=100").await;
        assert!(ids(&v, "/blog/id").iter().all(|b| *b == 2));
        let (_, v) = get_json(&st, "/api/posts?directory=elsewhere&limit=100").await;
        assert!(v["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn filters_cover_and_category() {
        let st = seeded().await;
        let (_, v) = get_json(&st, "/api/posts?cover=1&limit=100").await;
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        assert!(items.iter().all(|it| it["cover"].is_string()));

        let (_, v) = get_json(&st, "/api/posts?category=%E9%9A%8F%E7%AC%94&limit=100").await;
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|it| it["tags"][0] == "随笔"));
    }

    #[tokio::test]
    async fn search_escapes_like_wildcards() {
        let st = seeded().await;
        let (_, v) = get_json(&st, "/api/posts?q=100%25&limit=100").await;
        assert_eq!(v["items"].as_array().unwrap().len(), 10);
        let (_, v) = get_json(&st, "/api/posts?q=_&limit=100").await;
        assert_eq!(v["items"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn tags_endpoint_counts_live_blogs() {
        let st = seeded().await;
        let (_, v) = get_json(&st, "/api/tags").await;
        let tags = v["directories"][0]["tags"].as_array().unwrap();
        let tech = tags.iter().find(|t| t["tag"] == "tech").unwrap();
        assert_eq!(tech["label"], "技术");
        assert_eq!(tech["blogs"], 1);
        assert_eq!(tech["posts"], 5);
        let blog = tags.iter().find(|t| t["tag"] == "blog").unwrap();
        assert_eq!(blog["label"], "blog", "unlabelled tags fall back to the raw tag");
        let cats: Vec<&str> = v["categories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(cats.len(), 2);
        assert!(cats.contains(&"旅行") && cats.contains(&"随笔"));
    }

    #[tokio::test]
    async fn summary_blogs_never_expose_content() {
        let st = seeded().await;
        let (status, v) = get_json(&st, "/api/posts/1").await;
        assert_eq!(status, StatusCode::OK);
        assert!(v["content_html"].is_null());
        assert_eq!(v["has_full_text"], false);
        let (status, _) = get_json(&st, "/api/posts/99999").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn comments_are_exposed_and_embedding_needs_permission() {
        let st = seeded().await;
        sqlx::query(
            r#"UPDATE blogs SET comment_system = '{"kind":"waline","server_url":"https://w.example"}',
                   description = '一个博客', about_text = '关于我',
                   social_links = '[{"kind":"github","url":"https://github.com/x"}]', generator = 'Hexo 7.3.0'
               WHERE id = 1"#,
        )
        .execute(&st.pool)
        .await
        .unwrap();

        // Post 5 is blog 1's fifth post, which carries a feed comment count.
        let (_, v) = get_json(&st, "/api/posts/5").await;
        assert_eq!(v["comment_count"], 9);
        assert_eq!(v["comments"]["system"], "Waline");
        assert!(
            v["comments"]["embed"].is_null(),
            "no embed without the author's permission"
        );

        sqlx::query("UPDATE blogs SET embed_comments = 1 WHERE id = 1")
            .execute(&st.pool)
            .await
            .unwrap();
        let (_, v) = get_json(&st, "/api/posts/5").await;
        assert_eq!(v["comments"]["embed"]["kind"], "waline");
        assert_eq!(v["comments"]["embed"]["server_url"], "https://w.example");
        assert_eq!(v["comments"]["embed"]["path"], "/p/4");

        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(b["description"], "一个博客");
        assert_eq!(b["about_text"], "关于我");
        assert_eq!(b["comment_system"], "Waline");
        assert_eq!(b["generator"], "Hexo 7.3.0");
        assert_eq!(b["social"][0]["kind"], "github");
        let (_, b2) = get_json(&st, "/api/blogs/2").await;
        assert!(b2["comment_system"].is_null());
    }

    #[tokio::test]
    async fn blogs_list_filters_and_labels_tags() {
        let st = seeded().await;
        let (_, v) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(v["name"], "Blog 1");
        assert_eq!(v["post_count"], 5);
        assert_eq!(v["directories"][0]["name"], "开往");
        let labels: Vec<&str> = v["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["blog", "技术"]);

        let (_, list) = get_json(&st, "/api/blogs").await;
        assert_eq!(ids(&list, "/id"), [2, 1], "most recently updated first");
        let (_, list) = get_json(&st, "/api/blogs?sort=name").await;
        assert_eq!(ids(&list, "/id"), [1, 2]);
        let (_, list) = get_json(&st, "/api/blogs?tag=life").await;
        assert_eq!(ids(&list, "/id"), [2]);
    }
}
