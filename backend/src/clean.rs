//! Repairs for what feeds commonly get wrong, found by auditing the stored
//! posts: HTML escaped twice, raw Markdown in descriptions, "read more"
//! boilerplate, placeholder hosts (`http://example.com/...`), junk in author
//! fields, meaningless categories, and gambling spam on taken-over domains.

use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use crate::html;

fn re(s: &str) -> Regex {
    Regex::new(s).unwrap()
}

/// Closing tags that only appear in text when the feed escaped its HTML twice.
static ESCAPED_HTML: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)</(?:p|h[1-6]|li|ol|ul|a|strong|em|b|blockquote|span|figure|figcaption|table|section|article)>|<br\s*/?>")
});

/// Plain text for an excerpt, from a feed's description or content.
pub fn excerpt_text(raw: &str) -> String {
    let mut text = html::to_text(raw);
    if ESCAPED_HTML.is_match(&text) {
        text = html::to_text(&text);
    }
    let text = strip_markdown(&text);
    let text = strip_boilerplate(&text);
    let text = BARE_LINKS.replace_all(&text, " ");
    SPACES.replace_all(text.trim(), " ").into_owned()
}

/// Bare URLs and image file names left over from Markdown or image captions.
static BARE_LINKS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)https?://\S+|\S+\.(?:jpe?g|png|webp|gif|avif|svg)\b"));

static MD_IMAGE: LazyLock<Regex> = LazyLock::new(|| re(r"!\[[^\]]*\](?:\([^)\s]*\)?)?"));
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[([^\]]+)\]\((?:https?://|/|\.)[^)\s]*\)"));
static MD_HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"(^|\s)#{1,6}\s+"));
static MD_QUOTE: LazyLock<Regex> = LazyLock::new(|| re(r"(^|\s)>\s+"));
static MD_FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"```[\w-]*"));
static MD_EMPHASIS: LazyLock<Regex> = LazyLock::new(|| re(r"(^|\s)_([^_\s][^_]*?)_(\s|$)"));
static TEMPLATE_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"\{%.*?%\}"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s{2,}"));

/// Some feeds put the Markdown source in `<description>`.
pub fn strip_markdown(s: &str) -> String {
    let s = MD_IMAGE.replace_all(s, " ");
    let s = MD_LINK.replace_all(&s, "$1");
    let s = MD_HEADING.replace_all(&s, "$1");
    let s = if s.trim_start().starts_with("> ") {
        MD_QUOTE.replace_all(&s, "$1").into_owned()
    } else {
        s.into_owned()
    };
    let s = MD_FENCE.replace_all(&s, " ");
    let s = MD_EMPHASIS.replace_all(&s, "$1$2$3");
    let s = TEMPLATE_TAG.replace_all(&s, " ");
    let s = s.replace("**", "").replace('`', "");
    SPACES.replace_all(s.trim(), " ").into_owned()
}

static BOILERPLATE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // 访问 https://x/posts/a 阅读全文。 / ... 阅读全文 → / Continue reading »
        r"(?i)\s*(?:(?:访问|点击|前往)?\s*https?://\S+\s*)?(?:阅读全文|继续阅读|查看全文|阅读更多|阅读原文|read more|continue reading)\s*(?:[。.»→>…]|&raquo;)*\s*$",
        // WordPress: "The post X appeared first on Y."
        r"(?i)\s*the post .{1,200}? appeared first on .{1,100}?\.?\s*$",
        r"(?i)\s*[^。.!?！？]{0,200} first appeared on [^。]{1,100}?\.?\s*$",
        // WordPress zh: "X最先出现在Y。"
        r"\s*[^。.!?！？]{0,200}最先出现在[^。]{1,100}?。?\s*$",
    ]
    .iter()
    .map(|s| re(s))
    .collect()
});
static TRAILING_ELLIPSIS: LazyLock<Regex> = LazyLock::new(|| re(r"\s*\[(?:…|\.\.\.|&hellip;|&#8230;)\]\s*$"));

