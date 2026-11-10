//! A blog's public profile: what its homepage says about itself (meta
//! description, avatar, links to an about page and social accounts) plus a
//! short excerpt of the about page. Refreshed about once a month.

use std::sync::LazyLock;

use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::html;
use crate::util::truncate_chars;

pub const DESCRIPTION_CHARS: usize = 200;
pub const ABOUT_CHARS: usize = 300;
const MAX_SOCIAL: usize = 16;
/// Authors often have more than one account on a platform (a main and an
/// English X account, a personal and a project GitHub).
const MAX_PER_KIND: usize = 3;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct HomeInfo {
    pub description: Option<String>,
    pub avatar: Option<Url>,
    pub about: Option<Url>,
    pub social: Vec<SocialLink>,
    pub generator: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocialLink {
    pub kind: String,
    pub url: String,
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).unwrap()
}

fn meta(doc: &Html, names: &[&str]) -> Option<String> {
    let s = sel("meta[content]");
    names.iter().find_map(|n| {
        doc.select(&s)
            .find(|m| {
                let v = m.value();
                v.attr("name")
                    .or(v.attr("property"))
                    .is_some_and(|a| a.eq_ignore_ascii_case(n))
            })
            .and_then(|m| m.value().attr("content"))
            .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|c| !c.is_empty())
    })
}

pub fn from_home(page: &str, base: &Url) -> HomeInfo {
    let doc = Html::parse_document(page);
    HomeInfo {
        description: meta(&doc, &["description", "og:description", "twitter:description"])
            .and_then(|d| crate::clean::description(&d))
            .map(|d| truncate_chars(&redact(&d), DESCRIPTION_CHARS)),
        avatar: avatar(&doc, base),
        about: about_link(&doc, base),
        social: social_links(&doc, base),
        generator: meta(&doc, &["generator"]).map(|g| truncate_chars(&g, 40)),
    }
}

/// The author's picture as common themes render it, else the touch icon.
/// Avatars elsewhere on a page often belong to commenters, so only
/// header/sidebar author slots are trusted.
fn avatar(doc: &Html, base: &Url) -> Option<Url> {
    const SLOTS: &[&str] = &[
        ".avatar-img img",         // Butterfly
        ".author-info-avatar img", // AnZhiYu
        "img.author-info__avatar", //
        ".site-author-image",      // NexT
        ".author-avatar img",      //
        ".sidebar img.avatar",     //
        "header img.avatar",       //
        "#avatar img",             //
        "img.site-avatar",         //
        ".profile-avatar img",     //
    ];
    let img = |e: ElementRef| -> Option<Url> {
        let v = e.value();
        let src = v.attr("data-lazy-src").or(v.attr("data-src")).or(v.attr("src"))?;
        html::resolve(src.trim(), Some(base)).filter(|u| matches!(u.scheme(), "http" | "https"))
    };
    for s in SLOTS {
        if let Some(u) = doc.select(&sel(s)).find_map(img) {
            return Some(u);
        }
    }
    let icon = |rel: &str| {
        doc.select(&sel("link[rel][href]"))
            .filter(|l| {
                l.value()
                    .attr("rel")
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .split_whitespace()
                    .any(|r| r == rel)
            })
            .filter_map(|l| html::resolve(l.value().attr("href")?, Some(base)))
            .next()
    };
    icon("apple-touch-icon")
}

const ABOUT_WORDS: &[&str] = &[
    "关于",
    "关于我",
    "关于本站",
    "关于博主",
    "关于博客",
    "about",
    "about me",
    "aboutme",
];

