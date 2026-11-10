use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::StatusCode;
use sqlx::SqlitePool;
use url::Url;

use crate::analyze;
use crate::comments::{self, CommentSystem};
use crate::config::{Config, CrawlerConfig};
use crate::db::{self, BlogRow, Success};
use crate::discovery::{self, Page};
use crate::feed;
use crate::fulltext::FullTextExtractor;
use crate::html;
use crate::http::{ACCEPT_FEED, ACCEPT_IMAGE, FetchError, Fetched, PoliteClient, Validators, looks_like_image};
use crate::profile;
use crate::sources::{Directory, DirectoryFetch};
use crate::util::{jitter, now};

const LAST_SYNC_KEY: &str = "last_directory_sync";
/// A feed whose text is at least this long is treated as full text.
const FULL_FEED_MIN_CHARS: usize = 500;
/// Article pages fetched per blog per crawl, for full-text blogs only.
const PAGES_PER_CRAWL: i64 = 3;
/// An about page shorter than this is taken as not found.
const ABOUT_HTML_MIN_CHARS: usize = 40;
const MAX_BACKOFF_SECS: i64 = 7 * 86400;
/// Posts per batched comment-count request.
const COUNT_BATCH: i64 = 20;
/// Pause after a comment service refuses us (401/403/404).
const COUNT_REFUSED_PAUSE_SECS: i64 = 30 * 86400;
/// Blogs per profile / comment pass.
const ENRICH_ROUND: i64 = 200;
/// Corpus-wide tag statistics are recomputed this often (new posts are
/// tagged as they arrive, against the last statistics).
const RETAG_EVERY: Duration = Duration::from_secs(6 * 3600);

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

impl CrawlStats {
    fn add(&mut self, o: &CrawlStats) {
        self.checked += o.checked;
        self.updated += o.updated;
        self.new_posts += o.new_posts;
        self.not_modified += o.not_modified;
        self.no_feed += o.no_feed;
        self.failed += o.failed;
    }
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