pub fn strip_boilerplate(s: &str) -> String {
    let mut s = s.trim().to_string();
    for r in BOILERPLATE.iter() {
        s = r.replace(&s, "").into_owned();
    }
    let s = TRAILING_ELLIPSIS.replace(&s, "…");
    s.trim().to_string()
}

/// `(Name)` from `email (Name)`, `@user Name` from forums, bare emails dropped.
pub fn author(raw: &str) -> Option<String> {
    let mut s = html::to_text(raw);
    if s.starts_with('(') && s.ends_with(')') {
        s = s[1..s.len() - 1].trim().to_string();
    }
    if let Some(rest) = s.strip_prefix('@') {
        s = match rest.split_once(' ') {
            Some((_, name)) if !name.trim().is_empty() => name.trim().to_string(),
            _ => rest.to_string(),
        };
    }
    let looks_like_email = s.contains('@') && !s.contains(' ');
    (!s.is_empty() && !looks_like_email && s.chars().count() <= 40).then_some(s)
}

/// Categories that say nothing about a post.
const NOISE_TAGS: &[&str] = &[
    "未分类",
    "未分類",
    "默认分类",
    "默认",
    "无分类",
    "其他",
    "其它",
    "文章",
    "分类",
    "标签",
    "uncategorized",
    "uncategorised",
    "default",
    "other",
    "others",
    "misc",
    "post",
    "posts",
    "none",
    "null",
    "undefined",
];

/// A feed category cleaned for display, or `None` when it carries no meaning.
pub fn tag(raw: &str) -> Option<String> {
    let t = html::to_text(raw);
    let t = t.trim().trim_start_matches('#').trim();
    let lower = t.to_lowercase();
    let keep = !t.is_empty() && t.chars().count() <= 24 && !NOISE_TAGS.contains(&lower.as_str());
    keep.then(|| t.to_string())
}

/// Hosts that mean "the generator was never configured" (Hexo's default `url`).
const PLACEHOLDER_HOSTS: &[&str] = &[
    "example.com",
    "www.example.com",
    "example.org",
    "example.net",
    "localhost",
    "127.0.0.1",
    "0.0.0.0",
    "yoursite.com",
    "yourdomain.com",
    "your-domain.com",
];

/// Moves a post URL on a placeholder host onto the feed's own host.
pub fn repair_url(url: Url, feed_url: &Url) -> Url {
    let placeholder = url.host_str().is_some_and(|h| PLACEHOLDER_HOSTS.contains(&h));
    if !placeholder || feed_url.host_str().is_some_and(|h| PLACEHOLDER_HOSTS.contains(&h)) {
        return url;
    }
    let mut fixed = feed_url.clone();
    fixed.set_path(url.path());
    fixed.set_query(url.query());
    fixed.set_fragment(url.fragment());
    fixed
}

/// SEO spam that fills expired domains (Vietnamese/Indonesian betting, Chinese gambling).
static SPAM: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)soi kèo|nhà cái|kèo nhà|tài xỉu|cá cược|nổ hũ|slot gacor|situs slot|judi (?:online|bola|slot)|togel|casino online|百家乐|博彩|赌场|六合彩|时时彩|体育投注|开云体育|澳门新葡京|威尼斯人娱乐|彩票平台",
    )
});

pub fn is_spam(text: &str) -> bool {
    SPAM.is_match(text)
}

/// Theme defaults that sites never replaced.
const PLACEHOLDER_DESCRIPTIONS: &[&str] = &[
    "site.seo.description",
    "your description here",
    "hexo theme redefine",
    "lorem ipsum",
    "just another wordpress site",
    "又一个wordpress站点",
    "a new hexo site",
    "this is my blog",
    "description",
];