fn about_link(doc: &Html, base: &Url) -> Option<Url> {
    let host = base.host_str()?;
    let candidates: Vec<(Url, bool)> = doc
        .select(&sel("a[href]"))
        .filter_map(|a| {
            let url = html::resolve(a.value().attr("href")?, Some(base))?;
            if url.host_str() != Some(host) {
                return None;
            }
            // "#about" opens a menu on this page; it is not a page.
            let mut page = url.clone();
            page.set_fragment(None);
            if page.path() == base.path() && page.query() == base.query() {
                return None;
            }
            let text: String = a.text().collect::<String>().trim().to_lowercase();
            let title = a.value().attr("title").unwrap_or_default().trim().to_lowercase();
            let last = url
                .path_segments()?
                .rev()
                .find(|s| !s.is_empty() && *s != "index.html")
                .unwrap_or_default()
                .trim_end_matches(".html")
                .to_lowercase();
            let by_path = matches!(
                last.as_str(),
                "about" | "about-me" | "about_me" | "aboutme" | "%e5%85%b3%e4%ba%8e" | "%e5%85%b3%e4%ba%8e%e6%88%91"
            );
            let exact = ABOUT_WORDS.contains(&text.as_str()) || ABOUT_WORDS.contains(&title.as_str());
            // "📝 关于", "关于我 | About me": nothing but about-words once
            // emoji, spaces and punctuation are gone ("最近关于大模型的探索"
            // is a post title, not the about page).
            let letters: String = text.chars().filter(|c| c.is_alphanumeric()).collect();
            let rest = [
                "关于我们",
                "关于本站",
                "关于博主",
                "关于博客",
                "关于我",
                "关于",
                "aboutme",
                "about",
                "me",
            ]
            .iter()
            .fold(letters.clone(), |s, w| s.replace(w, ""));
            let loose = !letters.is_empty() && rest.is_empty() && letters != "me";
            (by_path || exact || loose).then_some((url, by_path || exact))
        })
        .collect();
    // A sure match first, then a loose one.
    candidates
        .iter()
        .find(|(_, sure)| *sure)
        .or(candidates.first())
        .map(|(u, _)| u.clone())
}

/// Social links on any page (the homepage or the about page), to be merged:
/// each with whether it may be a platform's second account.
pub fn social_from_page(page: &str, base: &Url) -> Vec<(SocialLink, bool)> {
    social_candidates(&Html::parse_document(page), base)
}

/// Adds accounts not yet present: a platform's first from anywhere, more (up
/// to [`MAX_PER_KIND`]) only from the author's own lists of links.
pub fn merge_social(into: &mut Vec<SocialLink>, more: Vec<(SocialLink, bool)>) {
    for (s, extra_ok) in more {
        let key = account_key(&s.url);
        let same_kind = into.iter().filter(|x| x.kind == s.kind).count();
        if into.len() < MAX_SOCIAL
            && !into.iter().any(|x| account_key(&x.url) == key)
            && (same_kind == 0 || (extra_ok && same_kind < MAX_PER_KIND))
        {
            into.push(s);
        }
    }
}

/// One account however it is written: twitter.com or x.com, with or without
/// "www.", a trailing slash, a query, or different letter case.
fn account_key(url: &str) -> String {
    let Ok(u) = Url::parse(url) else {
        return url.to_ascii_lowercase();
    };
    let host = u.host_str().unwrap_or_default().trim_start_matches("www.");
    let host = match host {
        "twitter.com" => "x.com",
        "telegram.me" => "t.me",
        "bangumi.tv" | "chii.in" => "bgm.tv",
        h => h,
    };
    let mut key = format!("{host}{}", u.path().trim_end_matches('/'));
    // NetEase Music keeps the account in the fragment: #/user/home?id=…
    if host == "music.163.com" {
        key.push_str(u.fragment().unwrap_or_default());
    }
    key.to_ascii_lowercase()
}

/// Where a link may be someone else's account: in a footer (theme credits),
/// comments or a friend list, or mentioned in a sentence ("a student of
/// @SomeUniversity").
fn others_context(el: &ElementRef) -> bool {
    in_others_section(el) || in_prose(el)
}

fn in_others_section(el: &ElementRef) -> bool {
    el.ancestors().filter_map(ElementRef::wrap).any(|a| {
        let v = a.value();
        if v.name() == "footer" {
            return true;
        }
        let marks = format!(
            "{} {}",
            v.attr("class").unwrap_or_default(),
            v.attr("id").unwrap_or_default()
        )
        .to_ascii_lowercase();
        [
            "footer",
            "copyright",
            "powered",
            "comment",
            "friend",
            "flink",
            "blogroll",
        ]
        .iter()
        .any(|m| marks.contains(m))
    })
}