    /// Brings forward the blogs waiting after errors, so they are tried
    /// again now (a feed search that failed is redone). Sites that asked us
    /// to wait (429, 503 with Retry-After) keep their time.
    pub async fn retry_failed(&self) -> anyhow::Result<u64> {
        Ok(sqlx::query(
            "UPDATE blogs SET next_check_at = 0,
                discovery_at = CASE WHEN feed_url IS NULL THEN NULL ELSE discovery_at END
             WHERE active = 1 AND excluded = 0 AND failures > 0
               AND COALESCE(last_error, '') NOT LIKE 'HTTP 429%' AND COALESCE(last_error, '') NOT LIKE 'HTTP 503%'",
        )
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    /// After the rules for reading feeds change (the parser, which entries
    /// are kept, where feeds are looked for), catches up at once instead of
    /// as each blog falls due: feeds read under the old rules are fetched in
    /// full now, and blogs without a feed are searched again.
    pub async fn catch_up_on_new_rules(&self) -> anyhow::Result<()> {
        const FEEDS: &str = "rules_feeds";
        const DISCOVERY: &str = "rules_discovery";
        let prefix = self.hash_prefix();
        if db::kv_get(&self.pool, FEEDS).await?.as_deref() != Some(prefix.as_str()) {
            let n = sqlx::query(
                "UPDATE blogs SET next_check_at = 0
                 WHERE active = 1 AND excluded = 0 AND feed_url IS NOT NULL
                   AND (content_hash IS NULL OR substr(content_hash, 1, length(?1)) <> ?1)",
            )
            .bind(&prefix)
            .execute(&self.pool)
            .await?
            .rows_affected();
            tracing::info!(feeds = n, rules = %prefix, "feed rules changed: reading every feed in full now");
            db::kv_set(&self.pool, FEEDS, &prefix).await?;
        }
        let version = discovery::VERSION.to_string();
        if db::kv_get(&self.pool, DISCOVERY).await?.as_deref() != Some(version.as_str()) {
            let n = sqlx::query(
                "UPDATE blogs SET discovery_at = NULL, next_check_at = 0
                 WHERE active = 1 AND excluded = 0 AND feed_url IS NULL",
            )
            .execute(&self.pool)
            .await?
            .rows_affected();
            tracing::info!(
                blogs = n,
                version,
                "feed discovery changed: searching blogs without a feed again"
            );
            db::kv_set(&self.pool, DISCOVERY, &version).await?;
        }
        Ok(())
    }

    /// Background loop: daily directory sync plus feed refreshes as they fall due.
    pub async fn run(self: Arc<Self>) {
        let mut last_retag: Option<std::time::Instant> = None;
        loop {
            if last_retag.is_none_or(|t| t.elapsed() >= RETAG_EVERY) {
                match analyze::retag_all(&self.pool).await {
                    Ok(s) => tracing::info!(?s, "tags and categories recomputed"),
                    Err(e) => tracing::warn!(error = %e, "retag failed"),
                }
                last_retag = Some(std::time::Instant::now());
            }
            if let Err(e) = self.sync_directories_if_due().await {
                tracing::warn!(error = %e, "directory sync failed");
            }
            match self.crawl_due(1000).await {
                Ok(s) if s.checked > 0 => tracing::info!(?s, "crawl pass finished"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "crawl pass failed"),
            }
            match self.enrich_due(ENRICH_ROUND).await {
                Ok(n) if n > 0 => tracing::info!(blogs = n, "profile / comment pass finished"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "profile / comment pass failed"),
            }
            tokio::time::sleep(Duration::from_secs(self.c().tick_secs.max(5))).await;
        }
    }

    pub async fn sync_directories_if_due(&self) -> anyhow::Result<()> {
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
        self.sync(false).await
    }

    /// Sources read from local files (`[[sources.feeds]]`, OPML), so config
    /// edits apply at start without waiting for the daily sync.
    pub async fn sync_local_directories(&self) -> anyhow::Result<()> {
        self.sync(true).await
    }

    async fn sync(&self, local_only: bool) -> anyhow::Result<()> {
        let mut any_ok = false;
        for dir in self.directories.iter().filter(|d| !local_only || d.is_local()) {
            let key = format!("directory_validators:{}", dir.id());
            let validators: Option<Validators> = db::kv_get(&self.pool, &key)
                .await?
                .and_then(|v| serde_json::from_str(&v).ok());
            match dir.fetch(&self.http, validators.as_ref()).await {
                Ok(DirectoryFetch::NotModified) => {
                    tracing::info!(directory = dir.id(), "directory unchanged");
                    any_ok = true;
                }
                Ok(DirectoryFetch::Fetched { entries, validators }) => {
                    let alive = entries.iter().filter(|e| e.alive).count();
                    let saved =
                        db::save_directory(&self.pool, dir.id(), &entries, self.c().refresh_secs as i64, now()).await?;
                    db::kv_set(&self.pool, &key, &serde_json::to_string(&validators)?).await?;
                    tracing::info!(directory = dir.id(), saved, alive, "directory synced");
                    any_ok = true;
                }
                Err(e) => tracing::warn!(directory = dir.id(), error = %e, "directory fetch failed"),
            }
        }
        db::apply_policy(&self.pool, &self.cfg).await?;
        if any_ok && !local_only {
            db::kv_set(&self.pool, LAST_SYNC_KEY, &now().to_string()).await?;
        }
        Ok(())
    }

    /// Crawls every blog that is due, at most `limit`. Actual request concurrency
    /// is capped inside `PoliteClient`; more tasks than permits just lets one
    /// blog wait for its per-host slot (discovery probes are 10 s apart) while
    /// others proceed. It never raises the load on any single site.
    pub async fn crawl_due(&self, limit: i64) -> anyhow::Result<CrawlStats> {
        let due = db::due_blogs(&self.pool, now(), limit).await?;
        Ok(self.crawl_blogs(due).await)
    }

    /// Crawls in rounds of `round` blogs until nothing is due or `deadline`
    /// passes (checked between rounds). For scheduled runs with a time limit.
    pub async fn crawl_until_done(&self, round: i64, deadline: std::time::Instant) -> anyhow::Result<CrawlStats> {
        let mut total = CrawlStats::default();
        while std::time::Instant::now() < deadline {
            let s = self.crawl_due(round.max(1)).await?;
            if s.checked == 0 {
                break;
            }
            tracing::info!(?s, "crawl round finished");
            total.add(&s);
        }
        while std::time::Instant::now() < deadline && self.enrich_due(ENRICH_ROUND).await? > 0 {}
        Ok(total)
    }

    /// Profiles and comment counts that are due, on their own schedule (not
    /// only when a feed happens to be due), so a fresh install fills in within
    /// minutes. Only blogs whose last contact succeeded. Returns how many ran.
    /// Re-reads the profile of up to `limit` blogs last read before
    /// `before`, due or not; returns how many. A failed read counts as read,
    /// so repeating this until it returns 0 ends.
    pub async fn refresh_profiles_before(&self, before: i64, limit: i64) -> anyhow::Result<usize> {
        let blogs = db::profiles_read_before(&self.pool, before, limit).await?;
        let n = blogs.len();
        let tasks = self.c().max_concurrency.max(1) * 16;
        futures::stream::iter(blogs)
            .for_each_concurrent(tasks, |b| async move {
                if let Err(e) = self.refresh_profile(&b, None).await {
                    tracing::debug!(blog = %b.homepage_url, error = %e, "profile refresh failed");
                    let _ = db::touch_profile(&self.pool, b.id, now()).await;
                }
            })
            .await;
        Ok(n)
    }

    /// Reads one blog's profile now, due or not.
    pub async fn refresh_profile_now(&self, blog: &BlogRow) -> anyhow::Result<()> {
        self.refresh_profile(blog, None).await.map(|_| ())
    }

    pub async fn enrich_due(&self, limit: i64) -> anyhow::Result<usize> {
        let blogs = db::due_for_enrich(
            &self.pool,
            now(),
            self.c().profile_refresh_days as i64 * 86400,
            self.c().comment_count_hours as i64 * 3600,
            limit,
        )
        .await?;
        let n = blogs.len();
        let tasks = self.c().max_concurrency.max(1) * 16;
        futures::stream::iter(blogs)
            .for_each_concurrent(tasks, |b| async move { self.enrich(&b, None).await })
            .await;
        Ok(n)
    }

    pub async fn crawl_blogs(&self, blogs: Vec<BlogRow>) -> CrawlStats {
        let tasks = self.c().max_concurrency.max(1) * 16;
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
        let mut homepage = None;
        let outcome = match self.try_crawl_blog(&blog, &mut homepage).await {
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
        match &outcome {
            Outcome::Failed(msg) => tracing::debug!(blog = %blog.homepage_url, %msg, "crawl failed"),
            // Only touch a site again when it just answered normally.
            _ => self.enrich(&blog, homepage).await,
        }
        outcome
    }

    /// Profile (monthly: homepage + about page) and comment counts (daily, one request).
    async fn enrich(&self, blog: &BlogRow, homepage: Option<Page>) {
        let t = now();
        let mut system = blog
            .comment_system
            .as_deref()
            .and_then(|s| serde_json::from_str::<CommentSystem>(s).ok());
        let profile_every = self.c().profile_refresh_days as i64 * 86400;
        if profile_every > 0 && blog.profile_at.is_none_or(|at| t - at >= profile_every) {
            match self.refresh_profile(blog, homepage).await {
                Ok(Some(found)) => system = Some(found),
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!(blog = %blog.homepage_url, error = %e, "profile refresh failed");
                    let _ = db::touch_profile(&self.pool, blog.id, t).await;
                }
            }
        }
        let count_every = self.c().comment_count_hours as i64 * 3600;
        let Some(sys) = system.filter(|_| count_every > 0 && blog.comments_next_at <= t) else {
            return;
        };
        if !sys.supports_counts() {
            // Nothing to ask; look again after the next profile refresh.
            let _ = db::save_comment_counts(&self.pool, blog.id, &[], t + profile_every.max(86400)).await;
            return;
        }
        if let Err(e) = self.refresh_comment_counts(blog, &sys, count_every).await {
            tracing::debug!(blog = %blog.homepage_url, error = %e, "comment counts failed");
        }
    }

    async fn refresh_profile(&self, blog: &BlogRow, homepage: Option<Page>) -> anyhow::Result<Option<CommentSystem>> {
        let home = match homepage.filter(|p| !p.html.is_empty()) {
            Some(p) => p,
            None => discovery::fetch_page(&self.http, &Url::parse(&blog.homepage_url)?).await?,
        };
        let info = profile::from_home(&home.html, &home.url);
        // The page the author named, else the linked one, else one guess at <site>/about/.
        let about_url = blog
            .about_page
            .as_deref()
            .and_then(|u| Url::parse(u).ok())
            .or_else(|| info.about.clone())
            .or_else(|| discovery::dir_url(&home.url).join("about/").ok());
        let about = match about_url {
            Some(u) => discovery::fetch_page(&self.http, &u)
                .await
                .ok()
                .filter(|p| !p.html.is_empty()),
            None => None,
        };
        let system = about
            .as_ref()
            .and_then(|p| comments::detect(&p.html))
            .or_else(|| comments::detect(&home.html))
            .or_else(|| comments::native(info.generator.as_deref()));

        let about_text = about.as_ref().and_then(|p| profile::about_text(&p.html));
        // The whole page, formatted, for authors who asked for it.
        let about_html = about
            .as_ref()
            .filter(|_| blog.about_full)
            .and_then(|p| crate::fulltext::main_content(&p.html, &p.url, ABOUT_HTML_MIN_CHARS));
        // Many authors list their accounts only on the about page.
        let mut social = info.social.clone();
        if let Some(p) = &about {
            profile::merge_social(&mut social, profile::social_from_page(&p.html, &p.url));
        }
        let social = serde_json::to_string(&social)?;
        let about_pictures = about.as_ref().map(|p| profile::pictures_from_page(&p.html, &p.url));
        let candidates =
            profile::Pictures::candidates(&info.pictures, about_pictures.as_ref(), blog.icon_url.is_some());
        let avatar = self.first_working_picture(candidates).await;
        let system_json = system.as_ref().map(serde_json::to_string).transpose()?;
        db::save_profile(
            &self.pool,
            blog.id,
            now(),
            &db::Profile {
                description: info.description.as_deref(),
                avatar_url: avatar.as_ref().map(|u| u.as_str()),
                about_url: about.as_ref().map(|p| p.url.as_str()),
                about_text: about_text.as_deref(),
                social_json: &social,
                generator: info.generator.as_deref(),
                comment_system_json: system_json.as_deref(),
                framable: home.frame_blocked.map(|blocked| !blocked),
                about_html: about_html.as_deref(),
            },
        )
        .await?;
        Ok(system)
    }

    /// The first picture that loads as an image, one request each (a few at
    /// most). An address with a doubled slash (`/gh//user/…`, a config slip
    /// some CDNs answer with 400) is also tried tidied up. A picture that
    /// cannot be reached at all (timeout, network) is taken: that says
    /// nothing about the picture.
    async fn first_working_picture(&self, candidates: Vec<Url>) -> Option<Url> {
        for url in candidates.into_iter().take(4) {
            let mut tries = vec![url.clone()];
            if url.path().contains("//") {
                let mut tidy = url.clone();
                let mut path = url.path().to_string();
                while path.contains("//") {
                    path = path.replace("//", "/");
                }
                tidy.set_path(&path);
                tries.push(tidy);
            }
            for u in tries {
                match self.http.get(&u, ACCEPT_IMAGE, None).await {
                    Ok(Fetched::Ok { body, .. }) if looks_like_image(&body) => return Some(u),
                    Err(FetchError::Network(_) | FetchError::TooLarge(_)) => return Some(u),
                    Ok(_) | Err(FetchError::Status { .. }) => {
                        tracing::debug!(picture = %u, "not a picture; trying the next");
                    }
                }
            }
        }
        None
    }

    async fn refresh_comment_counts(&self, blog: &BlogRow, sys: &CommentSystem, every: i64) -> anyhow::Result<()> {
        let t = now();
        let posts: Vec<(i64, Url)> = db::recent_post_urls(&self.pool, blog.id, COUNT_BATCH)
            .await?
            .into_iter()
            .filter_map(|(id, u)| Some((id, Url::parse(&u).ok()?)))
            .collect();
        if posts.is_empty() {
            return Ok(db::save_comment_counts(&self.pool, blog.id, &[], t + every).await?);
        }
        let homepage = Url::parse(&blog.homepage_url)?;
        match comments::fetch_counts(&self.http, sys, &homepage, &posts).await {
            Ok(counts) => {
                db::save_comment_counts(&self.pool, blog.id, &counts, t + jitter(every, 10, blog.id)).await?;
                Ok(())
            }
            Err(e) => {
                let refused = e.status().is_some_and(|s| matches!(s.as_u16(), 401 | 403 | 404 | 405));
                let next = t + if refused { COUNT_REFUSED_PAUSE_SECS } else { every };
                db::save_comment_counts(&self.pool, blog.id, &[], next).await?;
                Err(e.into())
            }
        }
    }

    async fn try_crawl_blog(&self, blog: &BlogRow, homepage_out: &mut Option<Page>) -> anyhow::Result<Outcome> {
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
                let discovered = discovery::discover(&self.http, &homepage).await.map(|(found, page)| {
                    *homepage_out = Some(page);
                    found
                });
                match discovered {
                    Ok(Some(found)) => {
                        db::set_discovered_feed(&self.pool, blog.id, found.feed_url.as_str(), t).await?;
                        tracing::info!(blog = %blog.name, feed = %found.feed_url, "feed discovered");
                        let fetched = Fetched::Ok {
                            final_url: found.feed_url.clone(),
                            validators: found.validators,
                            body: found.body,
                            fresh_for: found.fresh_for,
                            frame_blocked: found.frame_blocked,
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
                // After a parser upgrade, or a change to which entries are kept,
                // fetch each feed in full once so stored posts pick up what the
                // new parser extracts and entries skipped before come in.
                let current = blog
                    .content_hash
                    .as_deref()
                    .is_some_and(|h| h.starts_with(&self.hash_prefix()));
                let validators = current.then_some(&validators);
                match self.http.get(&feed_url, ACCEPT_FEED, validators).await {
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
            Fetched::NotModified { fresh_for } => self.unchanged(blog, None, fresh_for).await,
            Fetched::Ok {
                body,
                validators,
                fresh_for,
                frame_blocked,
                ..
            } => {
                // Headers set for the whole site show on the feed too: an
                // early "no" until the homepage gives the real answer.
                if frame_blocked && blog.framable.is_none() {
                    db::set_framable(&self.pool, blog.id, false).await?;
                }
                self.ingest(blog, &feed_url, &body, &validators, fresh_for).await
            }
        }
    }

    /// The feed is the same as last time (a 304, or identical bytes).
    async fn unchanged(
        &self,
        blog: &BlogRow,
        validators: Option<&Validators>,
        fresh_for: Option<Duration>,
    ) -> anyhow::Result<Outcome> {
        if blog.full_text_allowed() {
            self.fetch_full_text(blog).await?;
        }
        let t = now();
        let refresh = self.respect_hints(self.slower(blog.refresh_secs), None, fresh_for);
        db::record_success(
            &self.pool,
            blog.id,
            t,
            Success {
                validators,
                refresh_secs: refresh,
                next_check_at: t + jitter(refresh, 10, blog.id),
                had_new_posts: false,
                icon_url: None,
                content_hash: None,
            },
        )
        .await?;
        Ok(Outcome::NotModified)
    }

    async fn ingest(
        &self,
        blog: &BlogRow,
        feed_url: &Url,
        body: &[u8],
        validators: &Validators,
        fresh_for: Option<Duration>,
    ) -> anyhow::Result<Outcome> {
        let hash = content_hash(&self.hash_prefix(), body);
        if blog.content_hash.as_deref() == Some(hash.as_str()) {
            // The server ignores conditional requests but nothing changed.
            return self.unchanged(blog, Some(validators), fresh_for).await;
        }

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

        let spam = parsed.looks_like_spam();
        db::set_feed_signals(&self.pool, blog.id, parsed.detected_kind(), spam).await?;
        if blog.from_feed_meta {
            let link = parsed.link.as_ref().map(|u| u.as_str());
            db::set_feed_meta(&self.pool, blog.id, parsed.title.as_deref(), link).await?;
        }
        if spam {
            tracing::warn!(blog = %blog.homepage_url, "feed is mostly gambling spam; hiding the source");
            db::delete_posts(&self.pool, blog.id).await?;
        }

        let mut items = if spam { Vec::new() } else { parsed.items };
        items.sort_by_key(|i| std::cmp::Reverse(i.published));
        let max_age = self.c().max_post_age_days as i64 * 86400;
        if max_age > 0 {
            items.retain(|i| i.published.is_none_or(|p| p.timestamp() >= t - max_age));
        }
        if self.c().max_items_per_feed > 0 {
            items.truncate(self.c().max_items_per_feed);
        }

        let new = self.store_items(blog, &items, t).await?;
        if new > 0 && self.c().max_posts_per_blog > 0 {
            db::prune_posts(&self.pool, blog.id, self.c().max_posts_per_blog as i64).await?;
        }
        if blog.full_text_allowed() {
            self.fetch_full_text(blog).await?;
        }

        let refresh = if new > 0 {
            self.c().refresh_secs as i64
        } else {
            self.slower(blog.refresh_secs)
        };
        let refresh = self.respect_hints(refresh, parsed.ttl_minutes, fresh_for);
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
                content_hash: Some(&hash),
            },
        )
        .await?;
        Ok(Outcome::Updated { new })
    }

    /// Saves a feed's entries with their keywords, tags and category. Keyword
    /// extraction is CPU work and runs off the async threads.
    async fn store_items(&self, blog: &BlogRow, items: &[feed::ParsedItem], t: i64) -> anyhow::Result<usize> {
        if items.is_empty() {
            return Ok(0);
        }
        let known = db::post_guids(&self.pool, blog.id).await?;
        // Undated entries keep their feed order (dated ones come first).
        // Below one already known, an unseen one is older, not new: the feed
        // lists more of its past than before, or we used to keep fewer.
        let mut next_backlog = db::backlog_end(&self.pool, blog.id).await?.map_or(0, |s| s - 1);
        let mut below_known = false;
        let backlog_at: Vec<Option<i64>> = items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                if known.is_empty() {
                    return Some(-(idx as i64));
                }
                if item.published.is_some() {
                    return None;
                }
                if known.contains(&item.guid) {
                    below_known = true;
                    return None;
                }
                below_known.then(|| {
                    next_backlog -= 1;
                    next_backlog + 1
                })
            })
            .collect();
        let texts: Vec<(String, Vec<String>, String, Option<String>)> = items
            .iter()
            .map(|i| (i.title.clone(), i.tags.clone(), i.excerpt.clone(), i.body_html.clone()))
            .collect();
        let keywords: Vec<Vec<(String, f32)>> = tokio::task::spawn_blocking(move || {
            texts
                .iter()
                .map(|(title, tags, excerpt, body)| analyze::keywords(title, tags, excerpt, body.as_deref()))
                .collect()
        })
        .await?;

        let corpus = analyze::corpus();
        let full_text = blog.full_text_allowed();
        let mut new = 0;
        let mut tx = self.pool.begin().await?;
        for (idx, (item, kw)) in items.iter().zip(&keywords).enumerate() {
            let content = if full_text { feed_full_text(item) } else { None };
            let content_ref = content.as_deref().map(|c| (c, "feed"));
            let kw_json = serde_json::to_string(kw)?;
            let entry = db::Entry {
                position: idx as i64,
                backlog_at: backlog_at[idx],
                keywords_json: &kw_json,
                content: content_ref,
            };
            let up = db::upsert_post(&mut tx, blog.id, item, t, &entry).await?;
            new += usize::from(up.is_new);
            let derived = analyze::derive(blog.id, &item.tags, kw, &corpus);
            analyze::save_derived(&mut tx, up.id, &derived).await?;
        }
        tx.commit().await?;
        analyze::update_blog_categories(&self.pool, &[blog.id]).await?;
        Ok(new)
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

    /// The publisher's own hints (RSS `<ttl>`, HTTP `Cache-Control` / `Expires`)
    /// are a lower bound on the interval: never ask before they say it may change.
    fn respect_hints(&self, refresh: i64, ttl_minutes: Option<u32>, fresh_for: Option<Duration>) -> i64 {
        let hint = [
            ttl_minutes.map(|m| m as i64 * 60),
            fresh_for.map(|d| d.as_secs() as i64),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0);
        refresh.max(hint).min(self.c().max_refresh_secs as i64)
    }

    /// A feed that did not change gets polled less often next time.
    fn slower(&self, current: i64) -> i64 {
        (current * 3 / 2).clamp(self.c().min_refresh_secs as i64, self.c().max_refresh_secs as i64)
    }

    /// The parser version and the settings that decide which entries are
    /// kept; when any of them changes, every feed is read in full once more.
    fn hash_prefix(&self) -> String {
        format!(
            "v{}-{}d{}i-",
            feed::PARSER_VERSION,
            self.c().max_post_age_days,
            self.c().max_items_per_feed
        )
    }
}

/// Change detector for feed bodies, tagged with `Crawler::hash_prefix`.
/// `DefaultHasher` is stable within a build; after a toolchain upgrade each
/// feed is simply parsed once more.
fn content_hash(prefix: &str, body: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    body.hash(&mut h);
    format!("{prefix}{:016x}", h.finish())
}

fn feed_full_text(item: &feed::ParsedItem) -> Option<String> {
    let body = item.body_html.as_deref()?;
    (html::to_text(body).chars().count() >= FULL_FEED_MIN_CHARS).then(|| html::sanitize(body, Some(&item.url)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_rules_bring_stale_feeds_and_feedless_blogs_forward_once() {
        let pool = crate::api::tests::seeded().await.pool;
        let crawler = Crawler::new(
            pool.clone(),
            Arc::new(Config::default()),
            Vec::new(),
            Box::new(crate::fulltext::ArticleExtractor),
        )
        .unwrap();
        let current = format!("{}abc", crawler.hash_prefix());
        // Blog 1 was read under the current rules, blog 2 under older ones;
        // blog 3 has no feed and was searched recently.
        sqlx::query(
            "UPDATE blogs SET next_check_at = 9999999999,
                feed_url = CASE id WHEN 3 THEN NULL ELSE 'https://f.example/' || id END,
                content_hash = CASE id WHEN 1 THEN ? WHEN 2 THEN 'v1-30d20i-abc' END,
                discovery_at = CASE id WHEN 3 THEN 1000 END",
        )
        .bind(&current)
        .execute(&pool)
        .await
        .unwrap();
        let due = |pool: SqlitePool| async move {
            sqlx::query_scalar::<_, i64>("SELECT id FROM blogs WHERE next_check_at = 0 ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap()
        };
        // Blog 3 is not alive in the seed; make it so for the discovery part.
        sqlx::query("UPDATE blogs SET active = 1").execute(&pool).await.unwrap();

        crawler.catch_up_on_new_rules().await.unwrap();
        assert_eq!(due(pool.clone()).await, [2, 3]);
        let discovery_at: Option<i64> = sqlx::query_scalar("SELECT discovery_at FROM blogs WHERE id = 3")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(discovery_at, None, "searched again despite the recent attempt");

        sqlx::query("UPDATE blogs SET next_check_at = 9999999999")
            .execute(&pool)
            .await
            .unwrap();
        crawler.catch_up_on_new_rules().await.unwrap();
        assert!(due(pool.clone()).await.is_empty(), "only once per change of rules");
    }

    #[tokio::test]
    async fn undated_entries_below_known_ones_are_old() {
        let pool = crate::api::tests::seeded().await.pool;
        let crawler = Crawler::new(
            pool.clone(),
            Arc::new(Config::default()),
            Vec::new(),
            Box::new(crate::fulltext::ArticleExtractor),
        )
        .unwrap();
        sqlx::query("DELETE FROM posts WHERE blog_id = 2")
            .execute(&pool)
            .await
            .unwrap();
        let blog = db::blog_by_id(&pool, 2).await.unwrap().unwrap();
        let items = |guids: &[&str]| -> Vec<feed::ParsedItem> {
            guids
                .iter()
                .map(|g| feed::ParsedItem {
                    guid: g.to_string(),
                    url: Url::parse(&format!("https://b.example/{g}")).unwrap(),
                    title: g.to_string(),
                    excerpt: String::new(),
                    excerpt_html: None,
                    cover: None,
                    author: None,
                    tags: vec![],
                    published: None,
                    body_html: None,
                    comments_url: None,
                    comment_count: None,
                })
                .collect()
        };
        // The feed used to be cut to its first entries; read in full, it has
        // older ones below them, and meanwhile one new post at the top.
        crawler.store_items(&blog, &items(&["b", "c"]), 1_000).await.unwrap();
        let new = crawler
            .store_items(&blog, &items(&["a", "b", "c", "d", "e"]), 2_000)
            .await
            .unwrap();
        assert_eq!(new, 3);
        let rows: Vec<(String, i64, bool)> =
            sqlx::query_as("SELECT guid, sort_at, undated FROM posts WHERE blog_id = 2 ORDER BY undated, sort_at DESC")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            [
                ("a".into(), 2_000, false),
                ("b".into(), 0, true),
                ("c".into(), -1, true),
                ("d".into(), -2, true),
                ("e".into(), -3, true),
            ]
        );
    }
}
