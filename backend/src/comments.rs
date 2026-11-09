//! Blogs' comment systems.
//!
//! - **Detection** reads only pages we fetch anyway (homepage, about page) and
//!   trusts configuration *values* (an `envId`, a `serverURL`, a giscus repo),
//!   never a bare product name, which is often just a tag or a theme option.
//! - **Counts** use each system's public count endpoint: one batched request
//!   per blog per day. 401/403 means the owner restricted access, and we
//!   respect that (no Referer/Origin spoofing).
//! - **Embedding** is for blogs whose authors allowed it; see `EmbedConfig`.

use std::sync::LazyLock;

use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::http::{ACCEPT_JSON, FetchError, Fetched, PoliteClient};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CommentSystem {
    Twikoo {
        env_id: String,
    },
    Waline {
        server_url: String,
    },
    Artalk {
        server: String,
        site: Option<String>,
    },
    Giscus {
        repo: String,
        repo_id: Option<String>,
        category: Option<String>,
        category_id: Option<String>,
        mapping: Option<String>,
    },
    Utterances {
        repo: String,
        issue_term: Option<String>,
    },
    Disqus {
        shortname: String,
    },
    Valine,
    /// Halo 2's built-in comments.
    Halo,
    /// Comments built into the blog engine (WordPress, Typecho, ...); counts come from RSS.
    Native {
        generator: String,
    },
}

impl CommentSystem {
    pub fn label(&self) -> String {
        match self {
            CommentSystem::Twikoo { .. } => "Twikoo".into(),
            CommentSystem::Waline { .. } => "Waline".into(),
            CommentSystem::Artalk { .. } => "Artalk".into(),
            CommentSystem::Giscus { .. } => "Giscus".into(),
            CommentSystem::Utterances { .. } => "Utterances".into(),
            CommentSystem::Disqus { .. } => "Disqus".into(),
            CommentSystem::Valine => "Valine".into(),
            CommentSystem::Halo => "Halo".into(),
            CommentSystem::Native { generator } => generator.split_whitespace().next().unwrap_or("内置").to_string(),
        }
    }

    pub fn supports_counts(&self) -> bool {
        match self {
            CommentSystem::Twikoo { env_id } => is_http(env_id),
            CommentSystem::Waline { .. } | CommentSystem::Artalk { .. } | CommentSystem::Halo => true,
            _ => false,
        }
    }

    /// Widgets we can load from a pinned CDN build (never from the blog itself).
    fn embeddable(&self) -> bool {
        matches!(
            self,
            CommentSystem::Twikoo { .. }
                | CommentSystem::Waline { .. }
                | CommentSystem::Artalk { .. }
                | CommentSystem::Giscus { .. }
                | CommentSystem::Utterances { .. }
        )
    }
}

