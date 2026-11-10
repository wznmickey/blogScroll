//! Small HTML helpers. They only ever look at markup we already have
//! (feed content, or a homepage fetched once for feed discovery).

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

/// Feed generators that drop line breaks (Hexo's, for one) glue a tag's name
/// to an attribute written on the next line: `<img\nsrc="…">` comes out as
/// `<imgsrc="…">`, which no parser reads as an image. Puts the space back.
pub fn unglue_attributes(html: &str) -> Cow<'_, str> {
    static GLUED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)<(img|a|source|video|audio|iframe|span|div|p)(src|href|alt|class|id|width|height|title|style|srcset|loading|data-[a-z-]+)=",
        )
        .unwrap()
    });
    GLUED.replace_all(html, "<$1 $2=")
}

/// Visible text of an HTML fragment with whitespace collapsed.
/// Code blocks, scripts and styles are skipped; they make terrible excerpts.
pub fn to_text(html: &str) -> String {
    let doc = Html::parse_fragment(html);
    let mut out = String::new();
    collect_text(doc.root_element(), &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text(el: ElementRef, out: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) => {
                if matches!(e.name(), "script" | "style" | "noscript" | "pre" | "template" | "svg") {
                    continue;
                }
                if let Some(child_el) = ElementRef::wrap(child) {
                    let block = matches!(
                        e.name(),
                        "p" | "div"
                            | "br"
                            | "li"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "blockquote"
                            | "tr"
                            | "section"
                            | "article"
                    );
                    if block {
                        out.push(' ');
                    }
                    collect_text(child_el, out);
                    if block {
                        out.push(' ');
                    }
                }
            }
            _ => {}
        }
    }
}

/// Fragments in image URLs that almost always mean "not a cover picture".
const IMAGE_DENYLIST: &[&str] = &[
    "gravatar.com",
    "/smilies/",
    "/emoji",
    "emoji/",
    "/emotion/",
    "/face/",
    "feeds.feedburner.com",
    "feedburner.google",
    "stats.wordpress.com",
    "pixel.wp.com",
    "shields.io",
    "badge",
    "/avatar",
    "1x1",
    "spacer.gif",
    "blank.gif",
    "loading.gif",
    "loading.svg",
    "/wp-includes/images/",
];

pub fn plausible_cover(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let s = url.as_str().to_ascii_lowercase();
    if IMAGE_DENYLIST.iter().any(|d| s.contains(d)) {
        return false;
    }
    !url.path().to_ascii_lowercase().ends_with(".svg")
}

/// First image in an HTML fragment that looks like real content.
/// Handles common lazy-loading attributes used by Chinese blog themes.
pub fn first_image(html: &str, base: Option<&Url>) -> Option<Url> {
    let doc = Html::parse_fragment(html);
    let sel = Selector::parse("img").unwrap();
    for img in doc.select(&sel) {
        let attr = |n: &str| img.value().attr(n).map(str::trim).filter(|s| !s.is_empty());
        let tiny = |n: &str| {
            attr(n)
                .and_then(|v| v.trim_end_matches("px").parse::<u32>().ok())
                .is_some_and(|v| v <= 48)
        };
        if tiny("width") || tiny("height") {
            continue;
        }
        let candidates = [
            attr("data-src"),
            attr("data-original"),
            attr("data-lazy-src"),
            attr("data-actualsrc"),
            attr("data-lazyload"),
            attr("src"),
            attr("srcset")
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.split_whitespace().next()),
        ];
        for raw in candidates.into_iter().flatten() {
            if raw.starts_with("data:") {
                continue;
            }
            if let Some(url) = resolve(raw, base).filter(plausible_cover) {
                return Some(url);
            }
        }
    }
    None
}

pub fn resolve(raw: &str, base: Option<&Url>) -> Option<Url> {
    match base {
        Some(b) => b.join(raw).ok(),
        None => Url::parse(raw).ok(),
    }
}

