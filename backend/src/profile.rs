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
    doc.select(&sel("a[href]")).find_map(|a| {
        let url = html::resolve(a.value().attr("href")?, Some(base))?;
        if url.host_str() != Some(host) {
            return None;
        }
        let text: String = a.text().collect::<String>().trim().to_lowercase();
        let last = url
            .path_segments()?
            .rev()
            .find(|s| !s.is_empty() && *s != "index.html")
            .unwrap_or_default()
            .trim_end_matches(".html")
            .to_lowercase();
        let by_path = matches!(last.as_str(), "about" | "about-me" | "aboutme" | "%e5%85%b3%e4%ba%8e");
        (by_path || ABOUT_WORDS.contains(&text.as_str())).then_some(url)
    })
}

/// Profile links: rel="me" anywhere, plus account-shaped URLs on well-known sites.
fn social_links(doc: &Html, base: &Url) -> Vec<SocialLink> {
    let mut out: Vec<SocialLink> = Vec::new();
    for a in doc.select(&sel("a[href]")) {
        let Some(url) = html::resolve(a.value().attr("href").unwrap_or_default(), Some(base)) else {
            continue;
        };
        // Webrings and blog directories are not the author's accounts.
        if url.host_str().is_some_and(|h| WEBRINGS.iter().any(|w| h.ends_with(w))) {
            continue;
        }
        let rel_me = a
            .value()
            .attr("rel")
            .unwrap_or_default()
            .split_whitespace()
            .any(|r| r == "me");
        let kind = social_kind(&url).or_else(|| (rel_me && url.host_str() != base.host_str()).then(|| "link".into()));
        if let Some(kind) = kind
            && !out.iter().any(|s| s.kind == kind)
        {
            out.push(SocialLink {
                kind,
                url: url.to_string(),
            });
        }
        if out.len() >= 6 {
            break;
        }
    }
    out
}

const WEBRINGS: &[&str] = &["travellings.cn", "travellings.link", "foreverblog.cn", "boyouquan.com"];

fn social_kind(url: &Url) -> Option<String> {
    let host = url.host_str()?.trim_start_matches("www.");
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let kind = match (host, segs.as_slice()) {
        ("github.com", [_]) => "github",
        ("gitee.com", [user]) if !user.is_empty() => "gitee",
        ("space.bilibili.com", [_id, ..]) => "bilibili",
        ("weibo.com", [_, ..]) => "weibo",
        ("zhihu.com", ["people", _, ..]) => "zhihu",
        ("twitter.com" | "x.com", [_]) => "x",
        ("t.me", [_]) => "telegram",
        ("instagram.com", [_]) => "instagram",
        ("youtube.com", [ch, ..]) if ch.starts_with('@') || *ch == "channel" => "youtube",
        ("douban.com", ["people", _, ..]) => "douban",
        ("xiaohongshu.com", ["user", "profile", _]) => "xiaohongshu",
        _ => return None,
    };
    // github.com/<user> only; skip github.com/features etc.
    if kind == "github" && ["features", "about", "pricing", "login", "topics", "explore"].contains(&segs[0]) {
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
        assert_eq!(kinds, ["github", "bilibili", "link"]);
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
    fn webrings_are_not_social_links() {
        let base = Url::parse("https://b.example/").unwrap();
        let h = from_home(
            r#"<a rel="me" href="https://www.travellings.cn/go.html">开往</a>"#,
            &base,
        );
        assert!(h.social.is_empty());
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