/// The link's block (paragraph, list item, ...) has text of its own besides
/// links: a list of profile links has next to none.
fn in_prose(el: &ElementRef) -> bool {
    const BLOCKS: &[&str] = &[
        "p",
        "li",
        "dd",
        "td",
        "blockquote",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "div",
        "section",
        "article",
    ];
    let Some(block) = el
        .ancestors()
        .filter_map(ElementRef::wrap)
        .find(|a| BLOCKS.contains(&a.value().name()))
    else {
        return false;
    };
    let is_link =
        |e: &scraper::node::Element| e.name() == "a" || e.attr("data-url").is_some() || e.attr("data-href").is_some();
    let own_text: usize = block
        .descendants()
        .filter_map(|n| n.value().as_text().map(|t| (n, t)))
        .filter(|(n, _)| {
            !n.ancestors()
                .take_while(|a| a.id() != block.id())
                .filter_map(ElementRef::wrap)
                .any(|a| is_link(a.value()))
        })
        .map(|(_, t)| t.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    own_text > 30
}

/// Profile links: rel="me" anywhere (also `<link>` in the head), and
/// account-shaped URLs on well-known sites, also behind a theme's "you are
/// leaving this site" redirect, in elements a script turns into links
/// (NexT's `<span class="exturl" data-url="<base64>">`), or in a theme's
/// social settings embedded in a script (`"github": "https://github.com/…"`).
fn social_links(doc: &Html, base: &Url) -> Vec<SocialLink> {
    let mut out = Vec::new();
    merge_social(&mut out, social_candidates(doc, base));
    out
}

/// Every distinct account on the page, and whether it may be a platform's
/// second one.
fn social_candidates(doc: &Html, base: &Url) -> Vec<(SocialLink, bool)> {
    let mut found: Vec<(SocialLink, bool)> = Vec::new();
    let mut add = |kind: String, url: &Url, extra_ok: bool| {
        let key = account_key(url.as_str());
        match found.iter_mut().find(|(s, _)| account_key(&s.url) == key) {
            Some((_, ok)) => *ok |= extra_ok,
            None => found.push((
                SocialLink {
                    kind,
                    url: url.to_string(),
                },
                extra_ok,
            )),
        }
    };
    for a in doc.select(&sel("a[href], link[rel][href]")) {
        let Some(url) = html::resolve(a.value().attr("href").unwrap_or_default(), Some(base)) else {
            continue;
        };
        let url = unwrap_redirect(&url, base).unwrap_or(url);
        // Webrings and blog directories are not the author's accounts.
        if url.host_str().is_some_and(|h| WEBRINGS.iter().any(|w| h.ends_with(w))) {
            continue;
        }
        let rel_me = a
            .value()
            .attr("rel")
            .unwrap_or_default()
            .split_whitespace()
            .any(|r| r.eq_ignore_ascii_case("me"));
        // From the head only rel="me" counts (not stylesheets, feeds, ...).
        if a.value().name() == "link" && !rel_me {
            continue;
        }
        let kind = social_kind(&url).or_else(|| (rel_me && url.host_str() != base.host_str()).then(|| "link".into()));
        if let Some(kind) = kind {
            add(kind, &url, rel_me || !others_context(&a));
        }
    }
    for el in doc.select(&sel("[data-url], [data-href]")) {
        let v = el.value();
        let raw = v.attr("data-url").or(v.attr("data-href")).unwrap_or_default().trim();
        let text = if raw.starts_with("http") {
            raw.to_string()
        } else {
            match base64_decode(raw).and_then(|b| String::from_utf8(b).ok()) {
                Some(t) => t,
                None => continue,
            }
        };
        if let Ok(url) = Url::parse(&text)
            && let Some(kind) = social_kind(&url)
        {
            add(kind, &url, !others_context(&el));
        }
    }
    for script in doc.select(&sel("script")) {
        let text: String = script.text().collect();
        for c in SCRIPT_SOCIAL.captures_iter(&text) {
            let raw = c[1].replace("\\/", "/");
            // A theme's social settings are the author's own.
            if let Ok(url) = Url::parse(&raw)
                && let Some(kind) = social_kind(&url)
            {
                add(kind, &url, true);
            }
        }
    }
    // A page linking many accounts on one platform is listing other people:
    // only the first of those counts, as for links in footers and comments.
    let crowded: Vec<bool> = found
        .iter()
        .map(|(s, _)| found.iter().filter(|(x, ok)| *ok && x.kind == s.kind).count() > MAX_PER_KIND)
        .collect();
    found
        .into_iter()
        .zip(crowded)
        .map(|((s, ok), crowded)| (s, ok && !crowded))
        .collect()
}

/// Themes' social settings in scripts: a platform name as the key and a URL
/// as the value. Friend lists use "link"/"url" keys, so they do not match.
static SCRIPT_SOCIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)["'](?:github|gitee|gitlab|bilibili|weibo|zhihu|twitter|x|telegram|instagram|youtube|douban|xiaohongshu|steam|pixiv|bangumi|juejin|csdn|v2ex|mastodon|bluesky|linkedin|facebook|netease|neteasemusic|music|coolapk|douyin)["']\s*:\s*["'](https?:(?:\\?/){2}[^"'\s]+)["']"#,
    )
    .unwrap()
});

