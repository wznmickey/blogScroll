//! Pages for search engines in the static export. The built `index.html` is
//! filled in per route (home, the blog directory, about, each blog) with a
//! title, description, canonical URL, Open Graph tags, and the content itself
//! as plain HTML, so crawlers that do not run JavaScript see what the page
//! is about; the app replaces that content when it starts. Posts get no pages
//! of their own: lists link to the originals, so a search leads to the author.

use serde_json::Value;

use crate::util::truncate_chars;

/// Marks a filled-in page, so a later export does not use it as the template.
pub const MARKER: &str = "<!-- prerendered by feed-server export -->";

pub struct Site<'a> {
    pub name: &'a str,
    /// Absolute, without a trailing slash: https://you.github.io/blogScroll
    pub url: &'a str,
    /// The app's built index.html.
    pub template: &'a str,
}

pub struct Page {
    /// "" for the home page, "blogs", "blog/12", ...
    pub path: String,
    pub title: String,
    pub description: String,
    pub image: Option<String>,
    /// HTML for inside `<div id="app">`.
    pub body: String,
}

impl Site<'_> {
    fn link(&self, path: &str) -> String {
        if path.is_empty() {
            format!("{}/", self.url)
        } else {
            format!("{}/{path}", self.url)
        }
    }
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The template with this page's head and content; `None` if the template
/// is not the app's index.html (no `<div id="app">`).
pub fn render(site: &Site, page: &Page) -> Option<String> {
    const APP: &str = r#"<div id="app"></div>"#;
    if !site.template.contains(APP) {
        return None;
    }
    let url = site.link(&page.path);
    let mut head = format!(
        "{MARKER}\n    <meta name=\"description\" content=\"{d}\" />\n    <link rel=\"canonical\" href=\"{u}\" />\n    \
         <meta property=\"og:site_name\" content=\"{s}\" />\n    <meta property=\"og:title\" content=\"{t}\" />\n    \
         <meta property=\"og:description\" content=\"{d}\" />\n    <meta property=\"og:url\" content=\"{u}\" />\n    \
         <meta property=\"og:type\" content=\"website\" />\n",
        d = esc(&page.description),
        u = esc(&url),
        s = esc(site.name),
        t = esc(&page.title),
    );
    if let Some(img) = &page.image {
        head.push_str(&format!(
            "    <meta property=\"og:image\" content=\"{}\" />\n",
            esc(img)
        ));
    }
    let html = replace_title(site.template, &page.title);
    let html = remove_description(&html).replacen("</head>", &format!("{head}  </head>"), 1);
    Some(html.replacen(APP, &format!("<div id=\"app\">{}</div>", page.body), 1))
}

fn replace_title(html: &str, title: &str) -> String {
    match (html.find("<title>"), html.find("</title>")) {
        (Some(a), Some(b)) if a < b => format!("{}<title>{}</title>{}", &html[..a], esc(title), &html[b + 8..]),
        _ => html.replacen("</head>", &format!("<title>{}</title></head>", esc(title)), 1),
    }
}

/// The template's own description goes; each page brings its own.
fn remove_description(html: &str) -> String {
    let Some(start) = html.find("<meta name=\"description\"") else {
        return html.to_string();
    };
    let Some(len) = html[start..].find('>') else {
        return html.to_string();
    };
    format!(
        "{}{}",
        &html[..start],
        html[start + len + 1..].trim_start_matches(['\n', ' '])
    )
}

/// The template for routes without a page (search, 我的): not to be indexed.
pub fn not_found(site: &Site) -> String {
    site.template.replacen(
        "</head>",
        "  <meta name=\"robots\" content=\"noindex\" />\n  </head>",
        1,
    )
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

fn date(secs: Option<i64>) -> Option<String> {
    chrono::DateTime::from_timestamp(secs?, 0).map(|d| d.format("%Y-%m-%d").to_string())
}

/// A post as a list item: the title links to the original article.
fn post_item(site: &Site, p: &Value, with_blog: bool) -> String {
    let mut out = format!(
        "<li><article><h3><a href=\"{}\" rel=\"noopener\">{}</a></h3><p>",
        esc(s(p, "url")),
        esc(s(p, "title"))
    );
    if with_blog {
        out.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            esc(&site.link(&format!("blog/{}", p["blog"]["id"]))),
            esc(s(&p["blog"], "name"))
        ));
    }
    let published = if p["undated"].as_bool() == Some(true) {
        None
    } else {
        p["published_at"].as_i64().or(p["sort_at"].as_i64())
    };
    if let Some(d) = date(published) {
        out.push_str(&format!(
            "{}<time datetime=\"{d}\">{d}</time>",
            if with_blog { " · " } else { "" }
        ));
    }
    out.push_str("</p>");
    let excerpt = s(p, "excerpt");
    if !excerpt.is_empty() {
        out.push_str(&format!("<p>{}</p>", esc(&truncate_chars(excerpt, 140))));
    }
    out.push_str("</article></li>");
    out
}

