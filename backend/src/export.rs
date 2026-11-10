//! Static export: the JSON the API serves, written to files, so the site can be
//! hosted without a backend (GitHub Pages). The frontend's static mode reads:
//!
//! ```text
//! index.json        { generated_at, feed_posts, feed_shards, shard_size }
//! meta.json         = GET /api/meta
//! tags.json         = GET /api/tags
//! tag-index.json    every tag used more than once (= GET /api/tags/search), for search
//! feed-N.json       newest posts, newest first, `shard_size` per file (= latest mode)
//! related.json      { post id: [related post ids] }, limited to posts in the feed files
//! blogs.json        every blog (= GET /api/blogs, all pages)
//! blogs/{id}.json   { blog: GET /api/blogs/{id}, posts: [every stored post, newest first] }
//! posts/{id}.json   = GET /api/posts/{id}, only for blogs whose authors allowed full text
//!                   or their comment widget (otherwise a card says all there is)
//! details.json      [ids of the posts above]
//! ```
//!
//! With a site URL, the pages around it are filled in for search engines
//! (see `seo.rs`): index.html, blogs.html, about.html, blog/{key}/index.html
//! (and blog/{id}.html, which sends old links on),
//! 404.html (the app for every other route, not indexed), sitemap.xml and
//! robots.txt.
//!
//! Requests go through the API router in-process, so the files have exactly the
//! shapes (and the same privacy rules) as the live API.

use std::path::Path;

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::seo;

const RELATED_PER_POST: usize = 8;
const TAG_INDEX_SIZE: usize = 5000;

pub struct ExportOptions {
    /// How many of the newest posts go into the feed shards.
    pub feed_posts: usize,
    pub shard_size: usize,
    /// The site's public address (https://you.github.io/blogScroll): write
    /// pages for search engines next to the data directory.
    pub site_url: Option<String>,
    pub site_name: String,
}

#[derive(Debug)]
pub struct ExportStats {
    pub feed_posts: usize,
    pub feed_shards: usize,
    pub blogs: usize,
}

pub async fn export(app: Router, out: &Path, opts: &ExportOptions) -> anyhow::Result<ExportStats> {
    // Build next to the target and swap at the end, so a failed export never
    // leaves a half-written site behind.
    let tmp = out.with_file_name(format!(
        "{}.tmp",
        out.file_name().and_then(|n| n.to_str()).unwrap_or("data")
    ));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(tmp.join("blogs"))?;
    std::fs::create_dir_all(tmp.join("posts"))?;

    write(&tmp.join("meta.json"), &get(&app, "/api/meta").await?)?;
    write(&tmp.join("tags.json"), &get(&app, "/api/tags").await?)?;
    write(
        &tmp.join("tag-index.json"),
        &get(&app, &format!("/api/tags/search?limit={TAG_INDEX_SIZE}")).await?,
    )?;

    let posts = paged(&app, opts.feed_posts, |c| {
        format!(
            "/api/posts?mode=latest&nsfw=1&limit=100{}",
            c.map(|c| format!("&cursor={c}")).unwrap_or_default()
        )
    })
    .await?;
    let shard_size = opts.shard_size.max(1);
    let shards: Vec<&[Value]> = posts.chunks(shard_size).collect();
    for (i, shard) in shards.iter().enumerate() {
        write(&tmp.join(format!("feed-{i}.json")), &Value::from(shard.to_vec()))?;
    }

    // Related posts, resolved by the client from the feed files it loads.
    let exported: std::collections::HashSet<i64> = posts.iter().filter_map(|p| p["id"].as_i64()).collect();
    let mut related = serde_json::Map::new();
    for id in &exported {
        let list = get(&app, &format!("/api/posts/{id}/related?limit=16")).await?;
        let ids: Vec<i64> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["id"].as_i64())
            .filter(|i| exported.contains(i))
            .take(RELATED_PER_POST)
            .collect();
        if !ids.is_empty() {
            related.insert(id.to_string(), json!(ids));
        }
    }
    write(&tmp.join("related.json"), &Value::Object(related))?;

    let blogs = paged(&app, usize::MAX, |c| {
        format!("/api/blogs?limit=200&offset={}", c.unwrap_or("0"))
    })
    .await?;
    write(&tmp.join("blogs.json"), &Value::from(blogs.clone()))?;
    let mut blog_pages = Vec::new();
    let mut details: Vec<i64> = Vec::new();
    for b in &blogs {
        let id = b["id"].as_i64().context("blog without id")?;
        let blog = get(&app, &format!("/api/blogs/{id}")).await?;
        let posts = paged(&app, usize::MAX, |c| {
            format!(
                "/api/posts?mode=latest&nsfw=1&blog={id}&limit=100{}",
                c.map(|c| format!("&cursor={c}")).unwrap_or_default()
            )
        })
        .await?;
        write(
            &tmp.join(format!("blogs/{id}.json")),
            &json!({ "blog": blog, "posts": posts }),
        )?;
        if blog["full_text"] == true || blog["comments_embedded"] == true {
            for pid in posts.iter().filter_map(|p| p["id"].as_i64()) {
                let detail = get(&app, &format!("/api/posts/{pid}")).await?;
                write(&tmp.join(format!("posts/{pid}.json")), &detail)?;
                details.push(pid);
            }
        }
        if opts.site_url.is_some() {
            blog_pages.push((blog, posts));
        }
    }

    write(&tmp.join("details.json"), &json!(details))?;

    let stats = ExportStats {
        feed_posts: posts.len(),
        feed_shards: shards.len(),
        blogs: blogs.len(),
    };
    write(
        &tmp.join("index.json"),
        &json!({
            "generated_at": crate::util::now(),
            "feed_posts": stats.feed_posts,
            "feed_shards": stats.feed_shards,
            "shard_size": shard_size,
        }),
    )?;

    if out.exists() {
        std::fs::remove_dir_all(out)?;
    }
    std::fs::rename(&tmp, out)?;

    if let Some(url) = opts.site_url.as_deref() {
        let site_dir = out.parent().context("the data directory has no parent")?;
        let meta = get(&app, "/api/meta").await?;
        write_pages(site_dir, url, &opts.site_name, &meta, &posts, &blogs, &blog_pages)?;
    }
    Ok(stats)
}

