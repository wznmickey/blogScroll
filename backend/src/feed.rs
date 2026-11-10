//! Turns a fetched RSS / Atom / JSON Feed document into post records.
//! Only data present in the feed itself is used.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, Utc};
use feed_rs::model::{Entry, Feed, LinkTarget, MediaObject};
use regex::Regex;
use url::Url;

use crate::clean;
use crate::html;
use crate::util::truncate_chars;

pub const EXCERPT_CHARS: usize = 240;
/// Bump when parsing changes, so stored posts are re-parsed once even if
/// their feed did not change (it is part of the crawler's content hash).
pub const PARSER_VERSION: u32 = 5;
/// Dates further ahead than this are typos (2077) or scheduled posts; ignored.
const FUTURE_SLACK_SECS: i64 = 2 * 86400;

#[derive(Debug)]
pub struct ParsedFeed {
    /// The feed's own title and site link; used to name sources added by URL.
    pub title: Option<String>,
    pub link: Option<Url>,
    pub icon: Option<Url>,
    /// RSS `<ttl>` in minutes: how long the publisher says the feed may be cached.
    pub ttl_minutes: Option<u32>,
    pub items: Vec<ParsedItem>,
    /// What the entries carry, for telling podcasts and video channels from blogs.
    pub audio_items: usize,
    pub video_items: usize,
}

impl ParsedFeed {
    /// Source kind the feed itself makes obvious, if any.
    pub fn detected_kind(&self) -> Option<&'static str> {
        let n = self.items.len().max(1);
        if self.audio_items * 2 >= n && self.audio_items > 0 {
            return Some("podcast");
        }
        if self.video_items * 2 >= n && self.video_items > 0 {
            return Some("video");
        }
        let forum_items = self.items.iter().filter(|i| FORUM_URL.is_match(i.url.path())).count();
        if forum_items >= 3 && forum_items * 2 >= n {
            return Some("forum");
        }
        let title = self.title.as_deref().unwrap_or_default().to_lowercase();
        if ["周刊", "weekly", "newsletter", "月刊"]
            .iter()
            .any(|w| title.contains(w))
        {
            return Some("newsletter");
        }
        None
    }

    /// Most entries are gambling spam: a domain that changed hands.
    pub fn looks_like_spam(&self) -> bool {
        let spam = self.items.iter().filter(|i| clean::is_spam(&i.title)).count();
        spam >= 3 && spam * 2 >= self.items.len()
    }
}

/// Topic URLs of common forum software: Discourse, Discuz!, Flarum, NodeBB.
static FORUM_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?:t/[^/]+/\d+|thread-\d+-|d/\d+|topic/\d+)").unwrap());

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
    /// Where comments live (RSS `<comments>`, Atom `rel="replies"`).
    pub comments_url: Option<Url>,
    /// From `<slash:comments>` / `<thr:total>`, when the feed carries it.
    pub comment_count: Option<i64>,
}

pub fn parse(bytes: &[u8], feed_url: &Url) -> anyhow::Result<ParsedFeed> {
    let feed = feed_rs::parser::Builder::new()
        .base_uri(Some(feed_url.as_str()))
        .build()
        .parse(bytes)?;
    let counts = crate::comments::feed_comment_counts(bytes, feed.entries.len());
    let dates = if feed
        .entries
        .iter()
        .any(|e| e.published.is_none() && e.updated.is_none())
    {
        loose_entry_dates(bytes, feed.entries.len())
    } else {
        Vec::new()
    };
    let mut parsed = convert(feed, feed_url, &counts, &dates);
    fix_dates(&mut parsed.items, Utc::now());
    dedupe(&mut parsed.items);
    Ok(parsed)
}

