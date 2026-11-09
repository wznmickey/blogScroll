//! 开往 Travellings (https://www.travellings.cn), via its public member list.
//!
//! `GET https://api.travellings.cn/all` returns every member with a status
//! from their daily checker. The list changes about once a day, so we sync
//! at most daily and fall back to the static mirror if the API is down.

use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Deserialize;
use url::Url;

use super::{Directory, DirectoryEntry};
use crate::config::TravellingsConfig;
use crate::http::{ACCEPT_JSON, Fetched, PoliteClient};
use crate::util::parse_http_url;

pub struct Travellings {
    cfg: TravellingsConfig,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    success: bool,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Vec<Member>,
}

#[derive(Debug, Deserialize)]
struct Member {
    id: u64,
    name: String,
    url: String,
    /// RUN, LOST, ERROR, TIMEOUT, WAIT, or an HTTP status code like "404".
    status: String,
    /// Comma separated, e.g. "go,blog,tech".
    #[serde(default)]
    tag: Option<String>,
}

impl Travellings {
    pub fn new(cfg: TravellingsConfig) -> Self {
        Self { cfg }
    }

    async fn fetch_list(&self, http: &PoliteClient, url: &str) -> anyhow::Result<Vec<DirectoryEntry>> {
        let url = Url::parse(url)?;
        let body = match http.get(&url, ACCEPT_JSON, None).await? {
            Fetched::Ok { body, .. } => body,
            Fetched::NotModified => anyhow::bail!("unexpected 304"),
        };
        self.parse(&body)
    }

    fn parse(&self, body: &[u8]) -> anyhow::Result<Vec<DirectoryEntry>> {
        let resp: ApiResponse = serde_json::from_slice(body)?;
        if !resp.success {
            anyhow::bail!("travellings API error: {}", resp.msg.unwrap_or_default());
        }
        let entries = resp
            .data
            .into_iter()
            .filter_map(|m| {
                let homepage = parse_http_url(&m.url)?;
                let status = m.status.trim().to_ascii_uppercase();
                let alive = self.cfg.live_statuses.iter().any(|s| s.eq_ignore_ascii_case(&status));
                let tags = m
                    .tag
                    .unwrap_or_default()
                    .split(',')
                    .map(str::trim)
                    // "go" only marks participation in random jumps.
                    .filter(|t| !t.is_empty() && *t != "go")
                    .map(str::to_string)
                    .collect();
                Some(DirectoryEntry {
                    external_id: m.id.to_string(),
                    name: m.name.trim().to_string(),
                    homepage,
                    feed_url: None,
                    tags,
                    status,
                    alive,
                })
            })
            .collect();
        Ok(entries)
    }
}

impl Directory for Travellings {
    fn id(&self) -> &'static str {
        "travellings"
    }

    fn name(&self) -> &'static str {
        "开往"
    }

    fn homepage(&self) -> &'static str {
        "https://www.travellings.cn"
    }

    fn fetch<'a>(&'a self, http: &'a PoliteClient) -> BoxFuture<'a, anyhow::Result<Vec<DirectoryEntry>>> {
        async move {
            match self.fetch_list(http, &self.cfg.api_url).await {
                Ok(list) => Ok(list),
                Err(e) => {
                    let Some(fallback) = &self.cfg.fallback_url else {
                        return Err(e);
                    };
                    tracing::warn!(error = %e, "travellings API failed, trying mirror");
                    self.fetch_list(http, fallback).await
                }
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_response_shape() {
        let t = Travellings::new(TravellingsConfig::default());
        let body = include_bytes!("../../tests/fixtures/travellings_all.json");
        let list = t.parse(body).unwrap();
        assert_eq!(list.len(), 7);

        let first = &list[0];
        assert_eq!(first.external_id, "1");
        assert_eq!(first.homepage.as_str(), "https://blog.oalo.cn/");
        assert_eq!(first.tags, ["blog", "tech"]);
        assert!(first.alive);

        let alive: Vec<&str> = list.iter().filter(|e| e.alive).map(|e| e.status.as_str()).collect();
        assert_eq!(alive, ["RUN", "RUN", "LOST", "RUN"]);
    }

    #[test]
    fn api_error_is_reported() {
        let t = Travellings::new(TravellingsConfig::default());
        let err = t.parse(br#"{"success":false,"msg":"boom"}"#).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }
}