/// The pages for search engines; skipped (with a warning) when the site
/// directory holds no built app to use as the template.
fn write_pages(
    dir: &Path,
    url: &str,
    name: &str,
    meta: &Value,
    posts: &[Value],
    blogs: &[Value],
    blog_pages: &[(Value, Vec<Value>)],
) -> anyhow::Result<()> {
    // A filled-in index.html from an earlier export is not a template; the
    // untouched app was kept as 404.html then.
    let index = std::fs::read_to_string(dir.join("index.html")).unwrap_or_default();
    let template = if index.contains(seo::MARKER) {
        std::fs::read_to_string(dir.join("404.html")).unwrap_or_default()
    } else {
        index
    };
    let site = seo::Site {
        name,
        url: url.trim_end_matches('/'),
        template: &template,
    };
    let page = |p: seo::Page| -> anyhow::Result<()> {
        let Some(html) = seo::render(&site, &p) else {
            anyhow::bail!("no built app (index.html with <div id=\"app\">) in {}", dir.display());
        };
        let file = if p.path.is_empty() || p.path.ends_with('/') {
            dir.join(&p.path).join("index.html")
        } else {
            dir.join(format!("{}.html", p.path))
        };
        std::fs::create_dir_all(file.parent().unwrap_or(dir))?;
        std::fs::write(&file, html).with_context(|| format!("writing {}", file.display()))
    };
    if let Err(e) = page(seo::about(&site, meta)) {
        tracing::warn!(error = %e, "pages for search engines skipped");
        return Ok(());
    }
    std::fs::write(dir.join("404.html"), seo::not_found(&site))?;
    // Adult content stays out of what search engines see.
    let shown = |ps: &[Value]| ps.iter().filter(|p| p["nsfw"] != true).cloned().collect::<Vec<_>>();
    page(seo::home(&site, meta, &shown(posts)))?;
    page(seo::blogs(&site, blogs))?;
    let blog_dir = dir.join("blog");
    if blog_dir.exists() {
        std::fs::remove_dir_all(&blog_dir)?;
    }
    for (blog, posts) in blog_pages {
        let p = seo::blog(&site, blog, &shown(posts));
        // Links from before pages were named after the blog's address.
        if p.path.ends_with('/') {
            std::fs::create_dir_all(&blog_dir)?;
            std::fs::write(
                blog_dir.join(format!("{}.html", blog["id"])),
                seo::moved(&site, &p.path),
            )?;
        }
        page(p)?;
    }
    let mut map: Vec<(String, Option<i64>)> = vec![
        (String::new(), posts.first().and_then(|p| p["sort_at"].as_i64())),
        ("blogs".into(), None),
        ("about".into(), None),
    ];
    map.extend(blogs.iter().map(|b| (seo::blog_path(b), b["last_post_at"].as_i64())));
    std::fs::write(dir.join("sitemap.xml"), seo::sitemap(&site, &map))?;
    std::fs::write(dir.join("robots.txt"), seo::robots(&site))?;
    Ok(())
}

