use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::analyze;
use crate::comments::{CommentSystem, EmbedConfig};
use crate::profile::SocialLink;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub site_name: String,
    pub directories: Arc<Vec<DirectoryInfo>>,
    pub maintainer: Option<crate::config::Maintainer>,
    /// Waterfall columns on the widest screens.
    pub columns: u32,
    /// The source repository, and the config file in it (`[server] repo`).
    pub repo: Option<String>,
    pub repo_config: String,
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
        .route("/api/tags/search", get(search_tags))
        .route("/api/posts", get(list_posts))
        .route("/api/posts/{id}", get(get_post))
        .route("/api/posts/{id}/related", get(related_posts))
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

/// Sources shown anywhere: listed by a directory, not opted out, not spam.
macro_rules! visible {
    () => {
        "b.active = 1 AND b.excluded = 0 AND b.flagged IS NULL"
    };
}

/// Parses `1,2,3` into a JSON array for `json_each`, at most `max` ids.
fn id_list(s: &Option<String>, max: usize) -> Option<String> {
    let ids: Vec<i64> = non_empty(s)?
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .take(max)
        .collect();
    Some(serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into()))
}

#[derive(Debug, Serialize)]
pub struct KindRef {
    id: String,
    label: &'static str,
}

impl KindRef {
    fn new(id: &str) -> Self {
        KindRef {
            id: id.to_string(),
            label: analyze::kind_label(id),
        }
    }
}

/// A post's category (`tech/ai`, `travel`, `other`), or one of a blog's.
#[derive(Debug, Serialize)]
pub struct CategoryRef {
    id: String,
    label: &'static str,
    /// The top-level category of a sub-category.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<CategoryParent>,
    /// Blogs only: share of the blog's classified posts.
    #[serde(skip_serializing_if = "Option::is_none")]
    share: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct CategoryParent {
    id: &'static str,
    label: &'static str,
}

impl CategoryRef {
    fn new(id: &str) -> Self {
        let (id, (label, parent)) = match analyze::category(id) {
            Some(c) => (id, c),
            None => (
                analyze::OTHER,
                analyze::category(analyze::OTHER).unwrap_or(("其他", None)),
            ),
        };
        CategoryRef {
            id: id.to_string(),
            label,
            parent: parent.map(|p| CategoryParent {
                id: p.id,
                label: p.label,
            }),
            share: None,
        }
    }
}

fn blog_categories(json: &str) -> Vec<CategoryRef> {
    serde_json::from_str::<Vec<analyze::BlogCategory>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| analyze::category(&c.id).is_some())
        .map(|c| CategoryRef {
            share: Some(c.share),
            ..CategoryRef::new(&c.id)
        })
        .collect()
}

// ---------- meta ----------

#[derive(Serialize)]
struct Meta {
    site_name: String,
    directories: Vec<DirectoryInfo>,
    /// Who runs the site, and their blog here if it is one of ours.
    maintainer: Option<MaintainerInfo>,
    /// Waterfall columns on the widest screens (`[server] columns`).
    columns: u32,
    /// Where bloggers ask for changes: the source repository and its config file.
    repo: Option<RepoInfo>,
    stats: Stats,
    /// When this answer was produced; tells static exports' readers how fresh they are.
    generated_at: i64,
}

#[derive(Serialize)]
struct RepoInfo {
    url: String,
    config_path: String,
}

#[derive(Serialize)]
struct MaintainerInfo {
    name: String,
    url: Option<String>,
    blog: Option<MaintainerBlog>,
}

#[derive(Serialize)]
struct MaintainerBlog {
    id: i64,
    name: String,
}

#[derive(Serialize, FromRow)]
struct Stats {
    blogs: i64,
    blogs_with_feed: i64,
    posts: i64,
    last_directory_sync: Option<i64>,
}