fn is_http(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

// ---------- detection ----------

/// `key: 'value'`, `"key": "value"`, `key = "value"` for any of `keys`.
fn key_value(keys: &str) -> Regex {
    Regex::new(&format!(
        r#"(?i)["']?\b(?:{keys})["']?\s*[:=]\s*["']([^"'\s<>]{{2,300}})["']"#
    ))
    .unwrap()
}

static TWIKOO_ENV: LazyLock<Regex> = LazyLock::new(|| key_value("envId|env_id|envid|twikooEnvId|twikoo_env_id"));
static SERVER_URL: LazyLock<Regex> = LazyLock::new(|| key_value("serverURL|serverUrl|server_url"));
static SERVER: LazyLock<Regex> = LazyLock::new(|| key_value("server"));
static SITE: LazyLock<Regex> = LazyLock::new(|| key_value("site|siteName|site_name"));
static REPO: LazyLock<Regex> = LazyLock::new(|| key_value("repo"));
static REPO_ID: LazyLock<Regex> = LazyLock::new(|| key_value("repoId|repo_id"));
static CATEGORY: LazyLock<Regex> = LazyLock::new(|| key_value("category"));
static CATEGORY_ID: LazyLock<Regex> = LazyLock::new(|| key_value("categoryId|category_id"));
static MAPPING: LazyLock<Regex> = LazyLock::new(|| key_value("mapping"));
static VALINE_APP: LazyLock<Regex> = LazyLock::new(|| key_value("appId|app_id|appid"));
static DISQUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)//([a-z0-9-]{2,64})\.disqus\.com/embed\.js").unwrap());
static USE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)["']?use["']?\s*[:=]\s*\[?\s*["'](\w+)["']"#).unwrap());

fn unescape(s: &str) -> String {
    s.replace("\\/", "/").replace("\\u002F", "/").replace("\\u002f", "/")
}

/// Whether `word` occurs within `before` bytes before `pos` or `after` bytes after it.
fn near(lower: &str, pos: usize, word: &str, before: usize, after: usize) -> bool {
    let start = lower.floor_char_boundary(pos.saturating_sub(before));
    let end = lower.ceil_char_boundary((pos + after).min(lower.len()));
    lower[start..end].contains(word)
}

/// First value of `re` that sits near a mention of `word`.
fn value_near(html: &str, lower: &str, re: &Regex, word: &str, valid: impl Fn(&str) -> bool) -> Option<String> {
    re.captures_iter(html).find_map(|c| {
        let m = c.get(1)?;
        let v = unescape(m.as_str());
        (near(lower, m.start(), word, 1500, 200) && valid(&v)).then_some(v)
    })
}

/// Value of `re` in the `window` bytes after `from`.
fn value_after(html: &str, from: usize, window: usize, re: &Regex) -> Option<String> {
    let end = html.ceil_char_boundary((from + window).min(html.len()));
    re.captures(&html[from..end]).map(|c| unescape(&c[1]))
}

/// Comments built into the blog engine, judging by `<meta name="generator">`.
pub fn native(generator: Option<&str>) -> Option<CommentSystem> {
    let g = generator?;
    let engine = ["WordPress", "Typecho", "Z-Blog", "Emlog"]
        .into_iter()
        .find(|e| g.contains(e))?;
    Some(CommentSystem::Native {
        generator: engine.to_string(),
    })
}

pub fn detect(html: &str) -> Option<CommentSystem> {
    let lower = html.to_ascii_lowercase();
    let mut found: Vec<CommentSystem> = Vec::new();

    if let Some(env_id) = value_near(html, &lower, &TWIKOO_ENV, "twikoo", |v| v.len() >= 4) {
        found.push(CommentSystem::Twikoo { env_id });
    }
    if let Some(server_url) = value_near(html, &lower, &SERVER_URL, "waline", is_http) {
        found.push(CommentSystem::Waline {
            server_url: server_url.trim_end_matches('/').to_string(),
        });
    }
    for (pos, _) in lower.match_indices("artalk") {
        if let Some(server) = value_after(html, pos, 800, &SERVER).filter(|s| is_http(s)) {
            found.push(CommentSystem::Artalk {
                server: server.trim_end_matches('/').to_string(),
                site: value_after(html, pos, 800, &SITE),
            });
            break;
        }
    }
    if let Some(g) = giscus(html, &lower) {
        found.push(g);
    }
    if let Some(u) = script_attrs(html, "utteranc.es").and_then(|a| {
        Some(CommentSystem::Utterances {
            repo: a.get("repo")?.clone(),
            issue_term: a.get("issue-term").cloned(),
        })
    }) {
        found.push(u);
    }
    if let Some(c) = DISQUS.captures(html) {
        found.push(CommentSystem::Disqus {
            shortname: c[1].to_ascii_lowercase(),
        });
    }
    if lower.contains("<comment-widget") && lower.contains("content.halo.run") || lower.contains("plugincommentwidget")
    {
        found.push(CommentSystem::Halo);
    }
    if value_near(html, &lower, &VALINE_APP, "valine", |v| v.len() >= 8).is_some() {
        found.push(CommentSystem::Valine);
    }

    // A theme config may name the one in use: "use":"waline", use: ['Twikoo'].
    let used: Vec<String> = USE.captures_iter(html).map(|c| c[1].to_ascii_lowercase()).collect();
    if let Some(pick) = found
        .iter()
        .find(|s| used.iter().any(|u| s.label().eq_ignore_ascii_case(u)))
    {
        return Some(pick.clone());
    }
    found.into_iter().next()
}

fn script_attrs(html: &str, src_part: &str) -> Option<std::collections::HashMap<String, String>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("script[src]").unwrap();
    let el = doc
        .select(&sel)
        .find(|e| e.value().attr("src").is_some_and(|s| s.contains(src_part)))?;
    Some(
        el.value()
            .attrs()
            .filter_map(|(k, v)| {
                k.strip_prefix("data-")
                    .or(Some(k))
                    .map(|k| (k.to_string(), v.to_string()))
            })
            .collect(),
    )
}

