//! A formatted excerpt: the start of a post's description with its
//! paragraphs, lists, headings, quotes and emphasis kept, from its HTML or
//! from the Markdown source some feeds put there. Written out from scratch
//! with a few tags and no attributes, so it is safe to show as it is.

use std::sync::LazyLock;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;
use scraper::{ElementRef, Html, Node};

/// Bare URLs and image file names, dropped from the text as in the plain excerpt.
static BARE_LINKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)https?://\S+|\S+\.(?:jpe?g|png|webp|gif|avif|svg)\b").unwrap());
/// Hexo / Jekyll template tags left in the source.
static TEMPLATE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{%.*?%\}").unwrap());
/// Closing tags that only appear in text when the feed escaped its HTML twice.
static ESCAPED_HTML: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)</(?:p|h[1-6]|li|ol|ul|a|strong|em|b|blockquote|span|figure|figcaption|table|section|article)>|<br\s*/?>",
    )
    .unwrap()
});

/// The first `budget` characters of `raw` (a feed's description or content)
/// with their formatting, or `None` when there is none to keep (one plain
/// paragraph): the plain excerpt says it all.
pub fn excerpt_html(raw: &str, budget: usize) -> Option<String> {
    if budget == 0 {
        return None;
    }
    let mut w = Writer::new(budget);
    let doc = Html::parse_fragment(raw);
    // The fragment's root is an element of its own.
    if doc.root_element().descendants().skip(1).any(|n| n.value().is_element()) {
        walk(doc.root_element(), &mut w);
    } else {
        let text: String = doc.root_element().text().collect();
        if ESCAPED_HTML.is_match(&text) {
            walk(Html::parse_fragment(&text).root_element(), &mut w);
        } else {
            markdown(&TEMPLATE_TAG.replace_all(&text, " "), &mut w);
        }
    }
    w.finish()
}

/// What an element becomes.
enum Role {
    Block(&'static str),
    Inline(&'static str),
    /// Its text, without the element (a link, a span, ...).
    Transparent,
    /// Its text in a block of its own (a div, a table row, ...).
    Boundary,
    LineBreak,
    /// Not text worth showing: code blocks, scripts, media, forms.
    Skip,
}

fn role(name: &str) -> Role {
    match name {
        "p" => Role::Block("p"),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Role::Block("h4"),
        "ul" => Role::Block("ul"),
        "ol" => Role::Block("ol"),
        "li" => Role::Block("li"),
        "blockquote" => Role::Block("blockquote"),
        "strong" | "b" => Role::Inline("strong"),
        "em" | "i" | "cite" => Role::Inline("em"),
        "code" | "kbd" | "samp" | "tt" => Role::Inline("code"),
        "del" | "s" | "strike" => Role::Inline("del"),
        "br" => Role::LineBreak,
        "div" | "section" | "article" | "header" | "footer" | "main" | "aside" | "figure" | "figcaption" | "table"
        | "thead" | "tbody" | "tr" | "dl" | "dt" | "dd" | "hr" | "details" | "summary" | "center" => Role::Boundary,
        "script" | "style" | "noscript" | "pre" | "template" | "svg" | "math" | "img" | "picture" | "video"
        | "audio" | "iframe" | "object" | "embed" | "canvas" | "button" | "form" | "input" | "select" | "textarea"
        | "head" | "title" | "meta" | "link" => Role::Skip,
        _ => Role::Transparent,
    }
}

fn walk(el: ElementRef, w: &mut Writer) {
    for child in el.children() {
        if w.done {
            return;
        }
        match child.value() {
            Node::Text(t) => w.text(t),
            Node::Element(e) => {
                let Some(child_el) = ElementRef::wrap(child) else {
                    continue;
                };
                match role(e.name()) {
                    Role::Block(tag) => {
                        w.open_block(tag);
                        walk(child_el, w);
                        w.close(tag);
                    }
                    Role::Inline(tag) => {
                        w.open_inline(tag);
                        walk(child_el, w);
                        w.close(tag);
                    }
                    Role::Transparent => walk(child_el, w),
                    Role::Boundary => {
                        w.boundary();
                        walk(child_el, w);
                        w.boundary();
                    }
                    Role::LineBreak => w.line_break(),
                    Role::Skip => {}
                }
            }
            _ => {}
        }
    }
}

fn markdown(src: &str, w: &mut Writer) {
    // Inside a code block, an image's alt text or raw HTML: nothing shown.
    let mut skip = 0usize;
    for event in Parser::new_ext(src, Options::ENABLE_STRIKETHROUGH) {
        if w.done {
            return;
        }
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::Image { .. } | Tag::HtmlBlock) => skip += 1,
            Event::End(TagEnd::CodeBlock | TagEnd::Image | TagEnd::HtmlBlock) => skip = skip.saturating_sub(1),
            _ if skip > 0 => {}
            Event::Start(tag) => match tag {
                Tag::Paragraph => w.open_block("p"),
                Tag::Heading { .. } => w.open_block("h4"),
                Tag::BlockQuote(_) => w.open_block("blockquote"),
                Tag::List(Some(_)) => w.open_block("ol"),
                Tag::List(None) => w.open_block("ul"),
                Tag::Item => w.open_block("li"),
                Tag::Emphasis => w.open_inline("em"),
                Tag::Strong => w.open_inline("strong"),
                Tag::Strikethrough => w.open_inline("del"),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => w.close("p"),
                TagEnd::Heading(_) => w.close("h4"),
                TagEnd::BlockQuote(_) => w.close("blockquote"),
                TagEnd::List(true) => w.close("ol"),
                TagEnd::List(false) => w.close("ul"),
                TagEnd::Item => w.close("li"),
                TagEnd::Emphasis => w.close("em"),
                TagEnd::Strong => w.close("strong"),
                TagEnd::Strikethrough => w.close("del"),
                _ => {}
            },
            Event::Text(t) => w.text(&t),
            Event::Code(t) => {
                w.open_inline("code");
                w.text(&t);
                w.close("code");
            }
            // Feeds written this way keep one thought per line.
            Event::SoftBreak | Event::HardBreak => w.line_break(),
            Event::Rule => w.boundary(),
            _ => {}
        }
    }
}

