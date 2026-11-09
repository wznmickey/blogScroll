use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub crawler: CrawlerConfig,
    pub sources: SourcesConfig,
    pub policy: PolicyConfig,
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            tracing::warn!(path = %path.display(), "config file not found, using defaults");
            let mut cfg = Self::default();
            cfg.apply_env();
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.apply_env();
        Ok(cfg)
    }

    /// `FEED_USER_AGENT` overrides the configured User-Agent, so CI and
    /// containers can set it without editing files.
    fn apply_env(&mut self) {
        if let Ok(ua) = std::env::var("FEED_USER_AGENT")
            && !ua.trim().is_empty()
        {
            self.crawler.user_agent = ua.trim().to_string();
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: String,
    pub site_name: String,
    /// Built frontend (`frontend/dist`). Served at `/` when present.
    pub static_dir: Option<PathBuf>,
    /// API responses kept in memory (each lives 1–10 minutes).
    pub api_cache_entries: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".into(),
            site_name: "博客漫游".into(),
            static_dir: Some("../frontend/dist".into()),
            api_cache_entries: 512,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: "data/feed.db".into(),
        }
    }
}

/// Everything that controls how hard we hit other people's sites.
/// The defaults are deliberately conservative.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CrawlerConfig {
    /// Sent with every request. Please put a way to contact you in here.
    pub user_agent: String,
    /// How many requests may be in flight at once, across all sites.
    pub max_concurrency: usize,
    /// Minimum gap between two requests to the same host.
    pub per_host_interval_secs: u64,
    pub request_timeout_secs: u64,
    pub max_body_bytes: usize,
    /// Starting refresh interval for a feed. It adapts between min and max:
    /// feeds that rarely change are polled less often.
    pub refresh_secs: u64,
    pub min_refresh_secs: u64,
    pub max_refresh_secs: u64,
    /// After a failed discovery (no feed found) wait this long before trying again.
    pub discovery_retry_secs: u64,
    pub directory_sync_secs: u64,
    /// How often the scheduler wakes up to look for due feeds.
    pub tick_secs: u64,
    /// Only the newest N entries of a feed are read on each fetch.
    pub max_items_per_feed: usize,
    /// Older posts beyond this many per blog are deleted, keeping the database bounded.
    pub max_posts_per_blog: usize,
    /// Entries older than this are ignored (0 = keep everything).
    pub max_post_age_days: u32,
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            user_agent: concat!(
                "FeedWaterfall/",
                env!("CARGO_PKG_VERSION"),
                " (RSS reader; polite; +https://github.com/your-name/feed)"
            )
            .into(),
            max_concurrency: 4,
            per_host_interval_secs: 10,
            request_timeout_secs: 20,
            max_body_bytes: 4 * 1024 * 1024,
            refresh_secs: 4 * 3600,
            min_refresh_secs: 2 * 3600,
            max_refresh_secs: 24 * 3600,
            discovery_retry_secs: 7 * 86400,
            directory_sync_secs: 86400,
            tick_secs: 30,
            max_items_per_feed: 20,
            max_posts_per_blog: 50,
            max_post_age_days: 365,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SourcesConfig {
    pub travellings: TravellingsConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TravellingsConfig {
    pub enabled: bool,
    pub api_url: String,
    pub fallback_url: Option<String>,
    /// Directory statuses treated as "site is alive". RUN = all good,
    /// LOST = site is up but the webring badge was not found.
    pub live_statuses: Vec<String>,
}

impl Default for TravellingsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: "https://api.travellings.cn/all".into(),
            fallback_url: Some("https://backup.api.travellings.cn/list.json".into()),
            live_statuses: vec!["RUN".into(), "LOST".into()],
        }
    }
}

/// Per-author choices. Sites are matched by `util::site_key`, so
/// `https://www.example.com/` and `example.com` are the same site.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    /// Authors who asked not to be included. Their posts are hidden and
    /// their sites are never requested.
    pub exclude: Vec<String>,
    /// Authors who explicitly allowed full-text fetching.
    pub full_text: Vec<FullTextGrant>,
    /// Feed URLs set by hand (skips discovery).
    pub feed_overrides: Vec<FeedOverride>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullTextGrant {
    pub site: String,
    /// Where/when the permission was given, for your own records.
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedOverride {
    pub site: String,
    pub feed: String,
}
