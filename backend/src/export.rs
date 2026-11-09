//! Static export: the JSON the API serves, written to files, so the site can be
//! hosted without a backend (GitHub Pages). The frontend's static mode reads:
//!
//! ```text
//! index.json        { generated_at, feed_posts, feed_shards, shard_size }
//! meta.json         = GET /api/meta
//! tags.json         = GET /api/tags
//! feed-N.json       newest posts, newest first, `shard_size` per file (= latest mode)
//! blogs.json        every blog (= GET /api/blogs, all pages)
//! blogs/{id}.json   { blog: GET /api/blogs/{id}, posts: [...] }
//! ```
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

pub struct ExportOptions {
    /// How many of the newest posts go into the feed shards.
    pub feed_posts: usize,
    pub shard_size: usize,
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

    write(&tmp.join("meta.json"), &get(&app, "/api/meta").await?)?;
    write(&tmp.join("tags.json"), &get(&app, "/api/tags").await?)?;

    let posts = paged(&app, opts.feed_posts, |c| {
        format!(
            "/api/posts?mode=latest&limit=100{}",
            c.map(|c| format!("&cursor={c}")).unwrap_or_default()
        )
    })
    .await?;
    let shard_size = opts.shard_size.max(1);
    let shards: Vec<&[Value]> = posts.chunks(shard_size).collect();
    for (i, shard) in shards.iter().enumerate() {
        write(&tmp.join(format!("feed-{i}.json")), &Value::from(shard.to_vec()))?;
    }

    let blogs = paged(&app, usize::MAX, |c| {
        format!("/api/blogs?limit=200&offset={}", c.unwrap_or("0"))
    })
    .await?;
    write(&tmp.join("blogs.json"), &Value::from(blogs.clone()))?;
    for b in &blogs {
        let id = b["id"].as_i64().context("blog without id")?;
        let blog = get(&app, &format!("/api/blogs/{id}")).await?;
        let page = get(&app, &format!("/api/posts?mode=latest&blog={id}&limit=100")).await?;
        write(
            &tmp.join(format!("blogs/{id}.json")),
            &json!({ "blog": blog, "posts": page["items"] }),
        )?;
    }

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
    Ok(stats)
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
        let dir = std::env::temp_dir().join(format!("feed-export-test-{}", std::process::id()));
        let out = dir.join("data");
        std::fs::create_dir_all(out.join("stale")).unwrap();
        let opts = ExportOptions {
            feed_posts: 7,
            shard_size: 3,
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
        assert!(!out.join("blogs/3.json").exists(), "inactive blogs are not exported");
        assert!(!out.join("stale").exists(), "old output is replaced");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
