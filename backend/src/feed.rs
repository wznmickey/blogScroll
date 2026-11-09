//! Turns a fetched RSS / Atom / JSON Feed document into post records.
//! Only data present in the feed itself is used.

use chrono::{DateTime, Utc};
use feed_rs::model::{Entry, Feed, MediaObject};
use url::Url;

use crate::html;
use crate::util::truncate_chars;

pub const EXCERPT_CHARS: usize = 240;

#[derive(Debug)]
pub struct ParsedFeed {
    pub icon: Option<Url>,
    /// RSS `<ttl>` in minutes: how long the publisher says the feed may be cached.
    pub ttl_minutes: Option<u32>,
    pub items: Vec<ParsedItem>,
}

#[derive(Debug, Clone)]
pub struct ParsedItem {
    pub guid: String,
    pub url: Url,
    pub title: String,
    pub excerpt: String,
    pub cover: Option<Url>,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub published: Option<DateTime<Utc>>,
    /// Raw HTML body from the feed (content, else summary). The crawler decides
    /// whether it may be stored, based on the blog's content policy.
    pub body_html: Option<String>,
}

pub fn parse(bytes: &[u8], feed_url: &Url) -> anyhow::Result<ParsedFeed> {
    let feed = feed_rs::parser::Builder::new()
        .base_uri(Some(feed_url.as_str()))
        .build()
        .parse(bytes)?;
    Ok(convert(feed, feed_url))
}

fn convert(feed: Feed, feed_url: &Url) -> ParsedFeed {
    let icon = feed
        .icon
        .as_ref()
        .or(feed.logo.as_ref())
        .and_then(|img| html::resolve(&img.uri, Some(feed_url)))
        .filter(|u| matches!(u.scheme(), "http" | "https"));
    let items = feed.entries.iter().filter_map(|e| convert_entry(e, feed_url)).collect();
    ParsedFeed {
        icon,
        ttl_minutes: feed.ttl,
        items,
    }
}

fn convert_entry(entry: &Entry, feed_url: &Url) -> Option<ParsedItem> {
    let link = entry
        .links
        .iter()
        .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .or(entry.links.first())?;
    let url = html::resolve(&link.href, Some(feed_url))?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }

    let content = entry
        .content
        .as_ref()
        .and_then(|c| c.body.clone())
        .filter(|s| !s.trim().is_empty());
    let summary = entry
        .summary
        .as_ref()
        .map(|t| t.content.clone())
        .filter(|s| !s.trim().is_empty());

    let excerpt_source = summary.as_deref().or(content.as_deref()).unwrap_or_default();
    let excerpt = truncate_chars(&html::to_text(excerpt_source), EXCERPT_CHARS);

    let title = entry
        .title
        .as_ref()
        .map(|t| html::to_text(&t.content))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| truncate_chars(&excerpt, 40));
    if title.is_empty() {
        return None;
    }

    let cover = media_image(&entry.media).or_else(|| {
        [content.as_deref(), summary.as_deref()]
            .into_iter()
            .flatten()
            .find_map(|h| html::first_image(h, Some(&url)))
    });

    let mut tags: Vec<String> = Vec::new();
    for c in &entry.categories {
        let t = c.label.as_deref().unwrap_or(&c.term).trim();
        if !t.is_empty() && t.chars().count() <= 24 && !tags.iter().any(|x| x == t) {
            tags.push(t.to_string());
        }
    }
    tags.truncate(6);

    let guid = if entry.id.trim().is_empty() {
        url.to_string()
    } else {
        entry.id.clone()
    };

    Some(ParsedItem {
        guid,
        url,
        title: truncate_chars(&title, 200),
        excerpt,
        cover,
        author: entry
            .authors
            .first()
            .and_then(|p| p.name.clone())
            .filter(|n| !n.trim().is_empty()),
        tags,
        published: entry.published.or(entry.updated),
        body_html: content.or(summary),
    })
}