async fn meta(State(st): State<AppState>) -> ApiResult<Meta> {
    let stats: Stats = sqlx::query_as(concat!(
        "SELECT
            (SELECT COUNT(*) FROM blogs b WHERE ",
        visible!(),
        ") AS blogs,
            (SELECT COUNT(*) FROM blogs b WHERE ",
        visible!(),
        " AND b.last_success_at IS NOT NULL) AS blogs_with_feed,
            (SELECT COUNT(*) FROM posts p JOIN blogs b ON b.id = p.blog_id WHERE ",
        visible!(),
        ") AS posts,
            (SELECT CAST(value AS INTEGER) FROM kv WHERE key = 'last_directory_sync') AS last_directory_sync"
    ))
    .fetch_one(&st.pool)
    .await?;
    let maintainer = match &st.maintainer {
        Some(m) => {
            let key = m.url.as_deref().and_then(crate::util::site_key);
            let blog: Option<(i64, String)> = match key {
                Some(k) => {
                    sqlx::query_as(concat!(
                        "SELECT b.id, b.name FROM blogs b WHERE b.site_key = ? AND ",
                        visible!()
                    ))
                    .bind(k)
                    .fetch_optional(&st.pool)
                    .await?
                }
                None => None,
            };
            Some(MaintainerInfo {
                name: m.name.clone(),
                url: m.url.clone(),
                blog: blog.map(|(id, name)| MaintainerBlog { id, name }),
            })
        }
        None => None,
    };
    Ok(Json(Meta {
        site_name: st.site_name.clone(),
        directories: st.directories.to_vec(),
        maintainer,
        columns: st.columns,
        repo: st.repo.as_ref().map(|url| RepoInfo {
            url: url.trim_end_matches('/').to_string(),
            config_path: st.repo_config.clone(),
        }),
        stats,
        generated_at: crate::util::now(),
    }))
}

// ---------- tags ----------

#[derive(Serialize)]
struct TagsResponse {
    /// Directory tags (开往: 技术 / 生活 / ...).
    directories: Vec<DirectoryTags>,
    /// The category tree, in channel order, with post counts.
    categories: Vec<CategoryCount>,
    /// Source kinds present (博客 / 资讯 / 播客 ...).
    kinds: Vec<KindCount>,
    /// Popular tags: the authors' own plus those derived from keywords, last 180 days.
    popular_tags: Vec<TagStat>,
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

#[derive(Serialize)]
struct CategoryCount {
    /// `tech`, or `tech/ai` for a sub-category.
    id: String,
    label: &'static str,
    /// Top-level only: blogs whose main categories include this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    blogs: Option<i64>,
    /// Including those of its sub-categories.
    posts: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<CategoryCount>,
}

#[derive(Serialize)]
struct KindCount {
    id: String,
    label: &'static str,
    blogs: i64,
}

#[derive(Serialize, FromRow)]
struct TagStat {
    name: String,
    blogs: i64,
    posts: i64,
}

async fn tags(State(st): State<AppState>) -> ApiResult<TagsResponse> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(concat!(
        "SELECT bt.directory, bt.tag, COUNT(DISTINCT b.id) AS blogs, COUNT(p.id) AS posts
         FROM blog_tags bt
         JOIN blogs b ON b.id = bt.blog_id
         LEFT JOIN posts p ON p.blog_id = b.id
         WHERE ",
        visible!(),
        "
         GROUP BY bt.directory, bt.tag
         ORDER BY blogs DESC, bt.tag"
    ))
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

    let category_posts: Vec<(String, i64)> = sqlx::query_as(concat!(
        "SELECT p.category, COUNT(*) FROM posts p JOIN blogs b ON b.id = p.blog_id
         WHERE ",
        visible!(),
        " GROUP BY p.category"
    ))
    .fetch_all(&st.pool)
    .await?;
    let category_blogs: Vec<(String, i64)> = sqlx::query_as(concat!(
        "SELECT json_extract(j.value, '$.id'), COUNT(*) FROM blogs b, json_each(b.categories) j
         WHERE ",
        visible!(),
        " GROUP BY 1"
    ))
    .fetch_all(&st.pool)
    .await?;
    let count = |rows: &[(String, i64)], id: &str| rows.iter().find(|(t, _)| t == id).map_or(0, |(_, n)| *n);
    let categories = analyze::CATEGORIES
        .iter()
        .map(|c| {
            let children: Vec<CategoryCount> = analyze::sub_categories(c.id)
                .map(|t| {
                    let id = t.path();
                    CategoryCount {
                        posts: count(&category_posts, &id),
                        id,
                        label: t.label,
                        blogs: None,
                        children: Vec::new(),
                    }
                })
                .collect();
            CategoryCount {
                id: c.id.to_string(),
                label: c.label,
                blogs: Some(count(&category_blogs, c.id)),
                posts: count(&category_posts, c.id) + children.iter().map(|s| s.posts).sum::<i64>(),
                children,
            }
        })
        .collect();

    let kind_rows: Vec<(String, i64)> = sqlx::query_as(concat!(
        "SELECT COALESCE(b.kind, b.detected_kind, 'blog') AS k, COUNT(*) FROM blogs b WHERE ",
        visible!(),
        " GROUP BY k ORDER BY 2 DESC"
    ))
    .fetch_all(&st.pool)
    .await?;
    let kinds = kind_rows
        .into_iter()
        .map(|(id, blogs)| KindCount {
            label: analyze::kind_label(&id),
            id,
            blogs,
        })
        .collect();

    let since = crate::util::now() - 180 * 86400;
    let popular_tags: Vec<TagStat> = sqlx::query_as(concat!(
        "SELECT COALESCE(l.label, pt.term) AS name, COUNT(DISTINCT p.blog_id) AS blogs, COUNT(*) AS posts
         FROM post_terms pt
         JOIN posts p ON p.id = pt.post_id
         JOIN blogs b ON b.id = p.blog_id
         LEFT JOIN tag_labels l ON l.term = pt.term
         WHERE pt.is_tag = 1 AND p.sort_at >= ? AND ",
        visible!(),
        "
         GROUP BY pt.term
         HAVING blogs >= 2
         ORDER BY blogs DESC, posts DESC, name
         LIMIT 40"
    ))
    .bind(since)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(TagsResponse {
        directories,
        categories,
        kinds,
        popular_tags,
    }))
}

#[derive(Debug, Deserialize)]
pub struct TagSearchQuery {
    /// Part of a tag's name; without it, all tags.
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

/// Tags whose name contains `q` (an exact match first), most widely used first.
async fn search_tags(State(st): State<AppState>, Query(q): Query<TagSearchQuery>) -> ApiResult<Vec<TagStat>> {
    let limit = q.limit.unwrap_or(20).clamp(1, 5000);
    let needle = non_empty(&q.q);
    let rows: Vec<TagStat> = sqlx::query_as(concat!(
        "SELECT COALESCE(l.label, pt.term) AS name, COUNT(DISTINCT p.blog_id) AS blogs, COUNT(*) AS posts
         FROM post_terms pt
         JOIN posts p ON p.id = pt.post_id
         JOIN blogs b ON b.id = p.blog_id
         LEFT JOIN tag_labels l ON l.term = pt.term
         WHERE pt.is_tag = 1 AND ",
        visible!(),
        "
           AND (?1 IS NULL OR pt.term LIKE ?1 ESCAPE '\\' OR l.label LIKE ?1 ESCAPE '\\')
         GROUP BY pt.term
         HAVING ?1 IS NOT NULL OR posts >= 2
         ORDER BY pt.term = ?2 DESC, blogs DESC, posts DESC, name
         LIMIT ?3"
    ))
    .bind(needle.map(like_pattern))
    .bind(needle.map(analyze::tag_key))
    .bind(limit)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(rows))
}

// ---------- posts ----------

#[derive(Debug, Deserialize)]
pub struct PostQuery {
    /// `discover` (default): recent posts, lightly shuffled, at most a few per
    /// blog; with the reader's signals it leans towards their interests.
    /// `explore`: the same, leaning away from them (areas and blogs not read yet).
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
    /// Title, excerpt, blog name, or exactly a tag.
    #[serde(default)]
    q: Option<String>,
    /// Directory tag of the blog, e.g. `tech` (开往).
    #[serde(default)]
    dirtag: Option<String>,
    /// Only blogs listed in this directory, e.g. `travellings`.
    #[serde(default)]
    directory: Option<String>,
    /// A tag of the post: the author's own or one derived from its keywords.
    #[serde(default)]
    tag: Option<String>,
    /// `tech` (with its sub-categories), `tech/ai`, ... (see `analyze::CATEGORIES`).
    #[serde(default)]
    category: Option<String>,
    /// Source kind, e.g. `podcast`.
    #[serde(default)]
    kind: Option<String>,
    /// Only these blogs, `1,2,3` (the "following" feed).
    #[serde(default)]
    blogs: Option<String>,
    /// Personalised discover: blogs the reader follows (`1,2,3`), ...
    #[serde(default)]
    follow: Option<String>,
    /// ... posts they liked, saved, and recently opened (`4,5,6`).
    #[serde(default)]
    like: Option<String>,
    #[serde(default)]
    save: Option<String>,
    #[serde(default)]
    read: Option<String>,
    /// Posts recently shown to the reader without being opened (`7,8,9`):
    /// a little further down, so the same ones do not keep coming back.
    #[serde(default)]
    shown: Option<String>,
    /// `1`: only posts with a cover image; `none`: only posts without one.
    #[serde(default)]
    cover: Option<String>,
    /// `1`: only posts with some text: an excerpt of at least 20 characters
    /// (below that, cards show the title instead). Ignored with `cover`.
    #[serde(default)]
    text: Option<String>,
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
    auto_tags: String,
    category: String,
    published_at: Option<i64>,
    sort_at: i64,
    undated: bool,
    has_full_text: bool,
    comment_count: Option<i64>,
    comments_url: Option<String>,
    blog_id: i64,
    blog_name: String,
    blog_homepage: String,
    blog_icon: Option<String>,
    blog_tags: String,
    blog_kind: String,
    blog_framable: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct BlogRef {
    id: i64,
    name: String,
    homepage: String,
    icon: Option<String>,
    /// Directory tags of the blog (e.g. `tech`), so clients can filter cards.
    tags: Vec<String>,
    kind: String,
    /// Whether its pages may be shown in a frame on this site (`null` = not known yet).
    framable: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PostCard {
    id: i64,
    title: String,
    url: String,
    excerpt: String,
    cover: Option<String>,
    author: Option<String>,
    /// The author's own tags and categories.
    tags: Vec<String>,
    /// Tags derived from the post's keywords.
    auto_tags: Vec<String>,
    /// Exactly one, from `analyze::CATEGORIES`.
    category: CategoryRef,
    published_at: Option<i64>,
    sort_at: i64,
    /// Age unknown: the feed gave no date and the post came with the first fetch.
    undated: bool,
    has_full_text: bool,
    /// From the feed (`slash:comments`) or the blog's comment service.
    comment_count: Option<i64>,
    comments_url: Option<String>,
    blog: BlogRef,
}

impl From<PostRow> for PostCard {
    fn from(r: PostRow) -> Self {
        let tags: Vec<String> = serde_json::from_str(&r.tags).unwrap_or_default();
        PostCard {
            id: r.id,
            title: r.title,
            url: r.url,
            excerpt: r.excerpt,
            cover: r.cover_url,
            author: r.author,
            tags: tags.iter().map(|t| analyze::tag_display(t)).collect(),
            auto_tags: serde_json::from_str(&r.auto_tags).unwrap_or_default(),
            category: CategoryRef::new(&r.category),
            published_at: r.published_at,
            sort_at: r.sort_at,
            undated: r.undated,
            has_full_text: r.has_full_text,
            comment_count: r.comment_count,
            comments_url: r.comments_url,
            blog: BlogRef {
                id: r.blog_id,
                name: r.blog_name,
                homepage: r.blog_homepage,
                icon: r.blog_icon,
                tags: serde_json::from_str(&r.blog_tags).unwrap_or_default(),
                kind: r.blog_kind,
                framable: r.blog_framable,
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
        "p.id, p.title, p.url, p.excerpt, p.cover_url, p.author, p.tags, p.auto_tags, p.category,
     p.published_at, p.sort_at, p.undated,
     (p.content_html IS NOT NULL) AS has_full_text, p.comment_count, p.comments_url,
     b.id AS blog_id, b.name AS blog_name, b.homepage_url AS blog_homepage,
     COALESCE(b.avatar_custom, b.avatar_url, b.icon_url) AS blog_icon,
     (SELECT json_group_array(DISTINCT bt.tag) FROM blog_tags bt WHERE bt.blog_id = b.id) AS blog_tags,
     COALESCE(b.kind, b.detected_kind, 'blog') AS blog_kind, b.framable AS blog_framable"
    };
}

/// Filters shared by both feed modes, on parameters ?10–?16: directory tag,
/// directory, with or without a cover or with text, tag key, category (a top-level one includes its
/// sub-categories), kind, blog ids (JSON).
macro_rules! post_filters {
    () => {
        " AND (?10 IS NULL OR EXISTS (SELECT 1 FROM blog_tags bt WHERE bt.blog_id = b.id AND bt.tag = ?10))
          AND (?11 IS NULL OR EXISTS (SELECT 1 FROM directory_entries de
                 WHERE de.blog_id = b.id AND de.active = 1 AND de.directory = ?11))
          AND (?12 IS NULL OR (?12 = 'cover' AND p.cover_url IS NOT NULL) OR (?12 = 'none' AND p.cover_url IS NULL)
               OR (?12 = 'text' AND length(trim(p.excerpt)) >= 20))
          AND (?13 IS NULL OR EXISTS (SELECT 1 FROM post_terms pt WHERE pt.post_id = p.id AND pt.is_tag = 1 AND pt.term = ?13))
          AND (?14 IS NULL OR p.category = ?14 OR substr(p.category, 1, length(?14) + 1) = ?14 || '/')
          AND (?15 IS NULL OR COALESCE(b.kind, b.detected_kind, 'blog') = ?15)
          AND (?16 IS NULL OR b.id IN (SELECT value FROM json_each(?16)))"
    };
}

struct Filters<'a> {
    dirtag: Option<&'a str>,
    directory: Option<&'a str>,
    /// `cover`: only with a cover image, `none`: only without, `text`:
    /// only with an excerpt worth reading.
    media: Option<&'static str>,
    tag: Option<String>,
    category: Option<&'a str>,
    kind: Option<&'a str>,
    blogs: Option<String>,
}

impl Filters<'_> {
    /// Binds ?10–?16 in `post_filters!` order.
    fn bind<'q, O>(
        &'q self,
        q: sqlx::query::QueryAs<'q, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments>,
    ) -> sqlx::query::QueryAs<'q, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments> {
        q.bind(self.dirtag)
            .bind(self.directory)
            .bind(self.media)
            .bind(self.tag.as_deref())
            .bind(self.category)
            .bind(self.kind)
            .bind(self.blogs.as_deref())
    }
}

/// What the reader has shown interest in; empty for the plain discover feed.
struct Signals {
    follow: Option<String>,
    like: Option<String>,
    save: Option<String>,
    read: Option<String>,
    shown: Option<String>,
}

impl Signals {
    fn is_empty(&self) -> bool {
        self.follow.is_none()
            && self.like.is_none()
            && self.save.is_none()
            && self.read.is_none()
            && self.shown.is_none()
    }

    /// Posts the reader has already seen (liked, saved or opened), as a JSON array.
    fn seen(&self) -> String {
        let mut ids: Vec<i64> = Vec::new();
        for list in [&self.like, &self.save, &self.read].into_iter().flatten() {
            ids.extend(serde_json::from_str::<Vec<i64>>(list).unwrap_or_default());
        }
        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into())
    }
}

async fn list_posts(State(st): State<AppState>, Query(q): Query<PostQuery>) -> ApiResult<Page<PostCard>> {
    let limit = q.limit.unwrap_or(30).clamp(1, 100);
    let search = non_empty(&q.q);
    let f = Filters {
        dirtag: non_empty(&q.dirtag),
        directory: non_empty(&q.directory),
        media: match non_empty(&q.cover) {
            Some("none" | "no" | "without") => Some("none"),
            _ if truthy(&q.cover) => Some("cover"),
            _ if truthy(&q.text) => Some("text"),
            _ => None,
        },
        tag: non_empty(&q.tag).map(analyze::tag_key),
        category: non_empty(&q.category),
        kind: non_empty(&q.kind),
        blogs: id_list(&q.blogs, 500),
    };
    let discover = q.mode.as_deref() != Some("latest") && q.blog.is_none() && search.is_none() && f.blogs.is_none();
    if discover {
        let signals = Signals {
            follow: id_list(&q.follow, 500),
            like: id_list(&q.like, 200),
            save: id_list(&q.save, 200),
            read: id_list(&q.read, 100),
            shown: id_list(&q.shown, 300),
        };
        let explore = q.mode.as_deref() == Some("explore");
        let interest = interest(&st.pool, &signals, explore).await?;
        let leaning = if explore { &EXPLORE } else { &FOR_YOU };
        let seed = q.seed.unwrap_or(0);
        discover_posts(&st.pool, limit, q.cursor.as_deref(), seed, &interest, leaning, &f).await
    } else {
        latest_posts(&st.pool, limit, q.cursor.as_deref(), q.blog, search, &f).await
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
    let query = sqlx::query_as(concat!(
        "SELECT ",
        post_columns!(),
        "
         FROM posts p JOIN blogs b ON b.id = p.blog_id
         WHERE b.excluded = 0 AND b.flagged IS NULL
           AND (?1 IS NOT NULL OR b.active = 1)
           AND (?1 IS NULL OR p.blog_id = ?1)
           AND (?2 IS NULL OR (p.sort_at, p.id) < (?2, ?3))
           AND (?4 IS NULL OR p.title LIKE ?4 ESCAPE '\\' OR p.excerpt LIKE ?4 ESCAPE '\\' OR b.name LIKE ?4 ESCAPE '\\'
                OR EXISTS (SELECT 1 FROM post_terms pt WHERE pt.post_id = p.id AND pt.is_tag = 1 AND pt.term = ?6))",
        post_filters!(),
        "
         ORDER BY p.sort_at DESC, p.id DESC
         LIMIT ?5"
    ))
    .bind(blog)
    .bind(c_sort)
    .bind(c_id)
    .bind(search.map(like_pattern))
    .bind(limit)
    .bind(search.map(analyze::tag_key))
    .bind(None::<i64>)
    .bind(None::<i64>)
    .bind(None::<i64>);
    let rows: Vec<PostRow> = f.bind(query).fetch_all(pool).await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last().map(|r| format!("{}.{}", r.sort_at, r.id)))
        .flatten();
    Ok(Json(Page {
        items: rows.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

/// How much each signal counts towards the reader's interests. A followed
/// blog counts once in total, spread over its posts of the last year.
const LIKE_WEIGHT: f64 = 3.0;
const SAVE_WEIGHT: f64 = 3.0;
const READ_WEIGHT: f64 = 1.0;
const FOLLOW_WEIGHT: f64 = 2.0;

/// The reader's interests, for personalised discover.
struct Interest {
    /// Top terms of the weighted sum of the signals' term vectors, unit
    /// length: `{term: weight}`.
    terms: String,
    /// Share of the signals in each category, under both the full id and the
    /// top-level one: `{"tech/ai": 0.3, "tech": 0.5}`.
    categories: String,
    /// Blogs the reader knows: followed ones, and when exploring also those
    /// of posts they liked, saved or opened. Posts already seen, and posts
    /// shown to them without being opened.
    blogs: String,
    seen: String,
    shown: String,
}

impl Interest {
    fn none() -> Self {
        Interest {
            terms: "{}".into(),
            categories: "{}".into(),
            blogs: "[]".into(),
            seen: "[]".into(),
            shown: "[]".into(),
        }
    }
}

/// The reader's signals as weighted posts, on parameters ?1–?9 (see `Signals::bind`).
macro_rules! signal_posts {
    () => {
        "WITH signal(post_id, w) AS (
            SELECT value, ?1 FROM json_each(?2)
            UNION ALL SELECT value, ?3 FROM json_each(?4)
            UNION ALL SELECT value, ?5 FROM json_each(?6)
            UNION ALL SELECT p.id, ?7 / n.posts FROM posts p
              JOIN (SELECT blog_id, COUNT(*) AS posts FROM posts
                    WHERE blog_id IN (SELECT value FROM json_each(?8)) AND sort_at >= ?9
                    GROUP BY blog_id) n ON n.blog_id = p.blog_id
              WHERE p.sort_at >= ?9
         ) "
    };
}

impl Signals {
    fn bind<'q>(
        &'q self,
        sql: &'static str,
    ) -> sqlx::query::QueryAs<'q, sqlx::Sqlite, (String, f64), sqlx::sqlite::SqliteArguments> {
        sqlx::query_as(sql)
            .bind(LIKE_WEIGHT)
            .bind(self.like.as_deref().unwrap_or("[]"))
            .bind(SAVE_WEIGHT)
            .bind(self.save.as_deref().unwrap_or("[]"))
            .bind(READ_WEIGHT)
            .bind(self.read.as_deref().unwrap_or("[]"))
            .bind(FOLLOW_WEIGHT)
            .bind(self.follow.as_deref().unwrap_or("[]"))
            .bind(crate::util::now() - 365 * 86400)
    }
}

async fn interest(pool: &SqlitePool, s: &Signals, explore: bool) -> sqlx::Result<Interest> {
    if s.is_empty() {
        return Ok(Interest::none());
    }
    let terms: Vec<(String, f64)> = s
        .bind(concat!(
            signal_posts!(),
            "SELECT pt.term, SUM(pt.weight * s.w) AS w
             FROM signal s JOIN post_terms pt ON pt.post_id = s.post_id
             GROUP BY pt.term
             ORDER BY w DESC
             LIMIT 40"
        ))
        .fetch_all(pool)
        .await?;
    let norm = terms.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
    let terms: serde_json::Map<String, serde_json::Value> = terms
        .into_iter()
        .filter(|_| norm > 0.0)
        .map(|(t, w)| (t, serde_json::json!((w / norm * 10000.0).round() / 10000.0)))
        .collect();

    let per_category: Vec<(String, f64)> = s
        .bind(concat!(
            signal_posts!(),
            "SELECT p.category, SUM(s.w) FROM signal s JOIN posts p ON p.id = s.post_id
             WHERE p.category <> 'other'
             GROUP BY p.category"
        ))
        .fetch_all(pool)
        .await?;
    let total: f64 = per_category.iter().map(|(_, w)| w).sum();
    let mut shares: std::collections::BTreeMap<String, f64> = Default::default();
    for (id, w) in &per_category {
        let share = w / total;
        *shares.entry(id.clone()).or_default() += share;
        if id.contains('/') {
            *shares.entry(analyze::top_of(id).to_string()).or_default() += share;
        }
    }
    let categories: serde_json::Map<String, serde_json::Value> = shares
        .into_iter()
        .map(|(id, share)| (id, serde_json::json!((share * 1000.0).round() / 1000.0)))
        .collect();

    let seen = s.seen();
    let blogs = if explore {
        let read: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT blog_id FROM posts WHERE id IN (SELECT value FROM json_each(?))
             UNION SELECT value FROM json_each(?)",
        )
        .bind(&seen)
        .bind(s.follow.as_deref().unwrap_or("[]"))
        .fetch_all(pool)
        .await?;
        serde_json::to_string(&read).unwrap_or_else(|_| "[]".into())
    } else {
        s.follow.clone().unwrap_or_else(|| "[]".into())
    };
    Ok(Interest {
        terms: serde_json::Value::Object(terms).to_string(),
        categories: serde_json::Value::Object(categories).to_string(),
        blogs,
        seen,
        shown: s.shown.clone().unwrap_or_else(|| "[]".into()),
    })
}

/// How discover weighs the reader's interests, in days of freshness: a
/// perfect match with them, a category that makes up all of their signals,
/// a blog they know, a post they have already seen and one shown to them
/// but passed over (both always down). And how many of each blog's newest
/// posts take part.
struct Leaning {
    interest: f64,
    category: f64,
    blog: f64,
    seen: f64,
    shown: f64,
    per_blog: i64,
}

/// 推荐: towards what the reader likes, and the blogs they follow.
const FOR_YOU: Leaning = Leaning {
    interest: 40.0,
    category: 12.0,
    blog: 5.0,
    seen: 60.0,
    shown: 10.0,
    per_blog: 8,
};

/// 反推荐: the other way round. What is close to their reading, in their usual
/// categories or from blogs they already know moves down, and fewer posts
/// per blog take part, so the feed opens onto areas and blogs they have not
/// been to.
const EXPLORE: Leaning = Leaning {
    interest: -40.0,
    category: -24.0,
    blog: -20.0,
    seen: 60.0,
    shown: 10.0,
    per_blog: 3,
};

/// Score = age in days, plus up to two weeks of seeded noise, minus five days
/// for each newer post of the same blog. Keeps the feed fresh, mixed, and
/// stops one prolific blog from filling a screen. Then the reader's
/// interests, as the `Leaning` says. Undated backlog sorts last.
async fn discover_posts(
    pool: &SqlitePool,
    limit: i64,
    cursor: Option<&str>,
    seed: i64,
    interest: &Interest,
    leaning: &Leaning,
    f: &Filters<'_>,
) -> ApiResult<Page<PostCard>> {
    let offset: i64 = cursor.and_then(|c| c.parse().ok()).unwrap_or(0).max(0);
    let seed = seed.rem_euclid(1 << 31);
    let query = sqlx::query_as(concat!(
        "WITH ranked AS (
            SELECT p.id AS post_id, ROW_NUMBER() OVER (PARTITION BY p.blog_id ORDER BY p.sort_at DESC) AS rn
            FROM posts p JOIN blogs b ON b.id = p.blog_id
            WHERE ",
        visible!(),
        post_filters!(),
        "
         ),
         interest AS (SELECT key AS term, value AS w FROM json_each(?4)),
         sim AS (
            SELECT pt.post_id, SUM(pt.weight * i.w) AS s
            FROM post_terms pt JOIN interest i ON i.term = pt.term
            GROUP BY pt.post_id
         ),
         cat AS (SELECT key AS id, value AS w FROM json_each(?6))
         SELECT ",
        post_columns!(),
        "
         FROM ranked r JOIN posts p ON p.id = r.post_id JOIN blogs b ON b.id = p.blog_id
         LEFT JOIN sim ON sim.post_id = p.id
         LEFT JOIN cat c1 ON c1.id = p.category
         LEFT JOIN cat c2 ON c2.id = substr(p.category, 1, instr(p.category || '/', '/') - 1)
                         AND c2.id <> p.category
         WHERE r.rn <= ?9
         ORDER BY p.undated,
                  p.sort_at / 86400.0
                  + ((p.id * 2654435761 + ?1) % 1000003) / 1000003.0 * 14.0
                  - (r.rn - 1) * 5.0
                  + COALESCE(sim.s, 0) * ?5
                  + (COALESCE(c1.w, 0) + COALESCE(c2.w, 0)) * ?17
                  + (p.blog_id IN (SELECT value FROM json_each(?7))) * ?18
                  - (p.id IN (SELECT value FROM json_each(?8))) * ?19
                  - (p.id IN (SELECT value FROM json_each(?20))) * ?21 DESC,
                  p.id DESC
         LIMIT ?2 OFFSET ?3"
    ))
    .bind(seed)
    .bind(limit)
    .bind(offset)
    .bind(&interest.terms)
    .bind(leaning.interest)
    .bind(&interest.categories)
    .bind(&interest.blogs)
    .bind(&interest.seen)
    .bind(leaning.per_blog);
    let rows: Vec<PostRow> = f
        .bind(query)
        .bind(leaning.category)
        .bind(leaning.blog)
        .bind(leaning.seen)
        .bind(&interest.shown)
        .bind(leaning.shown)
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
         WHERE p.id = ? AND b.excluded = 0 AND b.flagged IS NULL"
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

#[derive(Debug, Deserialize)]
pub struct RelatedQuery {
    #[serde(default)]
    limit: Option<i64>,
}

/// Similar posts by cosine similarity of term vectors (tags + keywords),
/// at most two per blog so one prolific blog cannot take over.
async fn related_posts(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<RelatedQuery>,
) -> ApiResult<Vec<PostCard>> {
    let limit = q.limit.unwrap_or(8).clamp(1, 24) as usize;
    let ids = related_ids(&st.pool, id, limit).await?;
    Ok(Json(cards_by_id(&st.pool, &ids).await?))
}

pub(crate) async fn related_ids(pool: &SqlitePool, id: i64, limit: usize) -> sqlx::Result<Vec<i64>> {
    const MIN_SCORE: f64 = 0.12;
    const PER_BLOG: usize = 2;
    let own: String = sqlx::query_scalar("SELECT category FROM posts WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .unwrap_or_else(|| analyze::OTHER.to_string());
    let candidates: Vec<(i64, i64, f64, i64, String, String)> = sqlx::query_as(concat!(
        "SELECT p.id, p.blog_id, SUM(pt.weight * me.weight) AS score, COUNT(*) AS shared, p.category, p.title
         FROM post_terms me
         JOIN post_terms pt ON pt.term = me.term AND pt.post_id <> me.post_id
         JOIN posts p ON p.id = pt.post_id
         JOIN blogs b ON b.id = p.blog_id
         WHERE me.post_id = ? AND ",
        visible!(),
        "
         GROUP BY p.id
         ORDER BY score DESC, p.sort_at DESC
         LIMIT 200"
    ))
    .bind(id)
    .fetch_all(pool)
    .await?;
    // One shared word is weak evidence ("流程" links a laundry guide to a
    // data-pipeline post); agreeing categories are strong evidence.
    let mut scored: Vec<(i64, i64, f64, String)> = candidates
        .into_iter()
        .map(|(pid, blog, score, shared, category, title)| {
            let mut s = score;
            if shared < 2 {
                s *= 0.4;
            }
            if own != analyze::OTHER && category != analyze::OTHER {
                s *= if category == own {
                    1.25
                } else if analyze::top_of(&category) == analyze::top_of(&own) {
                    1.1
                } else {
                    0.7
                };
            }
            (pid, blog, s, title)
        })
        .collect();
    scored.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut per_blog: std::collections::HashMap<i64, usize> = Default::default();
    // Some blogs publish one post under two URLs (e.g. a Chinese and an English slug).
    let mut titles = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (pid, blog, score, title) in scored {
        if score < MIN_SCORE || out.len() == limit {
            break;
        }
        let n = per_blog.entry(blog).or_default();
        if *n < PER_BLOG && titles.insert((blog, title)) {
            *n += 1;
            out.push(pid);
        }
    }
    Ok(out)
}

/// Cards for these ids, in the given order (hidden ones are skipped).
async fn cards_by_id(pool: &SqlitePool, ids: &[i64]) -> sqlx::Result<Vec<PostCard>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<PostRow> = sqlx::query_as(concat!(
        "SELECT ",
        post_columns!(),
        " FROM posts p JOIN blogs b ON b.id = p.blog_id
         WHERE p.id IN (SELECT value FROM json_each(?)) AND ",
        visible!()
    ))
    .bind(serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()))
    .fetch_all(pool)
    .await?;
    let mut by_id: std::collections::HashMap<i64, PostRow> = rows.into_iter().map(|r| (r.id, r)).collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).map(Into::into).collect())
}

