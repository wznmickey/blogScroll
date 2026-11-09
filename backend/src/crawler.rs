use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::StatusCode;
use sqlx::SqlitePool;
use url::Url;

use crate::config::{Config, CrawlerConfig};
use crate::db::{self, BlogRow, Success};
use crate::discovery;
use crate::feed;
use crate::fulltext::FullTextExtractor;
use crate::html;
use crate::http::{ACCEPT_FEED, FetchError, Fetched, PoliteClient, Validators};
use crate::sources::Directory;
use crate::util::{jitter, now};

const LAST_SYNC_KEY: &str = "last_directory_sync";
/// A feed whose text is at least this long is treated as full text.
const FULL_FEED_MIN_CHARS: usize = 500;
/// Article pages fetched per blog per crawl, for full-text blogs only.
const PAGES_PER_CRAWL: i64 = 3;
const MAX_BACKOFF_SECS: i64 = 7 * 86400;

pub struct Crawler {
    pool: SqlitePool,
    cfg: Arc<Config>,
    http: PoliteClient,
    directories: Vec<Box<dyn Directory>>,
    extractor: Box<dyn FullTextExtractor>,
}

#[derive(Debug)]
pub enum Outcome {
    Updated { new: usize },
    NotModified,
    NoFeed,
    Waiting,
    Failed(String),
}

#[derive(Debug, Default)]
pub struct CrawlStats {
    pub checked: usize,
    pub updated: usize,
    pub new_posts: usize,
    pub not_modified: usize,
    pub no_feed: usize,
    pub failed: usize,
}

impl Crawler {
    pub fn new(
        pool: SqlitePool,
        cfg: Arc<Config>,
        directories: Vec<Box<dyn Directory>>,
        extractor: Box<dyn FullTextExtractor>,
    ) -> anyhow::Result<Self> {
        let http = PoliteClient::new(&cfg.crawler)?;
        Ok(Self {
            pool,
            cfg,
            http,
            directories,
            extractor,
        })
    }

    fn c(&self) -> &CrawlerConfig {
        &self.cfg.crawler
    }

    /// Background loop: daily directory sync plus feed refreshes as they fall due.
    pub async fn run(self: Arc<Self>) {
        loop {
            if let Err(e) = self.sync_directories_if_due().await {
                tracing::warn!(error = %e, "directory sync failed");
            }
            match self.crawl_due(1000).await {
                Ok(s) if s.checked > 0 => tracing::info!(?s, "crawl pass finished"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "crawl pass failed"),
            }
            tokio::time::sleep(Duration::from_secs(self.c().tick_secs.max(5))).await;
        }
    }

    async fn sync_directories_if_due(&self) -> anyhow::Result<()> {
        let last: i64 = db::kv_get(&self.pool, LAST_SYNC_KEY)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if now() - last >= self.c().directory_sync_secs as i64 {
            self.sync_directories().await?;
        }
        Ok(())
    }

    pub async fn sync_directories(&self) -> anyhow::Result<()> {
        let mut any_ok = false;
        for dir in &self.directories {
            match dir.fetch(&self.http).await {
                Ok(entries) => {
                    let alive = entries.iter().filter(|e| e.alive).count();
                    let saved =
                        db::save_directory(&self.pool, dir.id(), &entries, self.c().refresh_secs as i64, now()).await?;
                    tracing::info!(directory = dir.id(), saved, alive, "directory synced");
                    any_ok = true;
                }
                Err(e) => tracing::warn!(directory = dir.id(), error = %e, "directory fetch failed"),
            }
        }
        db::apply_policy(&self.pool, &self.cfg).await?;
        if any_ok {
            db::kv_set(&self.pool, LAST_SYNC_KEY, &now().to_string()).await?;
        }
        Ok(())
    }

    /// Crawls every blog that is due, at most `limit`. Actual request concurrency
    /// is capped inside `PoliteClient`; more tasks than permits just lets one
    /// blog wait for its per-host slot while others proceed.
    pub async fn crawl_due(&self, limit: i64) -> anyhow::Result<CrawlStats> {
        let due = db::due_blogs(&self.pool, now(), limit).await?;
        Ok(self.crawl_blogs(due).await)
    }