fn nav(site: &Site) -> String {
    format!(
        "<nav><a href=\"{}\">首页</a> · <a href=\"{}\">全部博客</a> · <a href=\"{}\">关于</a></nav>",
        esc(&site.link("")),
        esc(&site.link("blogs")),
        esc(&site.link("about"))
    )
}

pub fn home(site: &Site, meta: &Value, posts: &[Value]) -> Page {
    let blogs = meta["stats"]["blogs"].as_i64().unwrap_or(0);
    let description =
        format!("收录 {blogs} 个独立博客的 RSS，用瀑布流浏览它们的最新文章，发现新的博主；阅读全文请到原博客。");
    let items: String = posts.iter().take(60).map(|p| post_item(site, p, true)).collect();
    Page {
        path: String::new(),
        title: format!("{} · 独立博客的最新文章", site.name),
        body: format!(
            "<header><h1>{}</h1><p>{}</p>{}</header><main><h2>最新文章</h2><ul>{items}</ul></main>",
            esc(site.name),
            esc(&description),
            nav(site)
        ),
        description,
        image: None,
    }
}

pub fn blogs(site: &Site, blogs: &[Value]) -> Page {
    let items: String = blogs
        .iter()
        .map(|b| {
            let mut li = format!(
                "<li><a href=\"{}\">{}</a>",
                esc(&site.link(&format!("blog/{}", b["id"]))),
                esc(s(b, "name"))
            );
            let d = s(b, "description");
            if !d.is_empty() {
                li.push_str(&format!(" — {}", esc(&truncate_chars(d, 80))));
            }
            li.push_str("</li>");
            li
        })
        .collect();
    let description = format!("{} 收录的全部 {} 个独立博客，按最近更新排列。", site.name, blogs.len());
    Page {
        path: "blogs".into(),
        title: format!("全部博客 · {}", site.name),
        body: format!(
            "<header><h1>全部博客</h1><p>{}</p>{}</header><main><ul>{items}</ul></main>",
            esc(&description),
            nav(site)
        ),
        description,
        image: None,
    }
}

pub fn about(site: &Site, meta: &Value) -> Page {
    let sources: String = meta["directories"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let home = s(d, "homepage");
            if home.is_empty() {
                format!("<li>{}</li>", esc(s(d, "name")))
            } else {
                format!("<li><a href=\"{}\">{}</a></li>", esc(home), esc(s(d, "name")))
            }
        })
        .collect();
    let description = format!(
        "关于{}：只读取各博客公开的 RSS，不抓取文章页面，摘要之外的内容请到原博客阅读。",
        site.name
    );
    let m = &meta["maintainer"];
    let maintainer = if m.is_object() {
        let mut p = format!("<h2>维护者</h2><p>{}", esc(s(m, "name")));
        if !s(m, "url").is_empty() {
            let label = m["blog"]["name"].as_str().unwrap_or(s(m, "url"));
            p.push_str(&format!(
                " · <a href=\"{}\" rel=\"me\">{}</a>",
                esc(s(m, "url")),
                esc(label)
            ));
        }
        p.push_str("</p>");
        p
    } else {
        String::new()
    };
    Page {
        path: "about".into(),
        title: format!("关于 · {}", site.name),
        body: format!(
            "<header><h1>关于{}</h1><p>{}</p>{}</header><main>{maintainer}<h2>数据来源</h2><ul>{sources}</ul></main>",
            esc(site.name),
            esc(&description),
            nav(site)
        ),
        description,
        image: None,
    }
}