fn giscus(html: &str, lower: &str) -> Option<CommentSystem> {
    if let Some(a) = script_attrs(html, "giscus.app") {
        return Some(CommentSystem::Giscus {
            repo: a.get("repo")?.clone(),
            repo_id: a.get("repo-id").cloned(),
            category: a.get("category").cloned(),
            category_id: a.get("category-id").cloned(),
            mapping: a.get("mapping").cloned(),
        });
    }
    // Inline theme config: giscus: { repo: 'a/b', repoId: '...', ... }
    let pos = lower.find("giscus")?;
    let repo = value_after(html, pos, 1200, &REPO).filter(|r| r.contains('/'))?;
    Some(CommentSystem::Giscus {
        repo,
        repo_id: value_after(html, pos, 1200, &REPO_ID),
        category: value_after(html, pos, 1200, &CATEGORY),
        category_id: value_after(html, pos, 1200, &CATEGORY_ID),
        mapping: value_after(html, pos, 1200, &MAPPING),
    })
}

// ---------- counts in RSS ----------

static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<(?:item|entry)[\s>].*?</(?:item|entry)>").unwrap());
static ITEM_COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(?:slash:comments|thr:total)>\s*(\d{1,7})\s*</(?:slash:comments|thr:total)>").unwrap()
});

/// `<slash:comments>` (WordPress, Typecho, ...) / `<thr:total>` per entry,
/// in document order. Empty when the entries cannot be matched up reliably.
pub fn feed_comment_counts(body: &[u8], entries: usize) -> Vec<Option<i64>> {
    let text = String::from_utf8_lossy(body);
    let items: Vec<Option<i64>> = ITEM
        .find_iter(&text)
        .map(|m| ITEM_COUNT.captures(m.as_str()).and_then(|c| c[1].parse().ok()))
        .collect();
    if items.len() == entries && items.iter().any(Option::is_some) {
        items
    } else {
        Vec::new()
    }
}

// ---------- counts from comment services ----------