    pub async fn crawl_blogs(&self, blogs: Vec<BlogRow>) -> CrawlStats {
        let tasks = self.c().max_concurrency.max(1) * 4;
        let outcomes: Vec<Outcome> = futures::stream::iter(blogs)
            .map(|b| self.crawl_blog(b))
            .buffer_unordered(tasks)
            .collect()
            .await;
        let mut s = CrawlStats::default();
        for o in outcomes {
            s.checked += 1;
            match o {
                Outcome::Updated { new, .. } => {
                    s.updated += 1;
                    s.new_posts += new;
                }
                Outcome::NotModified => s.not_modified += 1,
                Outcome::NoFeed | Outcome::Waiting => s.no_feed += 1,
                Outcome::Failed(_) => s.failed += 1,
            }
        }
        s
    }

    pub async fn crawl_blog(&self, blog: BlogRow) -> Outcome {
        let outcome = match self.try_crawl_blog(&blog).await {
            Ok(o) => o,
            Err(e) => {
                let msg = format!("{e:#}");
                let next = now() + self.backoff(&blog, None);
                if let Err(db_err) = db::record_failure(&self.pool, blog.id, now(), &msg, next, false).await {
                    tracing::error!(error = %db_err, "could not record failure");
                }
                Outcome::Failed(msg)
            }
        };
        if let Outcome::Failed(msg) = &outcome {
            tracing::debug!(blog = %blog.homepage_url, %msg, "crawl failed");
        }
        outcome
    }

    async fn try_crawl_blog(&self, blog: &BlogRow) -> anyhow::Result<Outcome> {
        let t = now();
        let retry = self.c().discovery_retry_secs as i64;

        let (feed_url, prefetched) = match &blog.feed_url {
            Some(u) => (Url::parse(u)?, None),
            None => {
                if let Some(at) = blog.discovery_at.filter(|at| t - at < retry) {
                    db::record_no_feed(&self.pool, blog.id, at, at + retry).await?;
                    return Ok(Outcome::Waiting);
                }
                let homepage = Url::parse(&blog.homepage_url)?;
                match discovery::discover(&self.http, &homepage).await {
                    Ok(Some(found)) => {
                        db::set_discovered_feed(&self.pool, blog.id, found.feed_url.as_str(), t).await?;
                        tracing::info!(blog = %blog.name, feed = %found.feed_url, "feed discovered");
                        let fetched = Fetched::Ok {
                            final_url: found.feed_url.clone(),
                            validators: found.validators,
                            body: found.body,
                        };
                        (found.feed_url, Some(fetched))
                    }
                    Ok(None) => {
                        db::record_no_feed(&self.pool, blog.id, t, t + jitter(retry, 10, blog.id)).await?;
                        return Ok(Outcome::NoFeed);
                    }
                    Err(e) => return self.fail(blog, e, false).await,
                }
            }
        };

        let fetched = match prefetched {
            Some(f) => f,
            None => {
                let validators = Validators {
                    etag: blog.etag.clone(),
                    last_modified: blog.last_modified.clone(),
                };
                match self.http.get(&feed_url, ACCEPT_FEED, Some(&validators)).await {
                    Ok(f) => f,
                    Err(e) => {
                        // A vanished feed URL is re-discovered, unless it was set by hand.
                        let gone = matches!(e.status(), Some(StatusCode::NOT_FOUND | StatusCode::GONE));
                        let forget = gone && blog.feed_url_origin.as_deref() != Some("manual");
                        return self.fail(blog, e, forget).await;
                    }
                }
            }
        };

        match fetched {
            Fetched::NotModified => {
                let refresh = self.slower(blog.refresh_secs);
                db::record_success(
                    &self.pool,
                    blog.id,
                    t,
                    Success {
                        validators: None,
                        refresh_secs: refresh,
                        next_check_at: t + jitter(refresh, 10, blog.id),
                        had_new_posts: false,
                        icon_url: None,
                    },
                )
                .await?;
                Ok(Outcome::NotModified)
            }
            Fetched::Ok { body, validators, .. } => self.ingest(blog, &feed_url, &body, &validators).await,
        }
    }