/// Media RSS thumbnails / image enclosures.
fn media_image(media: &[MediaObject]) -> Option<Url> {
    let thumbs = media
        .iter()
        .flat_map(|m| &m.thumbnails)
        .filter_map(|t| Url::parse(&t.image.uri).ok());
    let images = media.iter().flat_map(|m| &m.content).filter_map(|c| {
        let is_image = c
            .content_type
            .as_ref()
            .is_some_and(|t| t.to_string().starts_with("image/"));
        if is_image { c.url.clone() } else { None }
    });
    thumbs.chain(images).find(html::plausible_cover)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
<channel>
  <title>小明的博客</title>
  <link>https://ming.example/</link>
  <ttl>60</ttl>
  <item>
    <title>第一篇 &amp; 测试</title>
    <link>/posts/1/</link>
    <guid>post-1</guid>
    <pubDate>Mon, 02 Nov 2026 10:00:00 +0800</pubDate>
    <category>生活</category>
    <category>旅行</category>
    <description><![CDATA[<p>今天去了<b>海边</b>。</p><img src="/img/sea.jpg" alt="">]]></description>
  </item>
  <item>
    <title>带缩略图</title>
    <link>https://ming.example/posts/2/</link>
    <media:content url="https://cdn.example/full.jpg" type="image/jpeg" medium="image"/>
    <media:thumbnail url="https://cdn.example/thumb.jpg"/>
    <description>短文</description>
  </item>
  <item>
    <title>图片附件</title>
    <link>https://ming.example/posts/3/</link>
    <enclosure url="https://cdn.example/enclosure.png" type="image/png" length="1234"/>
  </item>
  <item>
    <description>没有标题也没有链接的条目会被跳过</description>
  </item>
</channel>
</rss>"#;

    #[test]
    fn parses_rss_items() {
        let feed_url = Url::parse("https://ming.example/feed.xml").unwrap();
        let f = parse(RSS.as_bytes(), &feed_url).unwrap();
        assert_eq!(f.ttl_minutes, Some(60));
        assert_eq!(f.items.len(), 3);

        let a = &f.items[0];
        assert_eq!(a.guid, "post-1");
        assert_eq!(a.title, "第一篇 & 测试");
        assert_eq!(a.url.as_str(), "https://ming.example/posts/1/");
        assert_eq!(a.excerpt, "今天去了海边。");
        assert_eq!(a.cover.as_ref().unwrap().as_str(), "https://ming.example/img/sea.jpg");
        assert_eq!(a.tags, ["生活", "旅行"]);
        assert_eq!(a.published.unwrap().to_rfc3339(), "2026-11-02T02:00:00+00:00");

        let b = &f.items[1];
        assert_eq!(b.cover.as_ref().unwrap().as_str(), "https://cdn.example/thumb.jpg");
        assert!(b.published.is_none());

        let c = &f.items[2];
        assert_eq!(c.cover.as_ref().unwrap().as_str(), "https://cdn.example/enclosure.png");
        assert_eq!(c.excerpt, "");
    }

    #[test]
    fn parses_atom_with_content() {
        let atom = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Blog</title>
  <icon>/favicon.png</icon>
  <entry>
    <title>Hello</title>
    <id>tag:atom.example,2026:1</id>
    <link href="https://atom.example/hello" rel="alternate"/>
    <updated>2026-10-01T00:00:00Z</updated>
    <content type="html">&lt;p&gt;Body &lt;img src="pic.webp"&gt;&lt;/p&gt;</content>
  </entry>
</feed>"#;
        let feed_url = Url::parse("https://atom.example/atom.xml").unwrap();
        let f = parse(atom.as_bytes(), &feed_url).unwrap();
        assert_eq!(f.icon.unwrap().as_str(), "https://atom.example/favicon.png");
        let e = &f.items[0];
        assert_eq!(e.excerpt, "Body");
        assert_eq!(e.cover.as_ref().unwrap().as_str(), "https://atom.example/pic.webp");
        assert!(e.body_html.as_deref().unwrap().contains("<img"));
    }

    #[test]
    fn rejects_html_pages() {
        let url = Url::parse("https://x.example/").unwrap();
        assert!(parse(b"<!doctype html><html><body>hi</body></html>", &url).is_err());
    }
}