fn convert(
    feed: Feed,
    feed_url: &Url,
    comment_counts: &[Option<i64>],
    loose_dates: &[Option<DateTime<Utc>>],
) -> ParsedFeed {
    let icon = feed
        .icon
        .as_ref()
        .or(feed.logo.as_ref())
        .and_then(|img| html::resolve(&img.uri, Some(feed_url)))
        .filter(|u| matches!(u.scheme(), "http" | "https"));
    let has_media = |e: &Entry, prefix: &str| {
        e.media.iter().flat_map(|m| &m.content).any(|c| {
            c.content_type
                .as_ref()
                .is_some_and(|t| t.to_string().starts_with(prefix))
        })
    };
    let audio_items = feed.entries.iter().filter(|e| has_media(e, "audio/")).count();
    let video_items = feed.entries.iter().filter(|e| has_media(e, "video/")).count();
    let items = feed
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let mut item = convert_entry(e, feed_url)?;
            item.comment_count = comment_counts.get(i).copied().flatten();
            item.published = item.published.or(loose_dates.get(i).copied().flatten());
            Some(item)
        })
        .collect();
    let link = feed
        .links
        .iter()
        .filter(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .filter_map(|l| html::resolve(&l.href, Some(feed_url)))
        .find(|u| matches!(u.scheme(), "http" | "https") && u != feed_url);
    ParsedFeed {
        title: feed
            .title
            .as_ref()
            .map(|t| html::to_text(&t.content))
            .filter(|t| !t.is_empty()),
        link,
        icon,
        ttl_minutes: feed.ttl,
        items,
        audio_items,
        video_items,
    }
}

fn convert_entry(entry: &Entry, feed_url: &Url) -> Option<ParsedItem> {
    let link = entry
        .links
        .iter()
        .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .or(entry.links.first())?;
    let url = clean::repair_url(html::resolve(&link.href, Some(feed_url))?, feed_url);
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
    let excerpt = truncate_chars(&clean::excerpt_text(excerpt_source), EXCERPT_CHARS);

    let title = entry
        .title
        .as_ref()
        .map(|t| clean::strip_markdown(&html::to_text(&t.content)))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| truncate_chars(&excerpt, 40));
    // Unrendered templates (`{{ post.title }}`) from misconfigured generators.
    if title.is_empty() || (title.contains("{{") && title.contains("}}")) {
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
        if let Some(t) = clean::tag(c.label.as_deref().unwrap_or(&c.term))
            && !tags.iter().any(|x| x.eq_ignore_ascii_case(&t))
        {
            tags.push(t);
        }
    }
    tags.truncate(6);

    let guid = if entry.id.trim().is_empty() {
        url.to_string()
    } else {
        entry.id.clone()
    };

    let comments_url = entry
        .links
        .iter()
        .find(|l| {
            l.target == Some(LinkTarget::Comments)
                || (l.rel.as_deref() == Some("replies") && l.media_type.as_deref().is_none_or(|t| t == "text/html"))
        })
        .and_then(|l| html::resolve(&l.href, Some(&url)))
        .filter(|u| matches!(u.scheme(), "http" | "https"));

    Some(ParsedItem {
        guid,
        url,
        title: truncate_chars(&title, 200),
        excerpt,
        cover,
        author: entry
            .authors
            .first()
            .and_then(|p| p.name.as_deref())
            .and_then(clean::author),
        tags,
        published: entry.published.or(entry.updated),
        body_html: content.or(summary),
        comments_url,
        comment_count: None,
    })
}