pub fn blog(site: &Site, blog: &Value, posts: &[Value]) -> Page {
    let name = s(blog, "name");
    let description = [s(blog, "description"), s(blog, "about_text")]
        .into_iter()
        .find(|d| !d.is_empty())
        .map(|d| truncate_chars(d, 150))
        .unwrap_or_else(|| format!("{name} 的最新文章"));
    let mut body = format!("<header><h1>{}</h1>", esc(name));
    body.push_str(&format!("<p>{}</p>", esc(&description)));
    let mut links = vec![format!(
        "<a href=\"{}\" rel=\"noopener\">{}</a>",
        esc(s(blog, "homepage")),
        esc(s(blog, "homepage"))
    )];
    for l in blog["social"].as_array().into_iter().flatten() {
        links.push(format!(
            "<a href=\"{}\" rel=\"noopener me\">{}</a>",
            esc(s(l, "url")),
            esc(s(l, "kind"))
        ));
    }
    body.push_str(&format!("<p>{}</p>", links.join(" · ")));
    let about = s(blog, "about_text");
    if !about.is_empty() && about != description {
        body.push_str(&format!("<p>{}</p>", esc(about)));
    }
    let tags: Vec<String> = blog["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| s(c, "label").to_string())
        .chain(
            blog["top_tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(|t| format!("#{t}"))),
        )
        .collect();
    if !tags.is_empty() {
        body.push_str(&format!("<p>{}</p>", esc(&tags.join(" "))));
    }
    body.push_str(&nav(site));
    body.push_str("</header>");
    let items: String = posts.iter().take(50).map(|p| post_item(site, p, false)).collect();
    if !items.is_empty() {
        body.push_str(&format!("<main><h2>文章</h2><ul>{items}</ul></main>"));
    }
    Page {
        path: format!("blog/{}", blog["id"]),
        title: format!("{name} · {}", site.name),
        description,
        image: blog["icon"].as_str().map(str::to_string),
        body,
    }
}

/// `pages`: (path, last modified, unix seconds).
pub fn sitemap(site: &Site, pages: &[(String, Option<i64>)]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for (path, modified) in pages {
        out.push_str(&format!("  <url><loc>{}</loc>", esc(&site.link(path))));
        if let Some(d) = date(*modified) {
            out.push_str(&format!("<lastmod>{d}</lastmod>"));
        }
        out.push_str("</url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

/// Only takes effect when the site is at the root of its domain (a custom
/// domain); for a project page at github.io/<repo>/, submit the sitemap in
/// the search engines' webmaster tools instead.
pub fn robots(site: &Site) -> String {
    format!("User-agent: *\nAllow: /\nSitemap: {}/sitemap.xml\n", site.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEMPLATE: &str = "<!doctype html><html><head>\n<meta name=\"description\" content=\"default\" />\n<title>博客瀑布</title>\n</head><body><div id=\"app\"></div><script src=\"/x/assets/a.js\"></script></body></html>";

    fn site() -> Site<'static> {
        Site {
            name: "博客瀑布",
            url: "https://you.github.io/blogScroll",
            template: TEMPLATE,
        }
    }

    #[test]
    fn blog_page_has_head_content_and_links_to_originals() {
        let b = json!({"id": 12, "name": "A <b>& B", "homepage": "https://a.example/",
            "description": "写点东西", "about_text": "", "icon": "https://a.example/me.png",
            "social": [{"kind": "github", "url": "https://github.com/a"}],
            "categories": [{"id": "tech", "label": "技术"}], "top_tags": ["Rust"]});
        let posts = vec![
            json!({"title": "第一篇", "url": "https://a.example/p/1", "excerpt": "摘要",
            "published_at": 1794200000, "sort_at": 1794200000, "undated": false, "blog": {"id": 12, "name": "A"}}),
        ];
        let html = render(&site(), &blog(&site(), &b, &posts)).unwrap();
        assert!(html.contains("<title>A &lt;b&gt;&amp; B · 博客瀑布</title>"));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://you.github.io/blogScroll/blog/12\" />"));
        assert!(html.contains("<meta name=\"description\" content=\"写点东西\" />"));
        assert!(
            !html.contains("content=\"default\""),
            "the template's description is replaced"
        );
        assert!(html.contains("og:image\" content=\"https://a.example/me.png\""));
        assert!(html.contains("<a href=\"https://a.example/p/1\" rel=\"noopener\">第一篇</a>"));
        assert!(html.contains("<time datetime=\"2026-11-09\">"));
        assert!(html.contains("#Rust") && html.contains("rel=\"noopener me\""));
        assert!(html.contains(MARKER));
        assert!(html.contains("<script src=\"/x/assets/a.js\">"), "the app still loads");
    }

    #[test]
    fn sitemap_robots_and_not_found() {
        let xml = sitemap(&site(), &[(String::new(), None), ("blog/3".into(), Some(1794200000))]);
        assert!(xml.contains("<loc>https://you.github.io/blogScroll/</loc>"));
        assert!(xml.contains("<loc>https://you.github.io/blogScroll/blog/3</loc><lastmod>2026-11-09</lastmod>"));
        assert!(robots(&site()).contains("Sitemap: https://you.github.io/blogScroll/sitemap.xml"));
        assert!(not_found(&site()).contains("noindex"));
        let bad = Site {
            template: "<html></html>",
            ..site()
        };
        assert!(render(&bad, &about(&bad, &json!({}))).is_none());
    }
}