/// Path keys a widget would use for this post: the URL path as the browser
/// reports it, plus the decoded form (some widgets decodeURI() it).
fn path_keys(url: &Url) -> Vec<String> {
    let raw = url.path().to_string();
    let decoded = percent_decode(&raw);
    if decoded == raw { vec![raw] } else { vec![raw, decoded] }
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

/// One request for the whole batch. Returns (post id, count) for posts the service knows.
pub async fn fetch_counts(
    http: &PoliteClient,
    system: &CommentSystem,
    homepage: &Url,
    posts: &[(i64, Url)],
) -> Result<Vec<(i64, i64)>, FetchError> {
    let keys: Vec<(i64, String)> = posts
        .iter()
        .flat_map(|(id, url)| path_keys(url).into_iter().map(move |k| (*id, k)))
        .collect();
    let all: Vec<String> = keys.iter().map(|(_, k)| k.clone()).collect();

    let by_key: Vec<(String, i64)> = match system {
        CommentSystem::Twikoo { env_id } => {
            let url = parse(env_id)?;
            let body = json!({ "event": "GET_COMMENTS_COUNT", "urls": all, "includeReply": false });
            let v = http.post_json(&url, &body).await?;
            parse_twikoo(&v)
        }
        CommentSystem::Waline { server_url } => {
            let mut url = parse(&format!("{server_url}/api/comment"))?;
            url.query_pairs_mut()
                .append_pair("type", "count")
                .append_pair("url", &all.join(","))
                .append_pair("lang", "zh-CN");
            let v = get_json(http, &url).await?;
            all.iter().cloned().zip(parse_waline(&v)).collect()
        }
        CommentSystem::Artalk { server, site } => {
            let mut url = parse(&format!("{server}/api/v2/stats/page_comment"))?;
            url.query_pairs_mut().append_pair("page_keys", &all.join(","));
            if let Some(site) = site {
                url.query_pairs_mut().append_pair("site_name", site);
            }
            parse_artalk(&get_json(http, &url).await?)
        }
        CommentSystem::Halo => {
            let url = homepage
                .join("/apis/api.content.halo.run/v1alpha1/posts?page=1&size=50")
                .map_err(|_| bad_url())?;
            parse_halo(&get_json(http, &url).await?)
        }
        _ => Vec::new(),
    };

    let mut out: Vec<(i64, i64)> = Vec::new();
    for (id, key) in &keys {
        let trimmed = key.trim_end_matches('/');
        let hit = by_key
            .iter()
            .filter(|(k, _)| k == key || k.trim_end_matches('/') == trimmed)
            .map(|(_, c)| *c)
            .max();
        if let Some(c) = hit {
            match out.iter_mut().find(|(pid, _)| pid == id) {
                Some(e) => e.1 = e.1.max(c),
                None => out.push((*id, c)),
            }
        }
    }
    Ok(out)
}

fn bad_url() -> FetchError {
    FetchError::Status {
        status: reqwest::StatusCode::BAD_REQUEST,
        retry_after: None,
    }
}

fn parse(s: &str) -> Result<Url, FetchError> {
    Url::parse(s).map_err(|_| bad_url())
}

async fn get_json(http: &PoliteClient, url: &Url) -> Result<Value, FetchError> {
    match http.get(url, ACCEPT_JSON, None).await? {
        Fetched::Ok { body, .. } => Ok(serde_json::from_slice(&body).unwrap_or(Value::Null)),
        Fetched::NotModified { .. } => Ok(Value::Null),
    }
}

/// `{ data: [{ url, count }] }`, possibly wrapped in `{ result: ... }`.
fn parse_twikoo(v: &Value) -> Vec<(String, i64)> {
    let v = v.get("result").unwrap_or(v);
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e["url"].as_str()?.to_string(), e["count"].as_i64()?)))
        .collect()
}

/// v3: `{ errno, data: [n, ...] }`; v2: `[n, ...]`. In request order.
fn parse_waline(v: &Value) -> Vec<i64> {
    let arr = v.get("data").unwrap_or(v);
    match arr {
        Value::Array(a) => a.iter().map(|n| n.as_i64().unwrap_or(0)).collect(),
        Value::Number(n) => vec![n.as_i64().unwrap_or(0)],
        _ => Vec::new(),
    }
}

/// `{ data: { "/path": n } }`
fn parse_artalk(v: &Value) -> Vec<(String, i64)> {
    v["data"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, n)| Some((k.clone(), n.as_i64()?)))
        .collect()
}

/// Halo 2 public post list: `items[].status.permalink` and `items[].stats.comment`.
fn parse_halo(v: &Value) -> Vec<(String, i64)> {
    v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let link = p["status"]["permalink"].as_str()?;
            let path = Url::parse(link)
                .map(|u| u.path().to_string())
                .unwrap_or_else(|_| link.to_string());
            Some((path, p["stats"]["comment"].as_i64()?))
        })
        .collect()
}

// ---------- embedding ----------