/// Drops dates that cannot be true and fills gaps from the URL:
/// - far-future dates;
/// - one identical timestamp on most entries (generators that stamp every
///   entry with the build time, which made old posts look new on every rebuild).
///   Date-only stamps (midnight) are kept: a batch import on one day is plausible;
/// - entries without a date get one from `/2026/10/05/` or `/202610051011` URLs.
fn fix_dates(items: &mut [ParsedItem], now: DateTime<Utc>) {
    let limit = now.timestamp() + FUTURE_SLACK_SECS;
    for it in items.iter_mut() {
        if it.published.is_some_and(|d| d.timestamp() > limit) {
            it.published = None;
        }
    }
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for d in items.iter().filter_map(|i| i.published) {
        *counts.entry(d.timestamp()).or_default() += 1;
    }
    let dated = counts.values().sum::<usize>();
    if let Some((&ts, &n)) = counts.iter().max_by_key(|(_, n)| **n)
        && n >= 3
        && n * 2 >= dated
        && !is_midnight(ts)
    {
        for it in items.iter_mut() {
            if it.published.is_some_and(|d| d.timestamp() == ts) {
                it.published = None;
            }
        }
    }
    for it in items.iter_mut().filter(|i| i.published.is_none()) {
        it.published = url_date(&it.url).filter(|d| d.timestamp() <= limit);
    }
}

fn is_midnight(ts: i64) -> bool {
    [0, 8 * 3600].iter().any(|offset| (ts + offset).rem_euclid(86400) == 0)
}

static URL_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/((?:19|20)\d{2})[/-](\d{1,2})[/-](\d{1,2})(?:/|[-_.]|$)|/((?:19|20)\d{2})(\d{2})(\d{2})(\d{2})?(\d{2})?(?:/|[-_.]|$)").unwrap()
});

/// `/2026/10/05/slug`, `/2026-10-05-slug`, `/posts/202610051011` (UTC+8, like `loose_date`).
fn url_date(url: &Url) -> Option<DateTime<Utc>> {
    let c = URL_DATE.captures(url.path())?;
    let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
    let (y, m, d, hh, mm) = match num(1) {
        Some(y) => (y, num(2)?, num(3)?, 0, 0),
        None => (num(4)?, num(5)?, num(6)?, num(7).unwrap_or(0), num(8).unwrap_or(0)),
    };
    let date = NaiveDate::from_ymd_opt(y as i32, m, d)?;
    let naive = date.and_hms_opt(hh, mm, 0)?;
    let cst = FixedOffset::east_opt(8 * 3600)?;
    let dt = naive.and_local_timezone(cst).single()?.with_timezone(&Utc);
    (dt.year() >= 1995).then_some(dt)
}

static REPLY_ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^post_\d+$").unwrap());

/// One entry per page. Discourse feeds list every reply of a topic as its own
/// entry (`/t/topic/2851#post_3`). Other anchors are kept apart: microblog
/// pages give each entry one (`/moments/#moment-1a2b`).
/// Some blogs also publish one post under two URLs (a Chinese and an English
/// slug): same title and same excerpt. A weekly series shares only the title.
fn dedupe(items: &mut Vec<ParsedItem>) {
    let mut urls = HashSet::new();
    let mut texts = HashSet::new();
    items.retain(|it| {
        let mut key = it.url.clone();
        if key.fragment().is_some_and(|f| REPLY_ANCHOR.is_match(f)) {
            key.set_fragment(None);
        }
        urls.insert(key) && (it.excerpt.is_empty() || texts.insert((it.title.clone(), it.excerpt.clone())))
    });
}

static ENTRY_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(published|pubDate|dc:date|issued|updated|modified|lastBuildDate)>\s*([^<]{8,40}?)\s*</").unwrap()
});

/// Dates feed-rs rejects, such as `2026-10-05 10:11:00` (a space instead of
/// `T`, no time zone), read per entry in document order. Empty when the
/// entries cannot be matched up reliably.
fn loose_entry_dates(body: &[u8], entries: usize) -> Vec<Option<DateTime<Utc>>> {
    let text = String::from_utf8_lossy(body);
    let dates: Vec<Option<DateTime<Utc>>> = crate::comments::ITEM
        .find_iter(&text)
        .map(|m| {
            let mut found: Vec<(&str, &str)> = ENTRY_DATE
                .captures_iter(m.as_str())
                .map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str()))
                .collect();
            // Publication dates before modification dates.
            found.sort_by_key(|(tag, _)| matches!(*tag, "updated" | "modified" | "lastBuildDate"));
            found.into_iter().find_map(|(_, v)| loose_date(v))
        })
        .collect();
    if dates.len() == entries { dates } else { Vec::new() }
}

