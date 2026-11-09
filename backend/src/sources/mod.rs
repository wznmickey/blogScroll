//! Blog directories: where the list of blogs comes from.
//!
//! To add a directory (e.g. 十年之约 / foreverblog.cn), implement [`Directory`]
//! in a new module and register it in [`enabled`]. A directory only has to
//! say which blogs exist; feed discovery, crawling and storage are shared.

use futures::future::BoxFuture;
use url::Url;

use crate::config::Config;
use crate::http::{PoliteClient, Validators};

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
    out
}
