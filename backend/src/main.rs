mod analyze;
mod api;
mod cache;
mod clean;
mod comments;
mod config;
mod crawler;
mod db;
mod discovery;
mod export;
mod feed;
mod fulltext;
mod html;
mod http;
mod profile;
mod sources;
mod util;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::middleware::{from_fn, from_fn_with_state};
use clap::{Parser, Subcommand};
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::api::{AppState, DirectoryInfo};
use crate::cache::ResponseCache;
use crate::config::Config;
use crate::crawler::Crawler;

#[derive(Parser)]
#[command(about = "RSS waterfall for blog webrings")]
struct Cli {
    #[arg(short, long, env = "FEED_CONFIG", default_value = "config.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the API and frontend, and crawl in the background (default).
    Serve {
        /// Only serve; do not crawl.
        #[arg(long)]
        no_crawl: bool,
    },
    /// Fetch the blog directories once and apply [policy].
    Sync,
    /// Crawl due feeds once.
    Crawl {
        /// Maximum number of blogs to crawl (per round with --all).
        #[arg(long, default_value_t = 50)]
        limit: i64,
        /// Crawl just this blog id, due or not.
        #[arg(long)]
        blog: Option<i64>,
        /// Sync the directories first if a day has passed (for scheduled runs).
        #[arg(long)]
        sync: bool,
        /// Keep going until nothing is due, or until --max-minutes.
        #[arg(long)]
        all: bool,
        /// Time budget for --all; the round in progress is finished, then it stops.
        #[arg(long, default_value_t = 90)]
        max_minutes: u64,
    },
    /// Refresh blog profiles and comment counts that are due (normally done
    /// by `serve` in the background).
    Enrich {
        /// Blogs per round.
        #[arg(long, default_value_t = 200)]
        limit: i64,
        /// Keep going until nothing is due.
        #[arg(long)]
        all: bool,
    },
    /// Recompute keywords, tags, topics and related-post vectors for every post
    /// (normally done by `serve` every few hours, and before `export`).
    Analyze,
    /// Write the site's data as static JSON (for GitHub Pages and similar).
    Export {
        /// Output directory, e.g. ../frontend/dist/data
        #[arg(long)]
        out: PathBuf,
        /// Newest posts included in the feed.
        #[arg(long, default_value_t = 3000)]
        feed_posts: usize,
        #[arg(long, default_value_t = 500)]
        shard_size: usize,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // html5ever warns about every odd page it parses, and the TLS verifier
        // logs each bad certificate as an error; both are per-site problems
        // that the crawler records on the blog (last_error) anyway.
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "info,sqlx=warn,tower_http=info,html5ever=error,rustls_platform_verifier=off".into()
            }),
        )
        .init();

    let cli = Cli::parse();
    let cfg = Arc::new(Config::load(&cli.config)?);
    let pool = db::connect(&cfg.database.path).await.context("opening database")?;
    db::apply_policy(&pool, &cfg).await?;

    let directories = sources::enabled(&cfg);
    let dir_info: Vec<DirectoryInfo> = directories
        .iter()
        .map(|d| DirectoryInfo {
            id: d.id(),
            name: d.name(),
            homepage: d.homepage(),
            tag_labels: d.tag_labels(),
        })
        .collect();
    let crawler = Arc::new(Crawler::new(
        pool.clone(),
        cfg.clone(),
        directories,
        Box::new(fulltext::ArticleExtractor),
    )?);

    crawler.sync_local_directories().await?;

    match cli.command.unwrap_or(Command::Serve { no_crawl: false }) {
        Command::Sync => crawler.sync_directories().await?,
        Command::Crawl {
            limit,
            blog,
            sync,
            all,
            max_minutes,
        } => {
            if sync {
                crawler.sync_directories_if_due().await?;
            }
            let stats = match blog {
                Some(id) => {
                    let blog = db::blog_by_id(&pool, id).await?.context("no such blog")?;
                    crawler.crawl_blogs(vec![blog]).await
                }
                None if all => {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(max_minutes * 60);
                    crawler.crawl_until_done(limit, deadline).await?
                }
                None => crawler.crawl_due(limit).await?,
            };
            println!("{stats:#?}");
        }
        Command::Enrich { limit, all } => {
            let mut total = 0;
            loop {
                let n = crawler.enrich_due(limit).await?;
                total += n;
                if n == 0 || !all {
                    break;
                }
            }
            println!("refreshed {total} blogs");
        }
        Command::Analyze => {
            let s = analyze::retag_all(&pool).await?;
            println!("{s:#?}");
        }
        Command::Export {
            out,
            feed_posts,
            shard_size,
        } => {
            let s = analyze::retag_all(&pool).await?;
            println!("analyzed {} posts", s.posts);
            let state = AppState {
                pool: pool.clone(),
                site_name: cfg.server.site_name.clone(),
                directories: Arc::new(dir_info),
            };
            let opts = export::ExportOptions { feed_posts, shard_size };
            let s = export::export(api::router(state), &out, &opts).await?;
            println!(
                "exported {} posts in {} shards and {} blogs to {}",
                s.feed_posts,
                s.feed_shards,
                s.blogs,
                out.display()
            );
        }
        Command::Serve { no_crawl } => {
            if no_crawl {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = analyze::retag_all(&pool).await {
                        tracing::warn!(error = %e, "retag failed");
                    }
                });
            } else {
                tokio::spawn(crawler.clone().run());
            }
            let state = AppState {
                pool: pool.clone(),
                site_name: cfg.server.site_name.clone(),
                directories: Arc::new(dir_info),
            };
            let cache = Arc::new(ResponseCache::new(cfg.server.api_cache_entries));
            let mut app: Router = api::router(state).route_layer(from_fn_with_state(cache, cache::api_cache));
            if let Some(dir) = cfg.server.static_dir.as_ref().filter(|d| d.join("index.html").exists()) {
                let spa = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
                app = app.fallback_service(ServiceBuilder::new().layer(from_fn(cache::static_headers)).service(spa));
            } else {
                tracing::warn!("frontend not built; serving the API only");
            }
            let app = app.layer(CompressionLayer::new()).layer(TraceLayer::new_for_http());

            let listener = tokio::net::TcpListener::bind(&cfg.server.listen).await?;
            tracing::info!("listening on http://{}", listener.local_addr()?);
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
    }
    // Closing cleanly checkpoints the WAL, leaving a single self-contained file
    // (matters when the database is carried between CI runs).
    pool.close().await;
    Ok(())
}
