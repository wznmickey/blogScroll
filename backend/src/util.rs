use std::hash::{BuildHasher, RandomState};
use std::time::{SystemTime, UNIX_EPOCH};

use url::Url;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Identity of a site, used to deduplicate blogs listed by several directories
/// and to match policy entries: lowercase host without `www.`, plus the path
/// without trailing slash. Scheme, port, query and fragment are ignored.
pub fn site_key(input: &str) -> Option<String> {
    let input = input.trim();
    let url = if input.contains("://") {
        Url::parse(input).ok()?
    } else {
        Url::parse(&format!("https://{input}")).ok()?
    };
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let path = url.path().trim_end_matches('/');
    let path = path
        .strip_suffix("/index.html")
        .or_else(|| path.strip_suffix("/index.htm"))
        .unwrap_or(path);
    Some(format!("{host}{path}"))
}

/// Parses a user-supplied homepage, defaulting to https and requiring http(s).
pub fn parse_http_url(input: &str) -> Option<Url> {
    let input = input.trim();
    let url = if input.contains("://") {
        Url::parse(input).ok()?
    } else {
        Url::parse(&format!("https://{input}")).ok()?
    };
    let ok = matches!(url.scheme(), "http" | "https") && url.host_str().is_some();
    ok.then_some(url)
}

/// Truncates to at most `max` characters (not bytes), adding an ellipsis.
pub fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((idx, _)) => format!("{}…", s[..idx].trim_end()),
    }
}

/// Spreads `secs` by ±`percent`% so that feeds added together do not stay in lockstep.
pub fn jitter(secs: i64, percent: i64, salt: i64) -> i64 {
    let span = secs * percent / 100;
    if span <= 0 {
        return secs;
    }
    let r = RandomState::new().hash_one((salt, now())) as i64;
    secs + r.rem_euclid(2 * span + 1) - span
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_key_normalises() {
        assert_eq!(site_key("https://www.Example.com/").unwrap(), "example.com");
        assert_eq!(site_key("http://example.com").unwrap(), "example.com");
        assert_eq!(site_key("example.com/blog/").unwrap(), "example.com/blog");
        assert_eq!(site_key("https://a.github.io/index.html").unwrap(), "a.github.io");
        assert_eq!(site_key("https://example.com:8443/?x=1#y").unwrap(), "example.com");
        assert!(site_key("").is_none());
    }

    #[test]
    fn truncate_counts_chars() {
        assert_eq!(truncate_chars("你好世界", 2), "你好…");
        assert_eq!(truncate_chars("abc", 3), "abc");
    }

    #[test]
    fn jitter_stays_in_range() {
        for i in 0..100 {
            let v = jitter(1000, 10, i);
            assert!((900..=1100).contains(&v));
        }
    }
}