struct Open {
    tag: &'static str,
    /// Written out yet: only once text arrives inside, so empty ones vanish.
    written: bool,
}

/// Writes the allowed tags as they come, counting the characters shown, and
/// stops (with "…") when the budget is spent and there is more.
struct Writer {
    out: String,
    /// Where the last character shown ends (an ellipsis goes there).
    text_end: usize,
    left: usize,
    stack: Vec<Open>,
    space: bool,
    br: bool,
    done: bool,
    /// Blocks with text, and whether anything beyond plain paragraphs was written.
    blocks: usize,
    formatted: bool,
}

fn is_inline(tag: &str) -> bool {
    matches!(tag, "strong" | "em" | "code" | "del")
}

/// Blocks that hold text (the rest hold blocks).
fn holds_text(tag: &str) -> bool {
    matches!(tag, "p" | "h4" | "li")
}

impl Writer {
    fn new(budget: usize) -> Self {
        Writer {
            out: String::new(),
            text_end: 0,
            left: budget,
            stack: Vec::new(),
            space: false,
            br: false,
            done: false,
            blocks: 0,
            formatted: false,
        }
    }

    fn open_block(&mut self, tag: &'static str) {
        if self.done {
            return;
        }
        self.end_text_block();
        // A list item needs a list; a list belongs in an item or at the top.
        if tag == "li" && !matches!(self.stack.last().map(|o| o.tag), Some("ul" | "ol")) {
            self.push("ul");
        }
        self.push(tag);
    }

    fn open_inline(&mut self, tag: &'static str) {
        if !self.done {
            self.push(tag);
        }
    }

    fn push(&mut self, tag: &'static str) {
        self.stack.push(Open { tag, written: false });
    }

    /// Closes the innermost open `tag`, and whatever is open inside it.
    fn close(&mut self, tag: &str) {
        if self.done {
            return;
        }
        if let Some(i) = self.stack.iter().rposition(|o| o.tag == tag) {
            self.pop_to(i);
        }
    }

    fn boundary(&mut self) {
        if !self.done {
            self.end_text_block();
        }
    }

    /// Before a block starts: a paragraph or heading cannot hold it.
    fn end_text_block(&mut self) {
        if let Some(i) = self.stack.iter().rposition(|o| matches!(o.tag, "p" | "h4")) {
            self.pop_to(i);
        }
        self.space = false;
        self.br = false;
    }

    fn pop_to(&mut self, i: usize) {
        while self.stack.len() > i {
            let o = self.stack.pop().unwrap();
            if o.written {
                self.out.push_str("</");
                self.out.push_str(o.tag);
                self.out.push('>');
            }
        }
        self.space = false;
        self.br = false;
    }

    fn line_break(&mut self) {
        if !self.done && self.stack.iter().any(|o| o.written && holds_text(o.tag)) {
            self.br = true;
        }
    }