async fn get(app: &Router, uri: &str) -> anyhow::Result<Value> {
    let resp = app.clone().oneshot(Request::get(uri).body(Body::empty())?).await?;
    anyhow::ensure!(resp.status().is_success(), "{uri}: HTTP {}", resp.status());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Follows `next_cursor` until `max` items or the end.
async fn paged(app: &Router, max: usize, uri: impl Fn(Option<&str>) -> String) -> anyhow::Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = get(app, &uri(cursor.as_deref())).await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        cursor = page["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() || items.len() >= max {
            break;
        }
    }
    items.truncate(max);
    Ok(items)
}

fn write(path: &Path, value: &Value) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_vec(value)?).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_a_complete_static_site() {
        let state = crate::api::tests::seeded().await;
        // Blog 1's author allowed full text: its posts' details are exported too.
        sqlx::query("UPDATE blogs SET content_policy = 'full_text' WHERE id = 1")
            .execute(&state.pool)
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!("feed-export-test-{}", std::process::id()));
        let out = dir.join("data");
        std::fs::create_dir_all(out.join("stale")).unwrap();
        // The built app, as vite leaves it next to the data directory.
        std::fs::write(
            dir.join("index.html"),
            "<html><head><title>x</title></head><body><div id=\"app\"></div></body></html>",
        )
        .unwrap();
        let opts = ExportOptions {
            feed_posts: 7,
            shard_size: 3,
            site_url: Some("https://you.github.io/blogScroll/".into()),
            site_name: "博客瀑布".into(),
        };
        let stats = export(crate::api::router(state), &out, &opts).await.unwrap();
        assert_eq!((stats.feed_posts, stats.feed_shards, stats.blogs), (7, 3, 2));

        let read = |p: &str| -> Value { serde_json::from_slice(&std::fs::read(out.join(p)).unwrap()).unwrap() };
        assert_eq!(read("index.json")["feed_shards"], 3);
        assert_eq!(read("feed-0.json").as_array().unwrap().len(), 3);
        assert_eq!(read("feed-2.json").as_array().unwrap().len(), 1);
        assert_eq!(read("blogs.json").as_array().unwrap().len(), 2);
        assert_eq!(read("blogs/1.json")["posts"].as_array().unwrap().len(), 5);
        assert_eq!(read("blogs/1.json")["blog"]["name"], "Blog 1");
        assert!(read("tags.json")["directories"].is_array());
        assert!(!read("tag-index.json").as_array().unwrap().is_empty());
        let related = read("related.json");
        let first = related
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert!(!first.is_empty() && first.len() <= RELATED_PER_POST);
        assert!(!out.join("blogs/3.json").exists(), "inactive blogs are not exported");
        assert_eq!(read("details.json"), json!([5, 4, 3, 2, 1]));
        assert_eq!(read("posts/1.json")["id"], 1);
        assert!(!out.join("posts/6.json").exists(), "only authorised blogs");
        assert!(!out.join("stale").exists(), "old output is replaced");

        // Pages for search engines, next to the data.
        let page = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
        assert!(page("index.html").contains(seo::MARKER));
        // Named after the blog's address; the old numbered page sends readers on.
        let blog1 = page("blog/b1.example/index.html");
        assert!(blog1.contains("<link rel=\"canonical\" href=\"https://you.github.io/blogScroll/blog/b1.example/\""));
        assert!(blog1.contains("https://b1.example/p/4"), "posts link to the originals");
        let old = page("blog/1.html");
        assert!(old.contains("url=https://you.github.io/blogScroll/blog/b1.example/") && old.contains("noindex"));
        assert!(!dir.join("blog/3.html").exists() && !dir.join("blog/b3.example").exists());
        assert!(page("sitemap.xml").contains("<loc>https://you.github.io/blogScroll/blog/b1.example/</loc>"));
        assert!(page("404.html").contains("noindex"));
        assert!(page("sitemap.xml").contains("<loc>https://you.github.io/blogScroll/blogs</loc>"));
        // A second export uses the untouched app, not the filled-in index.html.
        export(crate::api::router(crate::api::tests::seeded().await), &out, &opts)
            .await
            .unwrap();
        assert_eq!(page("index.html").matches(seo::MARKER).count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