/// Webring blogs are overwhelmingly Chinese, so a date without a zone is read as UTC+8.
fn loose_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    for f in [
        "%Y-%m-%d %H:%M:%S%.f %z",
        "%Y-%m-%d %H:%M:%S %z",
        "%Y/%m/%d %H:%M:%S %z",
    ] {
        if let Ok(d) = DateTime::parse_from_str(s, f) {
            return Some(d.with_timezone(&Utc));
        }
    }
    let naive = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
    .or_else(|| {
        ["%Y-%m-%d", "%Y/%m/%d"]
            .iter()
            .find_map(|f| NaiveDate::parse_from_str(s, f).ok())
            .and_then(|d| d.and_hms_opt(0, 0, 0))
    })?;
    let cst = FixedOffset::east_opt(8 * 3600)?;
    naive.and_local_timezone(cst).single().map(|d| d.with_timezone(&Utc))
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
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/" xmlns:slash="http://purl.org/rss/1.0/modules/slash/">
<channel>
  <title>小明的博客</title>
  <link>https://ming.example/</link>
  <ttl>60</ttl>
  <item>
    <title>第一篇 &amp; 测试</title>
    <link>/posts/1/</link>
    <guid>post-1</guid>
    <comments>https://ming.example/posts/1/#comments</comments>
    <slash:comments>6</slash:comments>
    <pubDate>Mon, 02 Nov 2026 10:00:00 +0800</pubDate>
    <category>生活</category>
    <category>旅行</category>
    <description><![CDATA[<p>今天去了<b>海边</b>，**很开心**。</p><img src="/img/sea.jpg" alt="">]]></description>
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
  <item>
    <title>{{ post.title | xml_escape }}</title>
    <link>https://ming.example/{{ post.url }}</link>
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
        assert_eq!(a.excerpt, "今天去了海边，很开心。");
        assert_eq!(a.cover.as_ref().unwrap().as_str(), "https://ming.example/img/sea.jpg");
        assert_eq!(a.tags, ["生活", "旅行"]);
        assert_eq!(a.comment_count, Some(6));
        assert_eq!(
            a.comments_url.as_ref().unwrap().as_str(),
            "https://ming.example/posts/1/#comments"
        );
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
    fn reads_dates_feed_rs_rejects() {
        let atom = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>NanLin</title>
  <updated>2026-08-23 09:09:00</updated>
  <entry>
    <title>RSS 聚合</title>
    <link href="https://nl.example/posts/202610051011" />
    <id>https://nl.example/posts/202610051011</id>
    <updated>2026-10-05 10:11:00</updated>
  </entry>
  <entry>
    <title>Dated</title>
    <link href="https://nl.example/posts/b" />
    <id>b</id>
    <updated>2026-10-01T00:00:00Z</updated>
  </entry>
  <entry>
    <title>No date</title>
    <link href="https://nl.example/posts/c" />
    <id>c</id>
  </entry>
</feed>"#;
        let feed_url = Url::parse("https://nl.example/atom.xml").unwrap();
        let f = parse(atom.as_bytes(), &feed_url).unwrap();
        assert_eq!(f.items[0].published.unwrap().to_rfc3339(), "2026-10-05T02:11:00+00:00");
        assert_eq!(f.items[1].published.unwrap().to_rfc3339(), "2026-10-01T00:00:00+00:00");
        assert!(f.items[2].published.is_none());

        assert_eq!(
            loose_date("2026-10-05 10:11:00 +0000").unwrap().to_rfc3339(),
            "2026-10-05T10:11:00+00:00"
        );
        assert_eq!(
            loose_date("2026/10/05").unwrap().to_rfc3339(),
            "2026-10-04T16:00:00+00:00"
        );
        assert!(loose_date("yesterday").is_none());
    }

    fn item(url: &str, published: Option<i64>) -> ParsedItem {
        ParsedItem {
            guid: url.into(),
            url: Url::parse(url).unwrap(),
            title: "t".into(),
            excerpt: String::new(),
            cover: None,
            author: None,
            tags: vec![],
            published: published.and_then(|t| DateTime::from_timestamp(t, 0)),
            body_html: None,
            comments_url: None,
            comment_count: None,
        }
    }

    #[test]
    fn build_time_stamped_on_every_entry_is_dropped() {
        let now = DateTime::from_timestamp(1_794_240_674 + 3600, 0).unwrap();
        let build = 1_794_240_674; // 16:11:14 UTC
        let mut items: Vec<ParsedItem> = (0..5)
            .map(|i| item(&format!("https://p.example/posts/{i}"), Some(build)))
            .collect();
        items.push(item("https://p.example/2026/10/05/dated-by-url/", Some(build)));
        fix_dates(&mut items, now);
        assert!(items[..5].iter().all(|i| i.published.is_none()));
        assert_eq!(items[5].published.unwrap().to_rfc3339(), "2026-10-04T16:00:00+00:00");

        // A batch imported on one day (date-only stamps) is believable.
        let midnight_cst = 1_773_331_200; // 2026-03-13 00:00 +08
        let mut batch: Vec<ParsedItem> = (0..4)
            .map(|i| item(&format!("https://p.example/{i}"), Some(midnight_cst)))
            .collect();
        fix_dates(&mut batch, now);
        assert!(batch.iter().all(|i| i.published.is_some()));

        let mut future = vec![item("https://p.example/a", Some(3_382_000_000))];
        fix_dates(&mut future, now);
        assert!(future[0].published.is_none(), "2077 is not a publication date");
    }

    #[test]
    fn dates_from_urls() {
        let d = |u: &str| url_date(&Url::parse(u).unwrap()).map(|d| d.to_rfc3339());
        assert_eq!(
            d("https://n.example/posts/202610051011").as_deref(),
            Some("2026-10-05T02:11:00+00:00")
        );
        assert_eq!(
            d("https://n.example/2026/4/18/x/").as_deref(),
            Some("2026-04-17T16:00:00+00:00")
        );
        assert_eq!(
            d("https://n.example/2026-09-26-mcclient.html").as_deref(),
            Some("2026-09-25T16:00:00+00:00")
        );
        assert_eq!(d("https://n.example/archives/1776582851949"), None);
        assert_eq!(d("https://n.example/posts/37cb61b3"), None);
        assert_eq!(d("https://n.example/2026/13/40/"), None);
    }

    #[test]
    fn forum_replies_collapse_into_their_topic() {
        let mut items = vec![
            item("https://f.example/t/topic/2851#post_4", None),
            item("https://f.example/t/topic/2851#post_3", None),
            item("https://f.example/#/post/1", None),
            item("https://f.example/#/post/2", None),
        ];
        // Same title and excerpt under another URL is the same post; an empty
        // excerpt says nothing (a weekly series shares its title).
        items[2].excerpt = "同一篇".into();
        items[3].excerpt = "同一篇".into();
        let mut weekly = item("https://f.example/weekly/2", None);
        weekly.title = items[0].title.clone();
        items.push(weekly);
        items.push(item("https://f.example/moments/#moment-1a2b", None));
        items.push(item("https://f.example/moments/#moment-3c4d", None));
        dedupe(&mut items);
        let urls: Vec<&str> = items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://f.example/t/topic/2851#post_4",
                "https://f.example/#/post/1",
                "https://f.example/weekly/2",
                "https://f.example/moments/#moment-1a2b",
                "https://f.example/moments/#moment-3c4d"
            ]
        );
    }

    #[test]
    fn rejects_html_pages() {
        let url = Url::parse("https://x.example/").unwrap();
        assert!(parse(b"<!doctype html><html><body>hi</body></html>", &url).is_err());
    }
}