    fn text(&mut self, raw: &str) {
        if self.done {
            return;
        }
        let text = BARE_LINKS.replace_all(raw, " ");
        for c in text.chars() {
            if c.is_whitespace() {
                self.space = self.space || self.has_text_here();
                continue;
            }
            self.make_room();
            // A space, line break or new block counts as one character, as
            // the space it becomes in the plain excerpt.
            let gap = self.br || self.space || (self.text_end > 0 && !self.has_text_here());
            if self.left < 1 + usize::from(gap) {
                self.out.insert(self.text_end, '…');
                self.done = true;
                return;
            }
            if gap {
                self.left -= 1;
            }
            if self.br {
                self.out.push_str("<br>");
                self.formatted = true;
            } else if self.space {
                self.out.push(' ');
            }
            self.br = false;
            self.space = false;
            self.write_opens();
            match c {
                '&' => self.out.push_str("&amp;"),
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                _ => self.out.push(c),
            }
            self.text_end = self.out.len();
            self.left -= 1;
        }
    }

    /// Whether the innermost text block has had text yet.
    fn has_text_here(&self) -> bool {
        self.stack
            .iter()
            .rev()
            .find(|o| !is_inline(o.tag))
            .is_some_and(|o| o.written)
    }

    /// Text needs a block that holds text: adds a paragraph if there is none,
    /// as for text right in a div or a quote (below any inline tags waiting
    /// to be written).
    fn make_room(&mut self) {
        let block = self.stack.iter().rposition(|o| !is_inline(o.tag));
        if block.is_some_and(|i| holds_text(self.stack[i].tag)) {
            return;
        }
        let at = block.map_or(0, |i| i + 1);
        self.stack.insert(
            at,
            Open {
                tag: "p",
                written: false,
            },
        );
    }

    fn write_opens(&mut self) {
        for o in self.stack.iter_mut().filter(|o| !o.written) {
            o.written = true;
            self.out.push('<');
            self.out.push_str(o.tag);
            self.out.push('>');
            if holds_text(o.tag) {
                self.blocks += 1;
            }
            if o.tag != "p" {
                self.formatted = true;
            }
        }
    }

    fn finish(mut self) -> Option<String> {
        self.done = false;
        self.pop_to(0);
        (self.blocks > 1 || self.formatted).then_some(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_keeps_paragraphs_lists_and_emphasis() {
        let html = r#"<div class="x"><p>第一段，<strong>重点</strong>。</p><img src="a.png"><ul><li>甲</li><li><a href="/b">乙</a></li></ul>
            <pre>code()</pre><p style="color:red" onclick="x()">末段<script>alert(1)</script></p></div>"#;
        assert_eq!(
            excerpt_html(html, 100).unwrap(),
            "<p>第一段，<strong>重点</strong>。</p><ul><li>甲</li><li>乙</li></ul><p>末段</p>"
        );
    }

    #[test]
    fn markdown_source_is_formatted() {
        let md =
            "## 标题\n\n第一行\n第二行，`code` 和 *强调*\n\n- 一\n- 二\n\n```\nskip()\n```\n\n![图](x.png)\n> 引用";
        assert_eq!(
            excerpt_html(md, 100).unwrap(),
            "<h4>标题</h4><p>第一行<br>第二行，<code>code</code> 和 <em>强调</em></p><ul><li>一</li><li>二</li></ul><blockquote><p>引用</p></blockquote>"
        );
    }

    #[test]
    fn stops_at_the_budget_and_closes_what_is_open() {
        let html = "<p>一二三</p><ul><li>四五<strong>六七八九</strong></li></ul>";
        // The break between the blocks counts as one, as its space in the plain excerpt.
        assert_eq!(
            excerpt_html(html, 8).unwrap(),
            "<p>一二三</p><ul><li>四五<strong>六七…</strong></li></ul>"
        );
        // Exactly used up with nothing after: no ellipsis.
        assert_eq!(excerpt_html("<p>一二</p><p>三</p>", 4).unwrap(), "<p>一二</p><p>三</p>");
    }

    #[test]
    fn plain_text_in_one_paragraph_is_left_to_the_plain_excerpt() {
        assert_eq!(excerpt_html("<p>Just   one\n paragraph.</p>", 100), None);
        assert_eq!(excerpt_html("Plain text, no structure.", 100), None);
        assert_eq!(excerpt_html("<p></p><p> <img src=x> </p><p>One.</p>", 100), None);
    }

    #[test]
    fn text_outside_blocks_gets_a_paragraph_and_html_is_escaped() {
        assert_eq!(
            excerpt_html("<div>a &lt;b&gt; <em>c</em></div><div>d</div>", 100).unwrap(),
            "<p>a &lt;b&gt; <em>c</em></p><p>d</p>"
        );
        // Escaped twice: the markup is in the text.
        assert_eq!(
            excerpt_html("&lt;p&gt;一&lt;/p&gt;&lt;p&gt;二&lt;/p&gt;", 100).unwrap(),
            "<p>一</p><p>二</p>"
        );
        // Bare links go, as in the plain excerpt.
        assert_eq!(
            excerpt_html("<p>见 https://a.example/x 。</p><p>二</p>", 100).unwrap(),
            "<p>见 。</p><p>二</p>"
        );
    }
}