/// What the frontend needs to load a blog's own comment widget for one post.
/// Only produced for blogs whose authors allowed embedding.
#[derive(Debug, Serialize)]
pub struct EmbedConfig {
    #[serde(flatten)]
    pub system: CommentSystem,
    /// Path key as the original page would compute it.
    pub path: String,
    /// Search term for giscus / utterances, following the blog's mapping.
    pub term: Option<String>,
    pub url: String,
    pub title: String,
}

pub fn embed_config(system: &CommentSystem, post_url: &Url, title: &str) -> Option<EmbedConfig> {
    if !system.embeddable() {
        return None;
    }
    let path = match system {
        // Waline keys comments by the decoded path.
        CommentSystem::Waline { .. } => percent_decode(post_url.path()),
        _ => post_url.path().to_string(),
    };
    let mapping = match system {
        CommentSystem::Giscus { mapping, .. } => Some(mapping.as_deref().unwrap_or("pathname")),
        CommentSystem::Utterances { issue_term, .. } => Some(issue_term.as_deref().unwrap_or("pathname")),
        _ => None,
    };
    let term = match mapping {
        None => None,
        Some("pathname") => Some(pathname_term(post_url.path())),
        Some("url") => Some(post_url.to_string()),
        Some("title" | "og:title") => Some(title.to_string()),
        // "specific" / "number" / custom terms cannot be derived from outside.
        Some(_) => return None,
    };
    Some(EmbedConfig {
        system: system.clone(),
        path,
        term,
        url: post_url.to_string(),
        title: title.to_string(),
    })
}

