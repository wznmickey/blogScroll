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
    pub backup: BackupConfig,
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
        cfg.server.columns = cfg.server.columns.clamp(1, 12);
        let dir = path.parent().unwrap_or(Path::new(""));
        if let Some(f) = cfg.sources.readers.file.as_mut().filter(|f| f.is_relative()) {
            *f = dir.join(&*f);
        }
        for o in &mut cfg.sources.opml {
            if o.path.is_relative() {
                o.path = dir.join(&o.path);
            }
        }
        if let Some(p) = cfg.backup.git_dir.as_mut().filter(|p| p.is_relative()) {
            *p = dir.join(&*p);
        }
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
    /// Who runs the site; shown on the about page.
    pub maintainer: Option<Maintainer>,
    /// Columns of the waterfall on the widest screens (1–12); narrower
    /// screens get fewer, down to two on a phone. Readers can choose their own.
    pub columns: u32,
    /// The site's source repository (`https://github.com/<owner>/<repo>`):
    /// bloggers are pointed there to ask for changes (issues, pull requests).
    pub repo: Option<String>,
    /// The config file a pull request should change, relative to the repo.
    pub repo_config: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Maintainer {
    pub name: String,
    /// Their own site, e.g. their blog's homepage.
    #[serde(default)]
    pub url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".into(),
            site_name: "博客瀑布".into(),
            static_dir: Some("../frontend/dist".into()),
            api_cache_entries: 512,
            maintainer: None,
            columns: 6,
            repo: None,
            repo_config: "deploy/config.actions.toml".into(),
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
    /// Re-read each blog's homepage and about page this often (days).
    pub profile_refresh_days: u32,
    /// Ask a blog's comment service for counts at most this often (hours).
    /// One batched request per blog; 0 disables comment-service counts.
    pub comment_count_hours: u32,
    /// Only the newest N entries of a feed are read on each fetch; 0 (the default) reads them all.
    pub max_items_per_feed: usize,
    /// Older posts beyond this many per blog are deleted; 0 (the default) keeps them all.
    pub max_posts_per_blog: usize,
    /// Entries older than this are ignored; 0 (the default) keeps them all.
    pub max_post_age_days: u32,
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            user_agent: concat!(
                "blogScroll/",
                env!("CARGO_PKG_VERSION"),
                " (RSS reader; low frequency; honours ETag, Cache-Control and Retry-After)"
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
            profile_refresh_days: 30,
            comment_count_hours: 24,
            max_items_per_feed: 0,
            max_posts_per_blog: 0,
            max_post_age_days: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SourcesConfig {
    pub travellings: TravellingsConfig,
    /// Feeds you add yourself: news sites, podcasts, newsletters, any RSS.
    pub feeds: Vec<FeedSource>,
    /// Feed-reader exports (OPML); every feed in them is added like `feeds`.
    pub opml: Vec<OpmlSource>,
    /// Feeds readers added themselves, shown only to them (sources/readers.rs).
    pub readers: ReadersConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReadersConfig {
    pub enabled: bool,
    /// Also the feeds listed here, one address per line (the crawl workflow
    /// writes it from the Worker's database). Relative to the config file.
    pub file: Option<PathBuf>,
}

impl Default for ReadersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            file: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedSource {
    pub url: String,
    /// Shown instead of the feed's own title.
    #[serde(default)]
    pub name: Option<String>,
    /// The site the feed belongs to. Give it to merge with a webring member.
    #[serde(default)]
    pub homepage: Option<String>,
    /// blog | news | newsletter | podcast | video | forum | other
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpmlSource {
    /// Relative paths are relative to the config file's directory.
    pub path: PathBuf,
    /// Kind for every feed in the file.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
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
    pub full_text: Vec<Grant>,
    /// Authors who allowed their comment widget to be embedded in our post view
    /// (they must also allow our domain in their comment service).
    pub embed_comments: Vec<Grant>,
    /// Feed URLs set by hand (skips discovery).
    pub feed_overrides: Vec<FeedOverride>,
    /// About-page settings an author asked for.
    pub about: Vec<AboutPolicy>,
    /// What an author asked to be shown instead of what was found on their
    /// site: picture, introduction, accounts, tags.
    pub profile: Vec<ProfilePolicy>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilePolicy {
    pub site: String,
    /// The blog's picture (an image URL).
    #[serde(default)]
    pub avatar: Option<String>,
    /// A one-line introduction, instead of the homepage's description.
    #[serde(default)]
    pub description: Option<String>,
    /// The author's accounts, instead of the links found (profile URLs; the
    /// platform is recognised from each).
    #[serde(default)]
    pub social: Option<Vec<String>>,
    /// The blog's tags (标签), instead of those its posts use most.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// The about page, as in `[[policy.about]]`: which page to read, keep it
    /// whole, or the author's own introduction instead.
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub about_full: bool,
    #[serde(default)]
    pub about_text: Option<String>,
    #[serde(default)]
    pub note: String,
}

impl PolicyConfig {
    /// About-page settings from `[[policy.about]]` and `[[policy.profile]]`
    /// (a later one for the same site wins).
    pub fn about_settings(&self) -> Vec<AboutPolicy> {
        let from_profile = self
            .profile
            .iter()
            .filter(|p| p.about.is_some() || p.about_full || p.about_text.is_some())
            .map(|p| AboutPolicy {
                site: p.site.clone(),
                url: p.about.clone(),
                full: p.about_full,
                text: p.about_text.clone(),
                note: p.note.clone(),
            });
        self.about.iter().cloned().chain(from_profile).collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AboutPolicy {
    pub site: String,
    /// The about page, instead of the one found from the homepage.
    #[serde(default)]
    pub url: Option<String>,
    /// Keep the whole page, formatted, rather than a short excerpt.
    #[serde(default)]
    pub full: bool,
    /// The author's own introduction, shown instead of the page's.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
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

/// Commits a text snapshot of what was crawled to a git repository (e.g. a
/// private one on GitHub). See `backup.rs`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackupConfig {
    /// Working copy to write and commit in; unset = no backups. Relative
    /// paths are relative to the config file's directory.
    pub git_dir: Option<PathBuf>,
    /// Cloned into `git_dir` when that is not a repository yet, e.g.
    /// `git@github.com:you/blogscroll-data.git`. Credentials are git's own
    /// (an SSH deploy key, or a token in the URL).
    pub remote: Option<String>,
    /// While serving, commit this often (hours); 0 = only `feed-server backup`.
    pub every_hours: u32,
    /// Push after each commit.
    pub push: bool,
    /// Commit even a snapshot with far fewer blogs or posts than the last.
    pub allow_shrink: bool,
    /// When another crawler pushed first: wait a random time in this range
    /// (minutes), then take its snapshot in, commit and push again.
    pub conflict_wait_minutes: [u64; 2],
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            git_dir: None,
            remote: None,
            every_hours: 6,
            push: true,
            allow_shrink: false,
            conflict_wait_minutes: [5, 10],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The configs in the repository load (unknown keys are errors).
    #[test]
    fn shipped_configs_load() {
        for path in [
            "config.example.toml",
            "../deploy/config.actions.toml",
            "../deploy/config.docker.toml",
        ] {
            let cfg = Config::load(Path::new(path)).unwrap_or_else(|e| panic!("{path}: {e:#}"));
            assert_eq!(cfg.crawler.max_items_per_feed, 0, "{path}");
        }
        let actions = Config::load(Path::new("../deploy/config.actions.toml")).unwrap();
        assert_eq!(actions.server.maintainer.unwrap().name, "静静");
        assert_eq!(
            actions.server.repo.as_deref(),
            Some("https://github.com/wznmickey/blogScroll")
        );
        assert!(actions.policy.about[0].full);
    }
}
