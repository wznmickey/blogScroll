use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub site_name: String,
    pub directories: Arc<Vec<DirectoryInfo>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/meta", get(meta))
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

// ---------- meta ----------

#[derive(Serialize)]
struct Meta {
    site_name: String,
    directories: Vec<DirectoryInfo>,
    stats: Stats,
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
    blog_id: i64,
    blog_name: String,
    blog_homepage: String,
    blog_icon: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BlogRef {
    id: i64,
    name: String,
    homepage: String,
    icon: Option<String>,
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
            blog: BlogRef {
                id: r.blog_id,
                name: r.blog_name,
                homepage: r.blog_homepage,
                icon: r.blog_icon,
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
     (p.content_html IS NOT NULL) AS has_full_text,
     b.id AS blog_id, b.name AS blog_name, b.homepage_url AS blog_homepage, b.icon_url AS blog_icon"
    };
}

async fn list_posts(State(st): State<AppState>, Query(q): Query<PostQuery>) -> ApiResult<Page<PostCard>> {
    let limit = q.limit.unwrap_or(30).clamp(1, 100);
    let search =
        q.q.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(like_pattern);
    let discover = q.mode.as_deref() != Some("latest") && q.blog.is_none() && search.is_none();
    if discover {
        discover_posts(&st.pool, limit, q.cursor.as_deref(), q.seed.unwrap_or(0)).await
    } else {
        latest_posts(&st.pool, limit, q.cursor.as_deref(), q.blog, search.as_deref()).await
    }
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

async fn latest_posts(
    pool: &SqlitePool,
    limit: i64,
    cursor: Option<&str>,
    blog: Option<i64>,
    search: Option<&str>,
) -> ApiResult<Page<PostCard>> {
    let (c_sort, c_id) = cursor
        .and_then(|c| c.split_once('.'))
        .and_then(|(a, b)| Some((a.parse::<i64>().ok()?, b.parse::<i64>().ok()?)))
        .map_or((None, None), |(a, b)| (Some(a), Some(b)));
    let rows: Vec<PostRow> = sqlx::query_as(concat!(
        "SELECT ", post_columns!(), "
         FROM posts p JOIN blogs b ON b.id = p.blog_id
         WHERE b.excluded = 0
           AND (?1 IS NOT NULL OR b.active = 1)
           AND (?1 IS NULL OR p.blog_id = ?1)
           AND (?2 IS NULL OR (p.sort_at, p.id) < (?2, ?3))
           AND (?4 IS NULL OR p.title LIKE ?4 ESCAPE '\\' OR p.excerpt LIKE ?4 ESCAPE '\\' OR b.name LIKE ?4 ESCAPE '\\')
         ORDER BY p.sort_at DESC, p.id DESC
         LIMIT ?5"
    ))
    .bind(blog)
    .bind(c_sort)
    .bind(c_id)
    .bind(search)
    .bind(limit)
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
async fn discover_posts(pool: &SqlitePool, limit: i64, cursor: Option<&str>, seed: i64) -> ApiResult<Page<PostCard>> {
    let offset: i64 = cursor.and_then(|c| c.parse().ok()).unwrap_or(0).max(0);
    let seed = seed.rem_euclid(1 << 31);
    let rows: Vec<PostRow> = sqlx::query_as(concat!(
        "WITH ranked AS (
            SELECT p.id AS post_id, ROW_NUMBER() OVER (PARTITION BY p.blog_id ORDER BY p.sort_at DESC) AS rn
            FROM posts p JOIN blogs b ON b.id = p.blog_id
            WHERE b.active = 1 AND b.excluded = 0
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
}

async fn get_post(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<PostDetail> {
    #[derive(FromRow)]
    struct Row {
        #[sqlx(flatten)]
        post: PostRow,
        content_html: Option<String>,
    }
    let row: Option<Row> = sqlx::query_as(concat!(
        "SELECT ",
        post_columns!(),
        ", CASE WHEN b.content_policy = 'full_text' THEN p.content_html END AS content_html
         FROM posts p JOIN blogs b ON b.id = p.blog_id
         WHERE p.id = ? AND b.excluded = 0"
    ))
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let row = row.ok_or_else(not_found)?;
    Ok(Json(PostDetail {
        card: row.post.into(),
        content_html: row.content_html,
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
}

#[derive(Debug, Serialize, FromRow)]
pub struct BlogSummary {
    id: i64,
    name: String,
    homepage: String,
    icon: Option<String>,
    post_count: i64,
    last_post_at: Option<i64>,
    has_feed: bool,
}

macro_rules! blog_summary_sql {
    () => {
        "SELECT b.id, b.name, b.homepage_url AS homepage, b.icon_url AS icon,
        COUNT(p.id) AS post_count, MAX(p.sort_at) AS last_post_at,
        (b.last_success_at IS NOT NULL) AS has_feed
     FROM blogs b LEFT JOIN posts p ON p.blog_id = b.id"
    };
}

async fn list_blogs(State(st): State<AppState>, Query(q): Query<BlogQuery>) -> ApiResult<Page<BlogSummary>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let search =
        q.q.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(like_pattern);
    let rows: Vec<BlogSummary> = sqlx::query_as(concat!(
        "",
        blog_summary_sql!(),
        "
         WHERE b.active = 1 AND b.excluded = 0
           AND (?1 IS NULL OR b.name LIKE ?1 ESCAPE '\\' OR b.homepage_url LIKE ?1 ESCAPE '\\')
         GROUP BY b.id
         ORDER BY last_post_at IS NULL, last_post_at DESC, b.id
         LIMIT ?2 OFFSET ?3"
    ))
    .bind(search)
    .bind(limit)
    .bind(offset)
    .fetch_all(&st.pool)
    .await?;
    let next_cursor = (rows.len() as i64 == limit).then(|| (offset + limit).to_string());
    Ok(Json(Page {
        items: rows,
        next_cursor,
    }))
}

#[derive(Debug, Serialize)]
pub struct BlogDetail {
    #[serde(flatten)]
    summary: BlogSummary,
    full_text: bool,
    directories: Vec<BlogDirectory>,
}

#[derive(Debug, Serialize)]
pub struct BlogDirectory {
    directory: String,
    name: String,
    tags: Vec<String>,
}

async fn get_blog(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<BlogDetail> {
    let summary: Option<BlogSummary> = sqlx::query_as(concat!(
        blog_summary_sql!(),
        " WHERE b.id = ? AND b.excluded = 0 GROUP BY b.id"
    ))
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let summary = summary.ok_or_else(not_found)?;
    let full_text: bool = sqlx::query_scalar("SELECT content_policy = 'full_text' FROM blogs WHERE id = ?")
        .bind(id)
        .fetch_one(&st.pool)
        .await?;
    let dirs: Vec<(String, String)> =
        sqlx::query_as("SELECT directory, tags FROM directory_entries WHERE blog_id = ? AND active = 1")
            .bind(id)
            .fetch_all(&st.pool)
            .await?;
    let directories = dirs
        .into_iter()
        .map(|(directory, tags)| BlogDirectory {
            name: st
                .directories
                .iter()
                .find(|d| d.id == directory)
                .map_or_else(|| directory.clone(), |d| d.name.to_string()),
            directory,
            tags: serde_json::from_str(&tags).unwrap_or_default(),
        })
        .collect();
    Ok(Json(BlogDetail {
        summary,
        full_text,
        directories,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::ParsedItem;
    use crate::sources::DirectoryEntry;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    use url::Url;

    async fn seeded() -> AppState {
        let pool = crate::db::connect_memory().await;
        let entries: Vec<DirectoryEntry> = (1..=3)
            .map(|i| DirectoryEntry {
                external_id: i.to_string(),
                name: format!("Blog {i}"),
                homepage: Url::parse(&format!("https://b{i}.example/")).unwrap(),
                feed_url: None,
                tags: vec!["blog".into()],
                status: "RUN".into(),
                alive: i != 3,
            })
            .collect();
        crate::db::save_directory(&pool, "travellings", &entries, 3600, 1000)
            .await
            .unwrap();
        for blog in 1..=3 {
            for n in 0..5 {
                let item = ParsedItem {
                    guid: format!("{blog}-{n}"),
                    url: Url::parse(&format!("https://b{blog}.example/p/{n}")).unwrap(),
                    title: format!("Post {blog}-{n} 100%"),
                    excerpt: "摘要".into(),
                    cover: (n % 2 == 0).then(|| Url::parse("https://img.example/a.jpg").unwrap()),
                    author: None,
                    tags: vec!["t".into()],
                    published: chrono::DateTime::from_timestamp(1_700_000_000 + (blog * 10 + n) * 3600, 0),
                    body_html: Some("<p>secret full text</p>".into()),
                };
                crate::db::upsert_post(&pool, blog, &item, 1_800_000_000, 0, None)
                    .await
                    .unwrap();
            }
        }
        AppState {
            pool,
            site_name: "test".into(),
            directories: Arc::new(vec![DirectoryInfo {
                id: "travellings",
                name: "开往",
                homepage: "x",
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
    async fn search_escapes_like_wildcards() {
        let st = seeded().await;
        let (_, v) = get_json(&st, "/api/posts?q=100%25&limit=100").await;
        assert_eq!(v["items"].as_array().unwrap().len(), 10);
        let (_, v) = get_json(&st, "/api/posts?q=_&limit=100").await;
        assert_eq!(v["items"].as_array().unwrap().len(), 0);
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
    async fn blog_detail_lists_directories() {
        let st = seeded().await;
        let (_, v) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(v["name"], "Blog 1");
        assert_eq!(v["post_count"], 5);
        assert_eq!(v["directories"][0]["name"], "开往");
        let (_, list) = get_json(&st, "/api/blogs").await;
        assert_eq!(list["items"].as_array().unwrap().len(), 2);
    }
}