// ---------- blogs ----------

#[derive(Debug, Deserialize)]
pub struct BlogQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
    /// Name, address, description, about text, or a category's name.
    #[serde(default)]
    q: Option<String>,
    /// Directory tag, e.g. `tech` (开往).
    #[serde(default)]
    dirtag: Option<String>,
    /// A top-level category among the blog's main ones.
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    /// Only these blogs, `1,2,3` (the ones a reader follows).
    #[serde(default)]
    ids: Option<String>,
    /// `recent` (default) | `posts` | `name`; with `q`, best matches come first.
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
    kind: String,
    categories: String,
    framable: Option<bool>,
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
    kind: KindRef,
    /// Main top-level categories, with their share of the blog's posts.
    categories: Vec<CategoryRef>,
    /// Whether its pages may be shown in a frame on this site (`null` = not known yet).
    framable: Option<bool>,
}

impl BlogSummary {
    fn from_row(st: &AppState, r: BlogSummaryRow) -> Self {
        BlogSummary {
            tags: tag_refs(st, &r.tags),
            kind: KindRef::new(&r.kind),
            categories: blog_categories(&r.categories),
            framable: r.framable,
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
        "SELECT b.id, b.name, b.homepage_url AS homepage, COALESCE(b.avatar_custom, b.avatar_url, b.icon_url) AS icon,
            COALESCE(b.description_custom, NULLIF(b.description, b.name)) AS description,
            (SELECT COUNT(*) FROM posts p WHERE p.blog_id = b.id) AS post_count,
            (SELECT MAX(p.sort_at) FROM posts p WHERE p.blog_id = b.id AND p.undated = 0) AS last_post_at,
            (b.last_success_at IS NOT NULL) AS has_feed,
            (SELECT json_group_array(json_object('directory', bt.directory, 'tag', bt.tag))
             FROM blog_tags bt WHERE bt.blog_id = b.id) AS tags,
            COALESCE(b.kind, b.detected_kind, 'blog') AS kind,
            b.categories, b.framable
         FROM blogs b"
    };
}

async fn list_blogs(State(st): State<AppState>, Query(q): Query<BlogQuery>) -> ApiResult<Page<BlogSummary>> {
    let limit = q.limit.unwrap_or(48).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let needle = non_empty(&q.q);
    let search = needle.map(like_pattern);
    let prefix = needle.map(|n| like_pattern(n).trim_start_matches('%').to_string());
    // "摄影" also finds blogs whose main category is photography, "AI" those
    // in its parent category.
    let categories: Option<String> = needle.map(|n| {
        let matches = |label: &str| label.contains(n) || n.contains(label);
        let ids: std::collections::BTreeSet<&str> = analyze::CATEGORIES
            .iter()
            .filter(|c| c.id != analyze::OTHER && matches(c.label))
            .map(|c| c.id)
            .chain(analyze::TOPICS.iter().filter(|t| matches(t.label)).map(|t| t.parent))
            .collect();
        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into())
    });
    let rows: Vec<BlogSummaryRow> = sqlx::query_as(concat!(
        blog_summary_sql!(),
        "
         WHERE ",
        visible!(),
        "
           AND (?1 IS NULL OR b.name LIKE ?1 ESCAPE '\\' OR b.homepage_url LIKE ?1 ESCAPE '\\'
                OR COALESCE(b.description_custom, b.description) LIKE ?1 ESCAPE '\\' OR b.about_text LIKE ?1 ESCAPE '\\'
                OR EXISTS (SELECT 1 FROM json_each(b.categories) jc
                           WHERE json_extract(jc.value, '$.id') IN (SELECT value FROM json_each(?8))))
           AND (?4 IS NULL OR EXISTS (SELECT 1 FROM blog_tags bt WHERE bt.blog_id = b.id AND bt.tag = ?4))
           AND (?9 IS NULL OR EXISTS (SELECT 1 FROM json_each(b.categories) jc
                                      WHERE json_extract(jc.value, '$.id') = ?9))
           AND (?10 IS NULL OR COALESCE(b.kind, b.detected_kind, 'blog') = ?10)
           AND (?11 IS NULL OR b.id IN (SELECT value FROM json_each(?11)))
         ORDER BY
           CASE WHEN ?1 IS NULL THEN 0
                WHEN lower(b.name) = lower(?6) THEN 0
                WHEN b.name LIKE ?7 ESCAPE '\\' THEN 1
                WHEN b.name LIKE ?1 ESCAPE '\\' THEN 2
                WHEN b.homepage_url LIKE ?1 ESCAPE '\\' THEN 3
                WHEN COALESCE(b.description_custom, b.description) LIKE ?1 ESCAPE '\\' THEN 4
                ELSE 5 END,
           CASE WHEN ?5 = 'name' THEN b.name END COLLATE NOCASE,
           CASE WHEN ?5 = 'posts' THEN post_count END DESC,
           last_post_at IS NULL, last_post_at DESC, b.id
         LIMIT ?2 OFFSET ?3"
    ))
    .bind(search)
    .bind(limit)
    .bind(offset)
    .bind(non_empty(&q.dirtag))
    .bind(non_empty(&q.sort).unwrap_or("recent"))
    .bind(needle)
    .bind(prefix)
    .bind(categories.unwrap_or_else(|| "[]".into()))
    .bind(non_empty(&q.category).map(analyze::top_of))
    .bind(non_empty(&q.kind))
    .bind(id_list(&q.ids, 500))
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
    /// Tags the blog uses most (own and derived), for its profile.
    top_tags: Vec<String>,
    /// The whole about page (sanitised HTML), when the author asked for it.
    about_html: Option<String>,
    /// The author allowed their comment widget here.
    comments_embedded: bool,
    /// How crawling this blog goes, for its author to check.
    crawl: CrawlStatus,
}

