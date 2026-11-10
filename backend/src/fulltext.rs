//! Full-text fetching: an extension point that is off unless an author opts in.
//!
//! By default only what the RSS feed contains is used, and only an excerpt of
//! it is stored. For a blog listed under `policy.full_text` in the config
//! (i.e. its author gave permission), the crawler:
//!   1. stores the feed's own full content if it has one, otherwise
//!   2. asks a [`FullTextExtractor`] to fetch the article page, a few posts per crawl.
//!
//! Plug in a better extractor (readability-style, per-site rules, an author-provided
//! API...) by implementing the trait and passing it to `Crawler::new`.

use futures::FutureExt;
use futures::future::BoxFuture;
use scraper::{Html, Selector};
use url::Url;

use crate::html;
use crate::http::{ACCEPT_HTML, Fetched, PoliteClient};

pub trait FullTextExtractor: Send + Sync {
    /// Returns sanitised article HTML, or `None` if no article body was found.
    fn extract<'a>(&'a self, http: &'a PoliteClient, article: &'a Url)
    -> BoxFuture<'a, anyhow::Result<Option<String>>>;
}

/// Picks the main content element by selectors used by common blog themes.
pub struct ArticleExtractor;

/// Container selectors, most specific first (Butterfly, Hexo/NexT, WordPress, Hugo, ...).
pub(crate) const SELECTORS: &[&str] = &[
    "[itemprop=articleBody]",
    "#article-container",
    ".post-content",
    ".entry-content",
    ".article-content",
    ".post-body",
    ".markdown-body",
    "article",
    "main",
];
const MIN_TEXT_CHARS: usize = 200;

impl ArticleExtractor {
    pub fn extract_from_html(page: &str, base: &Url) -> Option<String> {
        main_content(page, base, MIN_TEXT_CHARS)
    }
}

/// The page's main content container (by the selectors above) with at least
/// `min_chars` of text, sanitised.
pub fn main_content(page: &str, base: &Url, min_chars: usize) -> Option<String> {
    let doc = Html::parse_document(page);
    for sel in SELECTORS {
        let sel = Selector::parse(sel).unwrap();
        for el in doc.select(&sel) {
            let inner = el.inner_html();
            if html::to_text(&inner).chars().count() >= min_chars {
                return Some(html::sanitize(&inner, Some(base)));
            }
        }
    }
    None
}

impl FullTextExtractor for ArticleExtractor {
    fn extract<'a>(
        &'a self,
        http: &'a PoliteClient,
        article: &'a Url,
    ) -> BoxFuture<'a, anyhow::Result<Option<String>>> {
        async move {
            match http.get(article, ACCEPT_HTML, None).await? {
                Fetched::Ok { body, final_url, .. } => {
                    Ok(Self::extract_from_html(&String::from_utf8_lossy(&body), &final_url))
                }
                Fetched::NotModified { .. } => Ok(None),
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_article_body() {
        let body = "正文内容。".repeat(60);
        let page = format!(
            r#"<html><body><nav>菜单</nav><article><h1>标题</h1><div class="post-content"><p>{body}</p><img src="a.png"></div></article><footer>页脚</footer></body></html>"#
        );
        let base = Url::parse("https://b.example/p/1/").unwrap();
        let out = ArticleExtractor::extract_from_html(&page, &base).unwrap();
        assert!(out.contains("正文内容"));
        assert!(!out.contains("菜单") && !out.contains("页脚"));
        assert!(out.contains("https://b.example/p/1/a.png"));
    }

    #[test]
    fn short_pages_yield_nothing() {
        let base = Url::parse("https://b.example/").unwrap();
        assert!(ArticleExtractor::extract_from_html("<article>短</article>", &base).is_none());
    }
}