/// Feed URLs advertised by a homepage via `<link rel="alternate">`,
/// best candidates first. Comment feeds are dropped.
pub fn feed_links(html: &str, base: &Url) -> Vec<Url> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("link[rel][href]").unwrap();
    let mut found: Vec<(u8, Url)> = Vec::new();
    for link in doc.select(&sel) {
        let el = link.value();
        let rel = el.attr("rel").unwrap_or_default().to_ascii_lowercase();
        if !rel.split_whitespace().any(|r| r == "alternate") {
            continue;
        }
        let ty = el.attr("type").unwrap_or_default().to_ascii_lowercase();
        let rank = match ty.trim() {
            "application/atom+xml" => 0,
            "application/rss+xml" => 0,
            "application/feed+json" | "application/json" => 1,
            "application/rdf+xml" | "application/xml" | "text/xml" => 2,
            _ => continue,
        };
        let title = el.attr("title").unwrap_or_default().to_lowercase();
        let href = el.attr("href").unwrap_or_default();
        if title.contains("comment") || title.contains("评论") || href.contains("comments") {
            continue;
        }
        if let Ok(url) = base.join(href)
            && matches!(url.scheme(), "http" | "https")
            && !found.iter().any(|(_, u)| *u == url)
        {
            found.push((rank, url));
        }
    }
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, u)| u).collect()
}

/// Sanitises HTML for display on our page. Used only for full-text content
/// that an author has allowed us to show.
pub fn sanitize(html: &str, base: Option<&Url>) -> String {
    let mut b = ammonia::Builder::default();
    b.link_rel(Some("noopener noreferrer nofollow"))
        .set_tag_attribute_value("a", "target", "_blank")
        .add_tag_attributes("img", &["loading"])
        .set_tag_attribute_value("img", "referrerpolicy", "no-referrer")
        .set_tag_attribute_value("img", "loading", "lazy");
    if let Some(base) = base {
        b.url_relative(ammonia::UrlRelative::RewriteWithBase(base.clone()));
    }
    b.clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_skips_code_and_collapses_space() {
        let html = "<p>Hello <b>world</b></p><pre>let x = 1;</pre><p>第二段\n\n文字</p><script>x()</script>";
        assert_eq!(to_text(html), "Hello world 第二段 文字");
    }

    #[test]
    fn first_image_prefers_lazy_attrs_and_skips_junk() {
        let base = Url::parse("https://blog.example/posts/a.html").unwrap();
        let html = r#"
            <img src="https://blog.example/wp-includes/images/smilies/smile.png">
            <img src="/tiny.png" width="1" height="1">
            <img src="data:image/gif;base64,AAAA" data-src="/img/cover.jpg">
        "#;
        assert_eq!(
            first_image(html, Some(&base)).unwrap().as_str(),
            "https://blog.example/img/cover.jpg"
        );
        assert!(first_image("<p>no images</p>", Some(&base)).is_none());
    }

    #[test]
    fn feed_links_are_found_and_ranked() {
        let base = Url::parse("https://blog.example/").unwrap();
        let html = r#"<html><head>
            <link rel="alternate" type="application/rss+xml" title="评论 Feed" href="/comments/feed">
            <link rel="alternate" type="application/feed+json" href="/feed.json">
            <link rel="alternate" type="application/atom+xml" href="atom.xml">
            <link rel="stylesheet" href="/style.css">
        </head></html>"#;
        let links: Vec<String> = feed_links(html, &base).into_iter().map(String::from).collect();
        assert_eq!(
            links,
            ["https://blog.example/atom.xml", "https://blog.example/feed.json"]
        );
    }

    #[test]
    fn sanitize_strips_scripts_and_fixes_links() {
        let base = Url::parse("https://blog.example/p/1").unwrap();
        let out = sanitize(
            r#"<p onclick="x()">hi<script>bad()</script><img src="/a.png"></p>"#,
            Some(&base),
        );
        assert!(!out.contains("script") && !out.contains("onclick"));
        assert!(out.contains(r#"src="https://blog.example/a.png""#));
        assert!(out.contains(r#"referrerpolicy="no-referrer""#));
    }
}
