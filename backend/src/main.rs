mod api;
mod config;
mod crawler;
mod db;
mod discovery;
mod feed;
mod fulltext;
mod html;
mod http;
mod sources;
mod util;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use clap::{Parser, Subcommand};
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::api::{AppState, DirectoryInfo};
use crate::config::Config;
use crate::crawler::Crawler;

#[derive(Parser)]
#[command(about = "RSS waterfall for blog webrings")]
struct Cli {
    #[arg(short, long, default_value = "config.toml")]
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
        /// Maximum number of blogs to crawl.
        #[arg(long, default_value_t = 50)]
        limit: i64,
        /// Crawl just this blog id, due or not.
        #[arg(long)]
        blog: Option<i64>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn,tower_http=info".into()))
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
        })
        .collect();
    let crawler = Arc::new(Crawler::new(
        pool.clone(),
        cfg.clone(),
        directories,
        Box::new(fulltext::ArticleExtractor),
    )?);

    match cli.command.unwrap_or(Command::Serve { no_crawl: false }) {
        Command::Sync => crawler.sync_directories().await?,
        Command::Crawl { limit, blog } => {
            let stats = match blog {
                Some(id) => {
                    let blog = db::blog_by_id(&pool, id).await?.context("no such blog")?;
                    crawler.crawl_blogs(vec![blog]).await
                }
                None => crawler.crawl_due(limit).await?,
            };
            println!("{stats:#?}");
        }
        Command::Serve { no_crawl } => {
            if !no_crawl {
                tokio::spawn(crawler.clone().run());
            }
            let state = AppState {
                pool,
                site_name: cfg.server.site_name.clone(),
                directories: Arc::new(dir_info),
            };
            let mut app: Router = api::router(state);
            if let Some(dir) = cfg.server.static_dir.as_ref().filter(|d| d.join("index.html").exists()) {
                let spa = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
                app = app.fallback_service(spa);
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
    Ok(())
}