/// What the crawler knows about a blog, and what the site's config sets.
#[derive(Debug, Serialize, FromRow)]
pub struct CrawlStatus {
    feed_url: Option<String>,
    /// `directory` (listed there), `discovered` (found on the site) or
    /// `manual` (set in the config).
    feed_origin: Option<String>,
    last_checked_at: Option<i64>,
    last_success_at: Option<i64>,
    next_check_at: i64,
    /// Failed checks in a row, and the last error.
    failures: i64,
    last_error: Option<String>,
    /// Last search for a feed, for blogs without one.
    discovery_at: Option<i64>,
    /// When the homepage and about page were last read.
    profile_at: Option<i64>,
    /// The picture found on the site, and the feed's own image.
    found_avatar: Option<String>,
    feed_image: Option<String>,
    /// The about page that is read: the configured one, else the one found.
    about_page: Option<String>,
    /// Which settings come from the site's config, e.g. `avatar`, `social`,
    /// `full_text`.
    #[sqlx(skip)]
    configured: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct BlogDirectory {
    directory: String,
    name: String,
}

async fn get_blog(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<BlogDetail> {
    let row: Option<BlogSummaryRow> = sqlx::query_as(concat!(
        blog_summary_sql!(),
        " WHERE b.id = ? AND b.excluded = 0 AND b.flagged IS NULL"
    ))
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let summary = BlogSummary::from_row(&st, row.ok_or_else(not_found)?);
    #[derive(FromRow)]
    struct Extra {
        full_text: bool,
        about_url: Option<String>,
        about_text: Option<String>,
        about_html: Option<String>,
        social_links: String,
        generator: Option<String>,
        comment_system: Option<String>,
        embed_comments: bool,
        tags_custom: Option<String>,
        configured: String,
    }
    // What the author set in the config wins over anything read from their page.
    let extra: Extra = sqlx::query_as(
        "SELECT content_policy = 'full_text' AS full_text, about_url,
                COALESCE(about_custom, about_text) AS about_text,
                CASE WHEN about_full = 1 AND about_custom IS NULL THEN about_html END AS about_html,
                COALESCE(social_custom, social_links) AS social_links, generator, comment_system, embed_comments,
                tags_custom,
                concat_ws(' ',
                    CASE WHEN avatar_custom IS NOT NULL THEN 'avatar' END,
                    CASE WHEN description_custom IS NOT NULL THEN 'description' END,
                    CASE WHEN social_custom IS NOT NULL THEN 'social' END,
                    CASE WHEN tags_custom IS NOT NULL THEN 'tags' END,
                    CASE WHEN about_page IS NOT NULL OR about_custom IS NOT NULL OR about_full = 1 THEN 'about' END,
                    CASE WHEN feed_url_origin = 'manual' THEN 'feed' END,
                    CASE WHEN content_policy = 'full_text' THEN 'full_text' END,
                    CASE WHEN embed_comments = 1 THEN 'comments' END) AS configured
         FROM blogs WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&st.pool)
    .await?;
    let mut crawl: CrawlStatus = sqlx::query_as(
        "SELECT feed_url, feed_url_origin AS feed_origin, last_checked_at, last_success_at, next_check_at,
                failures, last_error, discovery_at, profile_at, avatar_url AS found_avatar, icon_url AS feed_image,
                COALESCE(about_page, about_url) AS about_page
         FROM blogs WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&st.pool)
    .await?;
    crawl.configured = [
        "avatar",
        "description",
        "social",
        "tags",
        "about",
        "feed",
        "full_text",
        "comments",
    ]
    .into_iter()
    .filter(|k| extra.configured.split(' ').any(|c| c == *k))
    .collect();
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
    let found_tags: Vec<String> = sqlx::query_scalar(
        "SELECT COALESCE(l.label, pt.term)
         FROM post_terms pt JOIN posts p ON p.id = pt.post_id
         LEFT JOIN tag_labels l ON l.term = pt.term
         WHERE p.blog_id = ? AND pt.is_tag = 1
         GROUP BY pt.term
         ORDER BY COUNT(*) DESC, MAX(pt.weight) DESC
         LIMIT 8",
    )
    .bind(id)
    .fetch_all(&st.pool)
    .await?;
    let top_tags = match extra.tags_custom.as_deref() {
        Some(t) => serde_json::from_str(t).unwrap_or_default(),
        None => found_tags,
    };
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
        top_tags,
        about_html: extra.about_html,
        comments_embedded: extra.embed_comments,
        crawl,
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::feed::ParsedItem;
    use crate::sources::DirectoryEntry;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;
    use url::Url;

    /// Blogs 1–3; blog 3 is not alive. Tags: 1 = blog+tech, 2 = life, 3 = tech.
    /// Five posts each; even posts have a cover and the tag 旅行, odd ones 日记.
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
                kind: None,
                name_from_feed: false,
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
                        vec!["日记".into()]
                    },
                    published: chrono::DateTime::from_timestamp(now - 86400 * 10 + (blog * 10 + n) * 3600, 0),
                    body_html: Some("<p>secret full text</p>".into()),
                    comments_url: None,
                    comment_count: (blog == 1 && n == 4).then_some(9),
                };
                let mut conn = pool.acquire().await.unwrap();
                let e = crate::db::Entry {
                    position: n,
                    first_fetch: false,
                    keywords_json: "[]",
                    content: None,
                };
                crate::db::upsert_post(&mut conn, blog, &item, now, &e).await.unwrap();
            }
        }
        crate::analyze::retag_all(&pool).await.unwrap();
        AppState {
            pool,
            site_name: "test".into(),
            directories: Arc::new(vec![DirectoryInfo {
                id: "travellings",
                name: "开往",
                homepage: "x",
                tag_labels: &[("tech", "技术"), ("life", "生活")],
            }]),
            maintainer: Some(crate::config::Maintainer {
                name: "站长".into(),
                url: Some("https://b2.example/".into()),
            }),
            columns: 6,
            repo: Some("https://github.com/example/blogscroll".into()),
            repo_config: "deploy/config.actions.toml".into(),
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
            let (_, v) = get_json(&st, &format!("/api/posts?mode={mode}&dirtag=tech&limit=100")).await;
            let blogs = ids(&v, "/blog/id");
            assert_eq!(blogs.len(), 5, "{mode}: only blog 1 is alive and tagged tech");
            assert!(blogs.iter().all(|b| *b == 1));
        }
        let (_, v) = get_json(&st, "/api/posts?dirtag=life&directory=travellings&limit=100").await;
        assert!(ids(&v, "/blog/id").iter().all(|b| *b == 2));
        let (_, v) = get_json(&st, "/api/posts?directory=elsewhere&limit=100").await;
        assert!(v["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn filters_cover_and_tag() {
        let st = seeded().await;
        let (_, v) = get_json(&st, "/api/posts?cover=1&limit=100").await;
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        assert!(items.iter().all(|it| it["cover"].is_string()));
        let (_, v) = get_json(&st, "/api/posts?cover=none&limit=100").await;
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|it| it["cover"].is_null()), "text-only posts");

        // Excerpts of two characters are next to nothing; one real one.
        let (_, v) = get_json(&st, "/api/posts?text=1&limit=100").await;
        assert!(v["items"].as_array().unwrap().is_empty());
        sqlx::query("UPDATE posts SET excerpt = '这一篇的摘要足够长，写了好几句话，值得一读。' WHERE id = 2")
            .execute(&st.pool)
            .await
            .unwrap();
        let (_, v) = get_json(&st, "/api/posts?text=1&limit=100").await;
        assert_eq!(ids(&v, "/id"), [2]);

        let (_, v) = get_json(&st, "/api/posts?tag=%E6%97%A5%E8%AE%B0&limit=100").await;
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|it| it["tags"][0] == "日记"));
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
        let cats: Vec<&str> = v["popular_tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(cats.len(), 2);
        assert!(cats.contains(&"旅行") && cats.contains(&"日记"));
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
    async fn categories_kinds_and_followed_blogs_filter_posts() {
        let st = seeded().await;
        // Even posts are tagged 旅行 (travel), odd ones 日记 (life/daily).
        let (_, v) = get_json(&st, "/api/posts?category=travel&limit=100").await;
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        assert!(
            items
                .iter()
                .all(|it| it["category"]["id"] == "travel" && it["category"]["label"] == "旅行")
        );
        for mode in ["latest", "discover"] {
            let (_, v) = get_json(&st, &format!("/api/posts?mode={mode}&category=life&limit=100")).await;
            let items = v["items"].as_array().unwrap();
            assert_eq!(items.len(), 4, "{mode}: a category includes its sub-categories");
            assert!(items.iter().all(|it| it["category"]["id"] == "life/daily"
                && it["category"]["label"] == "随笔"
                && it["category"]["parent"]["label"] == "生活"));
        }
        let (_, v) = get_json(&st, "/api/posts?category=lif&limit=100").await;
        assert!(v["items"].as_array().unwrap().is_empty(), "no prefix matches");

        sqlx::query("UPDATE blogs SET kind = 'podcast' WHERE id = 2")
            .execute(&st.pool)
            .await
            .unwrap();
        let (_, v) = get_json(&st, "/api/posts?kind=podcast&limit=100").await;
        assert!(ids(&v, "/blog/id").iter().all(|b| *b == 2));
        assert_eq!(v["items"][0]["blog"]["kind"], "podcast");

        let (_, v) = get_json(&st, "/api/posts?blogs=2,999&limit=100").await;
        let blogs = ids(&v, "/blog/id");
        assert_eq!(blogs.len(), 5);
        assert!(
            blogs.iter().all(|b| *b == 2),
            "following feed is newest-first from those blogs"
        );

        let (_, t) = get_json(&st, "/api/tags").await;
        let travel = t["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "travel")
            .unwrap();
        assert_eq!(travel["posts"], 6);
        assert!(
            t["kinds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k["id"] == "podcast" && k["label"] == "播客")
        );
    }

    #[tokio::test]
    async fn tags_match_own_and_folded_tags_and_are_searchable() {
        let st = seeded().await;
        let (_, a) = get_json(&st, "/api/posts?tag=%E6%97%85%E8%A1%8C&limit=100").await;
        assert_eq!(a["items"].as_array().unwrap().len(), 6);
        let (_, s) = get_json(&st, "/api/posts?q=%E6%97%85%E8%A1%8C&limit=100").await;
        assert_eq!(s["items"].as_array().unwrap().len(), 6, "searching finds posts by tag");
        let (_, t) = get_json(&st, "/api/tags/search?q=%E6%97%85").await;
        assert_eq!(t[0]["name"], "旅行");
        assert_eq!(t[0]["posts"], 6);
        let (_, all) = get_json(&st, "/api/tags/search?limit=100").await;
        assert_eq!(all.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn related_posts_share_terms_and_skip_hidden_blogs() {
        let st = seeded().await;
        // Post 1 is blog 1's first post, tagged 旅行.
        let (status, v) = get_json(&st, "/api/posts/1/related?limit=10").await;
        assert_eq!(status, StatusCode::OK);
        let items = v.as_array().unwrap();
        assert!(!items.is_empty());
        assert!(items.iter().all(|it| it["id"] != 1));
        assert!(items.iter().all(|it| it["tags"][0] == "旅行"));
        assert!(
            items.iter().all(|it| it["blog"]["id"] != 3),
            "inactive blogs are not suggested"
        );
        let per_blog_1 = items.iter().filter(|it| it["blog"]["id"] == 1).count();
        assert!(per_blog_1 <= 2);
    }

    #[tokio::test]
    async fn discover_leans_towards_followed_interests() {
        let st = seeded().await;
        let (_, plain) = get_json(&st, "/api/posts?seed=1&limit=100").await;
        let (_, personal) = get_json(&st, "/api/posts?seed=1&limit=100&follow=2").await;
        assert_eq!(
            plain["items"].as_array().unwrap().len(),
            personal["items"].as_array().unwrap().len()
        );
        assert_ne!(ids(&plain, "/id"), ids(&personal, "/id"), "follows change the order");

        // Post 1 is a travel post: liking it brings travel up, and itself down.
        let (_, liked) = get_json(&st, "/api/posts?seed=1&limit=100&like=1").await;
        let items = liked["items"].as_array().unwrap();
        assert!(items[..3].iter().all(|it| it["category"]["id"] == "travel"));
        assert_eq!(items.last().unwrap()["id"], 1, "already seen goes last");

        // Shown before but passed over: further down, though not to the end.
        let first = ids(&plain, "/id")[0];
        let (_, again) = get_json(&st, &format!("/api/posts?seed=1&limit=100&shown={first}")).await;
        let again = ids(&again, "/id");
        let at = again.iter().position(|i| *i == first).unwrap();
        assert!(at > 0 && at < again.len() - 1, "{first} moved from the top to {at}");

        // Exploring turns it round: another blog, another category first.
        let (_, explore) = get_json(&st, "/api/posts?mode=explore&seed=1&limit=100&like=1").await;
        let items = explore["items"].as_array().unwrap();
        assert_eq!(items[0]["category"]["id"], "life/daily");
        assert_eq!(items[0]["blog"]["id"], 2, "a blog not read yet");
        assert_eq!(items.len(), 6, "three posts per blog when exploring");
    }

    #[tokio::test]
    async fn flagged_and_undated_posts() {
        let st = seeded().await;
        sqlx::query("UPDATE blogs SET flagged = 'spam' WHERE id = 2")
            .execute(&st.pool)
            .await
            .unwrap();
        let (_, v) = get_json(&st, "/api/posts?mode=latest&limit=100").await;
        assert!(ids(&v, "/blog/id").iter().all(|b| *b == 1), "spam sources are hidden");
        let (status, _) = get_json(&st, "/api/blogs/2").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // An undated backlog post sorts after everything dated.
        sqlx::query("UPDATE posts SET undated = 1, published_at = NULL, sort_at = -1 WHERE id = 1")
            .execute(&st.pool)
            .await
            .unwrap();
        let (_, v) = get_json(&st, "/api/posts?mode=latest&limit=100").await;
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.last().unwrap()["id"], 1);
        assert_eq!(items.last().unwrap()["undated"], true);
        let (_, d) = get_json(&st, "/api/posts?seed=3&limit=100").await;
        assert_eq!(d["items"].as_array().unwrap().last().unwrap()["id"], 1);
    }

    #[tokio::test]
    async fn blog_search_ranks_names_first_and_reads_descriptions() {
        let st = seeded().await;
        sqlx::query("UPDATE blogs SET description = '写写 Blog 2 的摄影' WHERE id = 1")
            .execute(&st.pool)
            .await
            .unwrap();
        let (_, v) = get_json(&st, "/api/blogs?q=Blog%202").await;
        assert_eq!(ids(&v, "/id"), [2, 1], "a name match outranks a description match");
        let (_, v) = get_json(&st, "/api/blogs?ids=1").await;
        assert_eq!(ids(&v, "/id"), [1]);
        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(b["kind"]["id"], "blog");
        assert_eq!(b["kind"]["label"], "博客");
        assert!(b["top_tags"].as_array().unwrap().iter().any(|t| t == "旅行"));
    }

    #[tokio::test]
    async fn profile_settings_replace_what_was_found_and_crawl_status_shows() {
        let st = seeded().await;
        let (_, m) = get_json(&st, "/api/meta").await;
        assert_eq!(m["repo"]["url"], "https://github.com/example/blogscroll");
        assert_eq!(m["repo"]["config_path"], "deploy/config.actions.toml");

        sqlx::query(
            "UPDATE blogs SET avatar_url = 'https://b1.example/found.png', description = '找到的简介',
                social_links = '[{\"kind\":\"github\",\"url\":\"https://github.com/found\"}]',
                feed_url = 'https://b1.example/feed', last_error = 'HTTP 500', failures = 2
             WHERE id = 1",
        )
        .execute(&st.pool)
        .await
        .unwrap();
        let mut cfg = crate::config::Config::default();
        cfg.policy.profile = vec![crate::config::ProfilePolicy {
            site: "https://b1.example/".into(),
            avatar: Some("https://img.example/me.png".into()),
            description: Some("  博主自己写的一句话 ".into()),
            social: Some(vec![
                "https://x.com/me".into(),
                "https://me.example/".into(),
                "not a url".into(),
            ]),
            tags: Some(vec!["Rust".into(), " 摄影 ".into()]),
            note: String::new(),
        }];
        crate::db::apply_policy(&st.pool, &cfg).await.unwrap();
        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(b["icon"], "https://img.example/me.png");
        assert_eq!(b["description"], "博主自己写的一句话");
        assert_eq!(
            b["social"],
            json!([{"kind": "x", "url": "https://x.com/me"}, {"kind": "link", "url": "https://me.example/"}])
        );
        assert_eq!(b["top_tags"], json!(["Rust", "摄影"]));
        let c = &b["crawl"];
        assert_eq!(c["configured"], json!(["avatar", "description", "social", "tags"]));
        assert_eq!(
            c["found_avatar"], "https://b1.example/found.png",
            "what was found stays"
        );
        assert_eq!(
            (c["feed_url"].clone(), c["failures"].clone()),
            (json!("https://b1.example/feed"), json!(2))
        );
        assert_eq!(c["last_error"], "HTTP 500");
        let (_, v) = get_json(&st, "/api/posts?blog=1&limit=1").await;
        assert_eq!(v["items"][0]["blog"]["icon"], "https://img.example/me.png", "cards too");

        // Taken out of the config: what was found is shown again.
        cfg.policy.profile.clear();
        crate::db::apply_policy(&st.pool, &cfg).await.unwrap();
        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(b["icon"], "https://b1.example/found.png");
        assert_eq!(b["description"], "找到的简介");
        assert_eq!(b["social"][0]["url"], "https://github.com/found");
        assert_eq!(b["crawl"]["configured"], json!([]));
    }

    #[tokio::test]
    async fn maintainer_and_about_settings() {
        let st = seeded().await;
        let (_, m) = get_json(&st, "/api/meta").await;
        assert_eq!(m["maintainer"]["name"], "站长");
        assert_eq!(
            m["maintainer"]["blog"]["id"], 2,
            "their blog here, found by its address"
        );

        sqlx::query("UPDATE blogs SET about_text = '摘要', about_html = '<p>全文</p>', profile_at = 1 WHERE id = 1")
            .execute(&st.pool)
            .await
            .unwrap();
        let about = |b: &serde_json::Value| (b["about_text"].clone(), b["about_html"].clone());
        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(
            about(&b),
            (json!("摘要"), json!(null)),
            "the whole page only when asked for"
        );

        let mut cfg = crate::config::Config::default();
        cfg.policy.about = vec![crate::config::AboutPolicy {
            site: "https://b1.example/".into(),
            url: Some("https://b1.example/me/".into()),
            full: true,
            text: None,
            note: String::new(),
        }];
        crate::db::apply_policy(&st.pool, &cfg).await.unwrap();
        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(about(&b), (json!("摘要"), json!("<p>全文</p>")));
        let (page, refresh): (Option<String>, Option<i64>) =
            sqlx::query_as("SELECT about_page, profile_at FROM blogs WHERE id = 1")
                .fetch_one(&st.pool)
                .await
                .unwrap();
        assert_eq!(page.as_deref(), Some("https://b1.example/me/"));
        assert_eq!(refresh, None, "a new about page is read soon");

        cfg.policy.about[0].text = Some("我自己写的介绍".into());
        crate::db::apply_policy(&st.pool, &cfg).await.unwrap();
        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(about(&b), (json!("我自己写的介绍"), json!(null)), "an own text wins");

        cfg.policy.about.clear();
        crate::db::apply_policy(&st.pool, &cfg).await.unwrap();
        let (_, b) = get_json(&st, "/api/blogs/1").await;
        assert_eq!(about(&b), (json!("摘要"), json!(null)), "withdrawn settings are undone");
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
        let (_, list) = get_json(&st, "/api/blogs?dirtag=life").await;
        assert_eq!(ids(&list, "/id"), [2]);
        let (_, list) = get_json(&st, "/api/blogs?category=life").await;
        assert_eq!(ids(&list, "/id"), [2, 1], "blogs by main category");
        assert_eq!(list["items"][0]["categories"][0]["id"], "travel");
        assert_eq!(list["items"][0]["categories"][0]["share"], 0.6);
        let (_, list) = get_json(&st, "/api/blogs?q=%E9%9A%8F%E7%AC%94").await;
        assert_eq!(
            ids(&list, "/id"),
            [2, 1],
            "a sub-category's name finds its parent's blogs"
        );
    }
}