/// A homepage `<meta name="description">`, cleaned: entities that were
/// escaped twice, stray markup, and theme placeholder text.
pub fn description(raw: &str) -> Option<String> {
    let mut s = html::to_text(raw);
    if s.contains('&') || s.contains('<') {
        s = html::to_text(&s);
    }
    let lower = s.to_lowercase();
    if s.is_empty() || PLACEHOLDER_DESCRIPTIONS.iter().any(|p| lower.starts_with(p)) {
        return None;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_escaped_html_is_unwrapped() {
        let raw = "&lt;p&gt;&lt;a href=&quot;https://g.example/x&quot;&gt;OneDrop&lt;/a&gt; 是一个工具。&lt;/p&gt;";
        assert_eq!(excerpt_text(raw), "OneDrop 是一个工具。");
        // Code that merely mentions a tag is left alone.
        assert_eq!(excerpt_text("用 &lt;WebView /&gt; 组件"), "用 <WebView /> 组件");
        assert_eq!(
            excerpt_text("optional&lt;T&amp;&gt; 的前世今生"),
            "optional<T&> 的前世今生"
        );
        assert_eq!(
            excerpt_text("俯瞰外高桥港.jpg https://fengc img.example/api/rfile/港口.jpg 飞临申城"),
            "飞临申城"
        );
    }

    #[test]
    fn markdown_is_flattened() {
        assert_eq!(
            strip_markdown("## 背景 Tailscale ![](/a.png) 见 [文档](https://t.example/d) 和 `tailscale up`"),
            "背景 Tailscale 见 文档 和 tailscale up"
        );
        assert_eq!(strip_markdown("> 引言： > 夫 OI 之道"), "引言： 夫 OI 之道");
        assert_eq!(strip_markdown("C# 和 a > b"), "C# 和 a > b");
        assert_eq!(strip_markdown("_原文：[破解](https://x.example/p)_"), "原文：破解");
    }

    #[test]
    fn boilerplate_tails_are_removed() {
        let cases = [
            (
                "题目链接见下 访问 https://r.example/posts/环形链表 阅读全文。",
                "题目链接见下",
            ),
            ("物竞... 阅读全文 →", "物竞..."),
            (
                "正式发售 。(其实部分地区29好就发售) 作为原版老玩家，最 […]",
                "正式发售 。(其实部分地区29好就发售) 作为原版老玩家，最…",
            ),
            ("正文。 The post 标题 appeared first on 某站.", "正文。"),
            ("正文。 rsync 同步与踩坑全记录 first appeared on 阿雷的小窝.", "正文。"),
            ("正文。白屏的问题最先出现在Ghost_chu's Blog。", "正文。"),
        ];
        for (input, want) in cases {
            assert_eq!(strip_boilerplate(input), want, "{input}");
        }
    }

    #[test]
    fn authors_and_tags_are_cleaned() {
        assert_eq!(author("(YeungYeah)").as_deref(), Some("YeungYeah"));
        assert_eq!(author("@xiaona 小纳").as_deref(), Some("小纳"));
        assert_eq!(author("@吴先生").as_deref(), Some("吴先生"));
        assert_eq!(author("me@blog.example"), None);
        assert_eq!(tag(" #Docker ").as_deref(), Some("Docker"));
        assert_eq!(tag("未分类"), None);
        assert_eq!(tag("Uncategorized"), None);
    }

    #[test]
    fn placeholder_hosts_move_to_the_feed_host() {
        let feed = Url::parse("https://blog.real.example/rss.xml").unwrap();
        let fixed = repair_url(Url::parse("http://example.com/post/a.html?x=1").unwrap(), &feed);
        assert_eq!(fixed.as_str(), "https://blog.real.example/post/a.html?x=1");
        let fine = Url::parse("https://other.example/p").unwrap();
        assert_eq!(repair_url(fine.clone(), &feed), fine);
    }

    #[test]
    fn spam_and_placeholder_descriptions() {
        assert!(is_spam("Soi kèo San Marino vs Albania ngày 30/09"));
        assert!(!is_spam("菠菜炒蛋的做法"));
        assert_eq!(
            description("Momo5418&amp;#39;s Blog").as_deref(),
            Some("Momo5418's Blog")
        );
        assert_eq!(description("site.seo.description"), None);
        assert_eq!(description("Hexo Theme Redefine, Redefine Your Hexo Journey."), None);
        assert_eq!(
            description("潘岳陆机赋何在？</br><i>有问题请邮件联系</i>").as_deref(),
            Some("潘岳陆机赋何在？ 有问题请邮件联系")
        );
    }
}