/// giscus / utterances "pathname" mapping: path without the leading slash or
/// a file extension, "index" for the root.
fn pathname_term(path: &str) -> String {
    if path.len() < 2 {
        return "index".into();
    }
    let p = &path[1..];
    match p.rfind('.') {
        Some(i) if !p[i + 1..].is_empty() && p[i + 1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            p[..i].to_string()
        }
        _ => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_waline_from_inline_init_with_escaped_slashes() {
        let html = r#"<div id="wcomments"></div><script>
            Waline.init({ el: '#wcomments', placeholder: '欢迎来留言~', serverURL: 'https:\/\/waline.example.top', pageview: true });
        </script>"#;
        assert_eq!(
            detect(html),
            Some(CommentSystem::Waline {
                server_url: "https://waline.example.top".into()
            })
        );
    }

    #[test]
    fn theme_json_with_all_options_picks_the_configured_one() {
        // Redefine-style theme config: every system listed, only one filled in.
        let html = r#"<script>window.theme={"comment":{"enable":true,"use":"waline",
            "valine":{"appid":null,"appkey":null,"server_urls":null},
            "twikoo":{"env_id":null,"region":null},
            "waline":{"server_url":"https://cot.example.cn","reaction":false},
            "giscus":{"repo":null}}}</script>"#;
        assert_eq!(
            detect(html),
            Some(CommentSystem::Waline {
                server_url: "https://cot.example.cn".into()
            })
        );
    }

    #[test]
    fn a_product_name_alone_is_not_detection() {
        let html = r#"<a href="/tag/twikoo/" class="label">twikoo</a> <a href="/tag/waline/">waline</a>"#;
        assert_eq!(detect(html), None);
        assert_eq!(
            native(Some("Typecho 1.2.1")),
            Some(CommentSystem::Native {
                generator: "Typecho".into()
            })
        );
        assert_eq!(native(Some("Hexo 7.3.0")), None);
    }

    #[test]
    fn detects_twikoo_artalk_giscus_halo_disqus() {
        let tw = r#"<script src="/js/twikoo.all.min.js"></script><script>twikoo.init({ envId: 'https://twikoo.example.app', el: '#tcomment' })</script>"#;
        assert_eq!(
            detect(tw),
            Some(CommentSystem::Twikoo {
                env_id: "https://twikoo.example.app".into()
            })
        );
        let tcb = r#"<script>twikoo.init({ envId: 'blog-1g2h3j4k', el: '#tcomment' })</script>"#;
        let sys = detect(tcb).unwrap();
        assert!(!sys.supports_counts(), "Tencent CloudBase env ids need the SDK");

        let at =
            r#"<script>Artalk.init({ el: '#Comments', server: 'https://artalk.example.com/', site: '小站' })</script>"#;
        assert_eq!(
            detect(at),
            Some(CommentSystem::Artalk {
                server: "https://artalk.example.com".into(),
                site: Some("小站".into())
            })
        );

        let gs = r#"<script src="https://giscus.app/client.js" data-repo="me/blog" data-repo-id="R_1" data-category="Announcements" data-category-id="DIC_2" data-mapping="pathname" async></script>"#;
        assert_eq!(
            detect(gs),
            Some(CommentSystem::Giscus {
                repo: "me/blog".into(),
                repo_id: Some("R_1".into()),
                category: Some("Announcements".into()),
                category_id: Some("DIC_2".into()),
                mapping: Some("pathname".into())
            })
        );

        let halo = r#"<comment-widget group="content.halo.run" kind="SinglePage" name="3adc" version="v1alpha1"></comment-widget>"#;
        assert_eq!(detect(halo), Some(CommentSystem::Halo));

        let dq = r#"<script>(function(){var s=document.createElement('script');s.src='https://myblog.disqus.com/embed.js';})()</script>"#;
        assert_eq!(
            detect(dq),
            Some(CommentSystem::Disqus {
                shortname: "myblog".into()
            })
        );
    }

    #[test]
    fn rss_comment_counts_align_with_entries() {
        let xml = br#"<rss><channel>
            <item><title>a</title><slash:comments>3</slash:comments></item>
            <item><title>b</title></item>
            <item><title>c</title><slash:comments>12</slash:comments></item>
        </channel></rss>"#;
        assert_eq!(feed_comment_counts(xml, 3), [Some(3), None, Some(12)]);
        assert!(feed_comment_counts(xml, 2).is_empty(), "mismatch means no counts");
        let atom = br#"<feed><entry><title>x</title><thr:total>5</thr:total></entry></feed>"#;
        assert_eq!(feed_comment_counts(atom, 1), [Some(5)]);
    }

    #[test]
    fn count_responses() {
        let tw = json!({ "result": { "code": 0, "data": [{ "url": "/p/1/", "count": 4 }] } });
        assert_eq!(parse_twikoo(&tw), [("/p/1/".to_string(), 4)]);
        assert_eq!(parse_waline(&json!({ "errno": 0, "data": [2, 0] })), [2, 0]);
        assert_eq!(parse_waline(&json!([1, 5])), [1, 5]);
        assert_eq!(parse_artalk(&json!({ "data": { "/a": 7 } })), [("/a".to_string(), 7)]);
        let halo = json!({ "items": [{ "status": { "permalink": "/archives/x" }, "stats": { "comment": 17 } }] });
        assert_eq!(parse_halo(&halo), [("/archives/x".to_string(), 17)]);
    }

    #[test]
    fn path_keys_include_decoded_form() {
        let u = Url::parse("https://b.example/posts/%E4%BD%A0%E5%A5%BD/").unwrap();
        assert_eq!(path_keys(&u), ["/posts/%E4%BD%A0%E5%A5%BD/", "/posts/你好/"]);
    }

    #[test]
    fn embed_terms_follow_mapping() {
        let url = Url::parse("https://b.example/2026/11/hello.html").unwrap();
        let gs = CommentSystem::Giscus {
            repo: "me/blog".into(),
            repo_id: None,
            category: None,
            category_id: None,
            mapping: Some("pathname".into()),
        };
        assert_eq!(
            embed_config(&gs, &url, "t").unwrap().term.as_deref(),
            Some("2026/11/hello")
        );
        let specific = CommentSystem::Giscus {
            repo: "me/blog".into(),
            repo_id: None,
            category: None,
            category_id: None,
            mapping: Some("specific".into()),
        };
        assert!(embed_config(&specific, &url, "t").is_none());
        assert!(
            embed_config(&CommentSystem::Halo, &url, "t").is_none(),
            "never load the blog's own scripts"
        );
    }
}