/// Themes send outbound links through a "you are leaving" page on their own
/// site: `/go.html?u=<base64>`, `/link?url=<url>`, ...
fn unwrap_redirect(url: &Url, base: &Url) -> Option<Url> {
    if url.host_str() != base.host_str() {
        return None;
    }
    url.query_pairs().find_map(|(k, v)| {
        if !matches!(
            k.as_ref(),
            "u" | "url" | "target" | "link" | "to" | "goto" | "redirect" | "href"
        ) {
            return None;
        }
        let v = v.trim();
        let text = if v.starts_with("http") {
            v.to_string()
        } else {
            String::from_utf8(base64_decode(v)?).ok()?
        };
        Url::parse(&text)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
    })
}

/// Standard or URL-safe base64, padding optional.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut bits: u32 = 0;
    let mut n = 0;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes().take_while(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    Some(out)
}

const WEBRINGS: &[&str] = &["travellings.cn", "travellings.link", "foreverblog.cn", "boyouquan.com"];

fn social_kind(url: &Url) -> Option<String> {
    let host = url.host_str()?.trim_start_matches("www.");
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    // A file (an avatar, a badge) is not an account.
    if segs.last().is_some_and(|s| s.contains('.')) {
        return None;
    }
    let kind = match (host, segs.as_slice()) {
        ("github.com", [_]) => "github",
        ("gitee.com", [_]) => "gitee",
        ("gitlab.com" | "codeberg.org", [_]) => "gitlab",
        ("space.bilibili.com", [_id, ..]) => "bilibili",
        ("m.bilibili.com", ["space", _, ..]) => "bilibili",
        ("weibo.com", [_, ..]) | ("m.weibo.cn" | "weibo.cn", ["u" | "profile", _, ..]) => "weibo",
        ("zhihu.com", ["people", _, ..]) => "zhihu",
        ("twitter.com" | "x.com", [_]) => "x",
        ("t.me" | "telegram.me", [_]) => "telegram",
        ("instagram.com", [_]) => "instagram",
        ("youtube.com", [ch, ..]) if ch.starts_with('@') || matches!(*ch, "channel" | "c" | "user") => "youtube",
        ("douban.com", ["people", _, ..]) => "douban",
        ("xiaohongshu.com", ["user", "profile", _]) => "xiaohongshu",
        ("douyin.com", ["user", _]) => "douyin",
        ("steamcommunity.com", ["id" | "profiles", _, ..]) => "steam",
        ("pixiv.net", ["users", _] | [_, "users", _]) => "pixiv",
        ("bgm.tv" | "bangumi.tv" | "chii.in", ["user", _, ..]) => "bangumi",
        ("juejin.cn", ["user", _, ..]) => "juejin",
        ("blog.csdn.net", [_]) => "csdn",
        ("v2ex.com", ["member", _]) => "v2ex",
        ("coolapk.com", ["u", _]) => "coolapk",
        ("bsky.app", ["profile", _]) => "bluesky",
        ("linkedin.com", ["in", _, ..]) => "linkedin",
        ("facebook.com", [_]) => "facebook",
        ("threads.net", [u]) if u.starts_with('@') => "threads",
        ("twitch.tv", [_]) => "twitch",
        ("music.163.com", _)
            if url
                .fragment()
                .is_some_and(|f| f.trim_start_matches('/').starts_with("user/home")) =>
        {
            "netease_music"
        }
        // Mastodon, Misskey and the like: https://<instance>/@<name>.
        (_, [u]) if u.starts_with('@') && u.len() > 1 && host != "medium.com" => "fediverse",
        _ => return None,
    };
    // Account pages only: not github.com/features, x.com/intent and the like.
    const NOT_ACCOUNTS: &[&str] = &[
        "features",
        "about",
        "pricing",
        "login",
        "topics",
        "explore",
        "intent",
        "share",
        "home",
        "search",
        "settings",
        "sponsors",
        "marketplace",
        "orgs",
        "apps",
        "watch",
        "hashtag",
        "i",
    ];
    if segs.len() == 1 && NOT_ACCOUNTS.contains(&segs[0]) {
        return None;
    }
    Some(kind.to_string())
}

