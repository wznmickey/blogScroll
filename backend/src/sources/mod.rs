//! Where the list of sources comes from: blog directories (webrings), plus
//! feeds listed in the config or in OPML files.
//!
//! To add a directory (e.g. 十年之约 / foreverblog.cn), implement [`Directory`]
//! in a new module and register it in [`enabled`]. A directory only has to
//! say which blogs exist; feed discovery, crawling and storage are shared.

use futures::future::BoxFuture;
use url::Url;

use crate::config::Config;
use crate::http::{PoliteClient, Validators};

pub mod manual;
pub mod travellings;

#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    /// Stable id of the blog inside this directory.
    pub external_id: String,
    pub name: String,
    pub homepage: Url,
    /// Some directories publish feed URLs; when absent we discover one.
    pub feed_url: Option<Url>,
    pub tags: Vec<String>,
    /// Raw status string as reported by the directory, for display/debugging.
    pub status: String,
    /// Whether the directory considers the site alive.
    pub alive: bool,
    /// Source kind (`news`, `podcast`, ...) when the directory knows it.
    pub kind: Option<String>,
    /// No name was given: use the feed's own title (and site link) once fetched.
    pub name_from_feed: bool,
}

pub enum DirectoryFetch {
    /// The list did not change since the validators we sent.
    NotModified,
    Fetched {
        entries: Vec<DirectoryEntry>,
        /// Stored and sent back next time for a conditional request.
        validators: Validators,
    },
}

pub trait Directory: Send + Sync {
    /// Short machine id, stored in the database (`travellings`).
    fn id(&self) -> &'static str;
    /// Human-readable name shown in the UI (`开往`).
    fn name(&self) -> &'static str;
    fn homepage(&self) -> &'static str;
    /// Display names for this directory's tags, e.g. `("tech", "技术")`.
    fn tag_labels(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    /// Read from local files, so it can be synced at every start.
    fn is_local(&self) -> bool {
        false
    }
    /// Fetches the complete member list. Called about once a day.
    fn fetch<'a>(
        &'a self,
        http: &'a PoliteClient,
        validators: Option<&'a Validators>,
    ) -> BoxFuture<'a, anyhow::Result<DirectoryFetch>>;
}

pub fn enabled(cfg: &Config) -> Vec<Box<dyn Directory>> {
    let mut out: Vec<Box<dyn Directory>> = Vec::new();
    if cfg.sources.travellings.enabled {
        out.push(Box::new(travellings::Travellings::new(cfg.sources.travellings.clone())));
    }
    if !cfg.sources.feeds.is_empty() || !cfg.sources.opml.is_empty() {
        out.push(Box::new(manual::Manual::new(
            cfg.sources.feeds.clone(),
            cfg.sources.opml.clone(),
        )));
    }
    out
}