    async fn ingest(
        &self,
        blog: &BlogRow,
        feed_url: &Url,
        body: &[u8],
        validators: &Validators,
    ) -> anyhow::Result<Outcome> {
        let t = now();
        let parsed = match feed::parse(body, feed_url) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("feed parse error: {e}");
                let forget = blog.feed_url_origin.as_deref() == Some("discovered") && blog.failures >= 2;
                db::record_failure(&self.pool, blog.id, t, &msg, t + self.backoff(blog, None), forget).await?;
                return Ok(Outcome::Failed(msg));
            }
        };

        let mut items = parsed.items;
        items.sort_by_key(|i| std::cmp::Reverse(i.published));
        let max_age = self.c().max_post_age_days as i64 * 86400;
        if max_age > 0 {
            items.retain(|i| i.published.is_none_or(|p| p.timestamp() >= t - max_age));
        }
        items.truncate(self.c().max_items_per_feed);

        let full_text = blog.full_text_allowed();
        let mut new = 0;
        for (idx, item) in items.iter().enumerate() {
            let content = if full_text { feed_full_text(item) } else { None };
            let content_ref = content.as_deref().map(|c| (c, "feed"));
            if db::upsert_post(&self.pool, blog.id, item, t, t - idx as i64, content_ref).await? {
                new += 1;
            }
        }
        if full_text {
            self.fetch_full_text(blog).await?;
        }

        let mut refresh = if new > 0 {
            self.c().refresh_secs as i64
        } else {
            self.slower(blog.refresh_secs)
        };
        if let Some(ttl) = parsed.ttl_minutes {
            refresh = refresh.max(ttl as i64 * 60).min(self.c().max_refresh_secs as i64);
        }
        let icon = parsed.icon.as_ref().map(|u| u.to_string());
        db::record_success(
            &self.pool,
            blog.id,
            t,
            Success {
                validators: Some(validators),
                refresh_secs: refresh,
                next_check_at: t + jitter(refresh, 10, blog.id),
                had_new_posts: new > 0,
                icon_url: icon.as_deref(),
            },
        )
        .await?;
        Ok(Outcome::Updated { new })
    }

    /// Only reached for blogs whose authors allowed full-text fetching.
    async fn fetch_full_text(&self, blog: &BlogRow) -> anyhow::Result<()> {
        for (post_id, url) in db::posts_missing_content(&self.pool, blog.id, PAGES_PER_CRAWL).await? {
            let Ok(url) = Url::parse(&url) else { continue };
            match self.extractor.extract(&self.http, &url).await {
                Ok(content) => db::set_post_content(&self.pool, post_id, content.as_deref()).await?,
                Err(e) => tracing::debug!(%url, error = %e, "full-text fetch failed, will retry"),
            }
        }
        Ok(())
    }

    async fn fail(&self, blog: &BlogRow, e: FetchError, forget_feed: bool) -> anyhow::Result<Outcome> {
        let retry_after = match &e {
            FetchError::Status { retry_after, .. } => retry_after.map(|d| d.as_secs() as i64),
            _ => None,
        };
        let msg = e.to_string();
        let t = now();
        db::record_failure(
            &self.pool,
            blog.id,
            t,
            &msg,
            t + self.backoff(blog, retry_after),
            forget_feed,
        )
        .await?;
        Ok(Outcome::Failed(msg))
    }

    /// Exponential backoff from the blog's refresh interval, capped at a week,
    /// and never sooner than the server's Retry-After.
    fn backoff(&self, blog: &BlogRow, retry_after: Option<i64>) -> i64 {
        let base = blog.refresh_secs.max(self.c().min_refresh_secs as i64);
        let exp = base
            .saturating_mul(1 << blog.failures.clamp(0, 10))
            .min(MAX_BACKOFF_SECS);
        jitter(exp.max(retry_after.unwrap_or(0)), 10, blog.id)
    }

    /// A feed that did not change gets polled less often next time.
    fn slower(&self, current: i64) -> i64 {
        (current * 3 / 2).clamp(self.c().min_refresh_secs as i64, self.c().max_refresh_secs as i64)
    }
}

fn feed_full_text(item: &feed::ParsedItem) -> Option<String> {
    let body = item.body_html.as_deref()?;
    (html::to_text(body).chars().count() >= FULL_FEED_MIN_CHARS).then(|| html::sanitize(body, Some(&item.url)))
}