/// A short plain-text excerpt of an about page's main content: the first
/// content container (same theme selectors as full-text extraction) with
/// enough real text once page chrome is removed.
pub fn about_text(page: &str) -> Option<String> {
    let doc = Html::parse_document(page);
    let candidates = crate::fulltext::SELECTORS
        .iter()
        .chain(&[".content", "#content", "body"]);
    let mut fallback = None;
    for s in candidates {
        for el in doc.select(&sel(s)) {
            let text = redact(&content_text(&el.inner_html()));
            if text.chars().count() >= 60 {
                return Some(truncate_chars(&text, ABOUT_CHARS));
            }
            if fallback.is_none() && !text.is_empty() {
                fallback = Some(text);
            }
        }
    }
    fallback.map(|t| truncate_chars(&t, ABOUT_CHARS))
}

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+").unwrap());

/// Addresses are public on the author's own page, but we do not republish
/// them in bulk; readers can follow the link to the original about page.
fn redact(text: &str) -> String {
    EMAIL.replace_all(text, "[邮箱见原页]").into_owned()
}

/// Visible text without page chrome: menus, headers/footers, sidebars, tables
/// of contents, and comment sections (other people's words are not the
/// author's introduction).
fn content_text(fragment: &str) -> String {
    fn chrome(e: &scraper::node::Element) -> bool {
        if matches!(
            e.name(),
            "nav"
                | "header"
                | "footer"
                | "aside"
                | "script"
                | "style"
                | "noscript"
                | "template"
                | "svg"
                | "form"
                | "button"
        ) {
            return true;
        }
        let marks = format!(
            "{} {}",
            e.attr("class").unwrap_or_default(),
            e.attr("id").unwrap_or_default()
        )
        .to_ascii_lowercase();
        [
            "nav",
            "menu",
            "toc",
            "sidebar",
            "breadcrumb",
            "comment",
            "footer",
            "header",
            "share",
            "copyright",
        ]
        .iter()
        .any(|m| marks.contains(m))
    }
    fn walk(el: ElementRef, out: &mut String) {
        for child in el.children() {
            match child.value() {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) if !chrome(e) => {
                    if let Some(c) = ElementRef::wrap(child) {
                        out.push(' ');
                        walk(c, out);
                        out.push(' ');
                    }
                }
                _ => {}
            }
        }
    }
    let doc = Html::parse_fragment(fragment);
    let mut out = String::new();
    walk(doc.root_element(), &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = r#"<!doctype html><html><head>
        <meta name="generator" content="Hexo 7.3.0">
        <meta name="description" content="  记录生活、学习，
            以及自己琢磨的一些有趣的东西。 ">
        <link rel="apple-touch-icon" href="/img/apple.png">
        </head><body>
        <nav><a href="/">首页</a><a href="/archives/">归档</a><a href="/about/index.html">关于</a></nav>
        <aside><div class="avatar-img"><img src="data:," data-lazy-src="/img/me.webp"></div>
          <a href="https://github.com/someone" rel="me noopener">GitHub</a>
          <a href="https://github.com/features/actions">not a profile</a>
          <a href="https://space.bilibili.com/12345">B站</a>
          <a href="https://mastodon.example/@me" rel="me">Mastodon</a>
          <a href="https://friend.example/">友链</a>
        </aside></body></html>"#;

    #[test]
    fn reads_homepage_profile() {
        let base = Url::parse("https://blog.example/").unwrap();
        let h = from_home(HOME, &base);
        assert_eq!(
            h.description.as_deref(),
            Some("记录生活、学习， 以及自己琢磨的一些有趣的东西。")
        );
        assert_eq!(h.avatar.unwrap().as_str(), "https://blog.example/img/me.webp");
        assert_eq!(h.about.unwrap().as_str(), "https://blog.example/about/index.html");
        assert_eq!(h.generator.as_deref(), Some("Hexo 7.3.0"));
        let kinds: Vec<&str> = h.social.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["github", "bilibili", "fediverse"],
            "an /@name page is a fediverse account"
        );
    }

    #[test]
    fn about_link_by_text_or_path_and_falls_back_to_touch_icon() {
        let base = Url::parse("https://b.example/").unwrap();
        let page = r#"<head><link rel="apple-touch-icon" href="/t.png"></head>
            <a href="https://other.example/about">别人的关于</a><a href="/s/about">关于本站</a>"#;
        let h = from_home(page, &base);
        assert_eq!(h.about.unwrap().as_str(), "https://b.example/s/about");
        assert_eq!(h.avatar.unwrap().as_str(), "https://b.example/t.png");
    }

    #[test]
    fn about_excerpt_skips_menus_toc_and_comments() {
        let intro = "我是一名高中生，喜欢写代码和拍照。".repeat(15);
        let page = format!(
            r#"<html><body><main>
              <div class="menus_items"><a href="/">首页</a><a href="/archives">归档</a></div>
              <div id="toc"><span>文章目录</span><a href="/x">关于我</a></div>
              <article><p>{intro}</p>
                <div class="comment-list"><p>访客：写得真好！</p></div>
              </article>
            </main></body></html>"#
        );
        let t = about_text(&page).unwrap();
        assert!(t.starts_with("我是一名高中生"), "{t}");
        assert!(!t.contains("首页") && !t.contains("文章目录") && !t.contains("访客"));
    }

    #[test]
    fn emails_are_redacted() {
        let page = format!(
            "<article><p>{}联系我：me.blog+x@example.co.uk 或 a@b.cn</p></article>",
            "介绍".repeat(40)
        );
        let t = about_text(&page).unwrap();
        assert!(!t.contains('@'), "{t}");
        assert!(t.contains("[邮箱见原页]"));
    }

    #[test]
    fn more_platforms_redirects_scripts_and_link_rel_me() {
        let base = Url::parse("https://b.example/").unwrap();
        // /go.html?u=base64("https://bgm.tv/user/someone")
        let go = format!("/go.html?u={}", "aHR0cHM6Ly9iZ20udHYvdXNlci9zb21lb25l");
        let page = format!(
            r#"<head><link rel="me" href="https://mastodon.social/@someone"><link rel="stylesheet" href="https://github.com/x"></head>
            <a href="{go}">Bangumi</a>
            <a href="https://music.163.com/#/user/home?id=445441609">网易云</a>
            <a href="https://www.pixiv.net/users/61761697">pixiv</a>
            <a href="https://github.com/someone.png">avatar, not an account</a>
            <a href="https://x.com/intent/tweet?text=hi">share, not an account</a>
            <script>window.KEEP = {{"links":{{"github":"https:\/\/github.com\/kawashiro-ryofu","x":"https://x.com/bkryofu","qq":null}},
              "friends":[{{"name":"a friend","link":"https://github.com/afriend"}}]}}</script>"#
        );
        let kinds: Vec<(String, String)> = social_from_page(&page, &base)
            .into_iter()
            .map(|(s, _)| (s.kind, s.url))
            .collect();
        assert_eq!(
            kinds,
            [
                ("fediverse".into(), "https://mastodon.social/@someone".into()),
                ("bangumi".into(), "https://bgm.tv/user/someone".into()),
                (
                    "netease_music".into(),
                    "https://music.163.com/#/user/home?id=445441609".into()
                ),
                ("pixiv".into(), "https://www.pixiv.net/users/61761697".into()),
                ("github".into(), "https://github.com/kawashiro-ryofu".into()),
                ("x".into(), "https://x.com/bkryofu".into()),
            ] as [(String, String); 6]
        );
    }

    #[test]
    fn next_theme_exturl_spans() {
        let base = Url::parse("https://b.example/").unwrap();
        // NexT with `exturl: true`: base64("https://github.com/wznmickey") etc.
        let page = r#"<span class="links-of-author-item">
            <span class="exturl" data-url="aHR0cHM6Ly9naXRodWIuY29tL3d6bm1pY2tleQ==" title="GitHub"><i class="fab fa-github"></i></span>
            <span class="exturl" data-url="aHR0cHM6Ly93d3cubGlua2VkaW4uY29tL2luL3d6bm1pY2tleS8="><i class="fab fa-linkedin"></i></span>
            <span data-url="not base64 at all!"></span></span>"#;
        let got: Vec<(String, String)> = social_from_page(page, &base)
            .into_iter()
            .map(|(s, _)| (s.kind, s.url))
            .collect();
        assert_eq!(
            got,
            [
                ("github".into(), "https://github.com/wznmickey".into()),
                ("linkedin".into(), "https://www.linkedin.com/in/wznmickey/".into()),
            ] as [(String, String); 2]
        );
    }

    #[test]
    fn about_page_links_are_added_to_the_homes() {
        let mut social = vec![SocialLink {
            kind: "github".into(),
            url: "https://github.com/a".into(),
        }];
        let base = Url::parse("https://b.example/about/").unwrap();
        let about = r#"<p>My lab is <a href="https://github.com/lab">the lab</a>, where we build robots and other things.</p>
            <a href="https://github.com/b">GitHub</a><a href="https://space.bilibili.com/1">B站</a>"#;
        merge_social(&mut social, social_from_page(about, &base));
        let urls: Vec<&str> = social.iter().map(|s| s.url.as_str()).collect();
        // The lab is mentioned in a sentence; on its own page it would be the
        // first GitHub link, but next to the homepage's it is not another account.
        assert_eq!(
            urls,
            [
                "https://github.com/a",
                "https://github.com/b",
                "https://space.bilibili.com/1"
            ]
        );
    }

    #[test]
    fn loose_about_labels() {
        let base = Url::parse("https://b.example/").unwrap();
        let h = from_home(
            r##"<a href="#about">关于</a><a href="/archives/138">最近关于大模型的探索历程</a><a href="/me/">📝 关于我 | About me</a>"##,
            &base,
        );
        assert_eq!(h.about.unwrap().as_str(), "https://b.example/me/");
        let h = from_home(r#"<a href="/x/">关于 About</a><a href="/about/">个人介绍</a>"#, &base);
        assert_eq!(
            h.about.unwrap().as_str(),
            "https://b.example/about/",
            "a sure match wins"
        );
    }

    #[test]
    fn webrings_are_not_social_links() {
        let base = Url::parse("https://b.example/").unwrap();
        let h = from_home(
            r#"<a rel="me" href="https://www.travellings.cn/go.html">开往</a>"#,
            &base,
        );
        assert!(h.social.is_empty());
    }

    #[test]
    fn several_accounts_on_one_platform() {
        let base = Url::parse("https://b.example/").unwrap();
        let h = from_home(
            r#"<div class="links-of-author">
                 <a href="https://twitter.com/me">Twitter</a>
                 <a href="https://twitter.com/meEn">Twitter in English</a>
                 <a href="https://x.com/Me/">X again</a>
               </div>
               <p>A student of <a href="https://github.com/uni">@uni</a>. I'm currently learning Japanese.</p>
               <a href="https://github.com/me">GitHub</a>
               <p>I write about <a href="https://github.com/project">my project</a> here, and much more besides that.</p>
               <footer>Theme by <a href="https://github.com/themeauthor">someone</a></footer>"#,
            &base,
        );
        let urls: Vec<&str> = h.social.iter().map(|s| s.url.as_str()).collect();
        // The same account written another way is not a second one; a
        // mention in a sentence or a theme credit is someone else's, though
        // the first GitHub link (in a sentence) is still taken, as before.
        assert_eq!(
            urls,
            [
                "https://twitter.com/me",
                "https://twitter.com/meEn",
                "https://github.com/uni",
                "https://github.com/me"
            ]
        );

        let many: String = (0..6)
            .map(|i| format!(r#"<li><a href="https://github.com/friend{i}">friend{i}</a></li>"#))
            .collect();
        let h = from_home(&format!("<ul>{many}</ul>"), &base);
        assert_eq!(h.social.len(), 1, "a long list is other people: {:?}", h.social);

        let mut merged = h.social.clone();
        let github = |u: &str| SocialLink {
            kind: "github".into(),
            url: u.into(),
        };
        merge_social(
            &mut merged,
            vec![
                (github("https://github.com/Friend0"), true),
                (github("https://github.com/second"), true),
                (github("https://github.com/mentioned"), false),
            ],
        );
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn about_excerpt() {
        let body = "我是一名学生，喜欢折腾服务器和写字。".repeat(20);
        let page = format!("<html><body><nav>菜单</nav><article><h1>关于</h1><p>{body}</p></article></body></html>");
        let t = about_text(&page).unwrap();
        assert!(t.starts_with("关于 我是一名学生"));
        assert!(t.chars().count() <= ABOUT_CHARS + 1);
        assert!(!t.contains("菜单"));
    }
}
