//! Cleaning: the main text of one HTML page, with the boilerplate left out, and what else the
//! page says about itself (title, description, language, headings, links, structured data).
//!
//! The page is parsed with a real HTML parser. Navigation, footers, sidebars, cookie banners,
//! scripts and hidden elements are dropped; what is left is a list of blocks (headings,
//! paragraphs, list items, code, table rows) in reading order, so [`super::chunk`] can cut it
//! along its headings. Pure functions over text: nothing here reads a file or the network, and
//! page text is never logged.

use std::collections::BTreeSet;

use scraper::{ElementRef, Html, Selector, node::Node};
use url::Url;

use crate::memory::text;

/// What a block of the page is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// `h1` to `h6`, with its level.
    Heading(u8),
    /// A paragraph, or text sitting directly in a container.
    Text,
    /// A list item.
    Item,
    /// Preformatted text.
    Code,
    /// A table row, its cells joined with ` | `.
    Row,
}

/// One block of the page's main text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
}

/// A link on the page, resolved against the page's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub text: String,
}

/// One cleaned page.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Page {
    pub title: String,
    pub description: String,
    /// The language: the `lang` attribute's first part, else a guess from common words, else
    /// `und`.
    pub lang: String,
    pub canonical: Option<String>,
    /// `og:site_name`.
    pub site_name: String,
    /// `<meta name="generator">`.
    pub generator: String,
    pub blocks: Vec<Block>,
    pub links: Vec<Link>,
    /// The page's JSON-LD objects, `@graph` flattened.
    pub structured: Vec<serde_json::Value>,
}

const SKIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "iframe", "object", "embed", "canvas",
    "form", "select", "button", "input", "textarea", "nav", "footer", "aside", "head", "dialog",
    "map", "audio", "video", "picture",
];

const INLINE_TAGS: &[&str] = &[
    "a", "abbr", "acronym", "b", "bdi", "bdo", "big", "cite", "code", "data", "dfn", "em", "font",
    "i", "img", "kbd", "label", "mark", "q", "s", "samp", "small", "span", "strike", "strong",
    "sub", "sup", "time", "tt", "u", "var", "wbr", "br", "ins", "del",
];

const SKIP_ROLES: &[&str] = &[
    "navigation",
    "banner",
    "contentinfo",
    "complementary",
    "search",
    "dialog",
    "alertdialog",
];

/// Words in a `class` or `id` that mark an element as boilerplate. Whole words only: `nav-bar`
/// has `nav`, `navigator` has none.
const SKIP_WORDS: &[&str] = &[
    "nav",
    "navbar",
    "navigation",
    "footer",
    "sidebar",
    "cookie",
    "cookies",
    "consent",
    "breadcrumb",
    "breadcrumbs",
    "share",
    "sharing",
    "modal",
    "popup",
    "newsletter",
    "skiplink",
];

/// Text of a code block is cut here.
const CODE_MAX: usize = 2000;

fn select(css: &str) -> Option<Selector> {
    Selector::parse(css).ok()
}

/// One space between words, nothing at the ends.
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn words_of(attr: &str) -> impl Iterator<Item = String> + '_ {
    attr.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Whether `el` (inside the main content or not) is boilerplate or hidden.
fn skipped(el: ElementRef<'_>, in_content: bool) -> bool {
    let v = el.value();
    let name = v.name();
    if SKIP_TAGS.contains(&name) {
        return true;
    }
    if name == "header" && !in_content {
        return true;
    }
    if v.attr("hidden").is_some() || v.attr("aria-hidden") == Some("true") {
        return true;
    }
    if v.attr("style").is_some_and(|s| {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        s.contains("display:none") || s.contains("visibility:hidden")
    }) {
        return true;
    }
    if v.attr("role").is_some_and(|r| SKIP_ROLES.contains(&r)) {
        return true;
    }
    if matches!(name, "body" | "html" | "main" | "article") {
        return false;
    }
    [v.attr("class"), v.attr("id")]
        .into_iter()
        .flatten()
        .any(|a| words_of(a).any(|w| SKIP_WORDS.contains(&w.as_str())))
}

fn is_inline(name: &str) -> bool {
    INLINE_TAGS.contains(&name)
}

/// Whether an element below `el` is a block (so an inline element that holds one is a container).
fn has_block(el: ElementRef<'_>) -> bool {
    el.descendants()
        .skip(1)
        .filter_map(ElementRef::wrap)
        .any(|d| !is_inline(d.value().name()) && !SKIP_TAGS.contains(&d.value().name()))
}

#[derive(Default)]
struct Walk {
    buf: String,
    link_chars: usize,
    out: Vec<Block>,
}

impl Walk {
    fn flush(&mut self, kind: BlockKind) {
        let t = squash(&self.buf);
        let linked = self.link_chars;
        self.buf.clear();
        self.link_chars = 0;
        if t.is_empty() {
            return;
        }
        // A short run that is nearly all link is a menu, not prose.
        let len = t.chars().count();
        if kind != BlockKind::Code && len <= 160 && linked * 10 >= len * 8 {
            return;
        }
        self.out.push(Block { kind, text: t });
    }

    fn inline(&mut self, el: ElementRef<'_>, in_content: bool, in_a: bool) {
        for child in el.children() {
            match child.value() {
                Node::Text(t) => {
                    self.buf.push_str(t);
                    if in_a {
                        self.link_chars += t.trim().chars().count();
                    }
                }
                Node::Element(_) => {
                    let Some(c) = ElementRef::wrap(child) else {
                        continue;
                    };
                    if skipped(c, in_content) {
                        continue;
                    }
                    if c.value().name() == "br" {
                        self.buf.push(' ');
                        continue;
                    }
                    self.inline(c, in_content, in_a || c.value().name() == "a");
                }
                _ => {}
            }
        }
    }

    fn walk(&mut self, el: ElementRef<'_>, in_content: bool, kind: BlockKind) {
        for child in el.children() {
            match child.value() {
                Node::Text(t) => self.buf.push_str(t),
                Node::Element(_) => {
                    let Some(c) = ElementRef::wrap(child) else {
                        continue;
                    };
                    if skipped(c, in_content) {
                        continue;
                    }
                    let name = c.value().name();
                    if is_inline(name) && !has_block(c) {
                        if name == "br" {
                            self.buf.push(' ');
                        } else {
                            self.inline(c, in_content, name == "a");
                        }
                        continue;
                    }
                    self.flush(kind);
                    self.block(c, name, in_content);
                }
                _ => {}
            }
        }
        self.flush(kind);
    }

    fn block(&mut self, c: ElementRef<'_>, name: &str, in_content: bool) {
        let content = in_content || matches!(name, "main" | "article");
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.inline(c, content, false);
                let level = name.as_bytes().get(1).map_or(2, |b| b - b'0');
                self.flush(BlockKind::Heading(level));
            }
            "pre" => {
                let raw: String = c.text().collect();
                let t = raw.trim_matches('\n');
                let cut: String = t.chars().take(CODE_MAX).collect();
                if !cut.trim().is_empty() {
                    self.out.push(Block {
                        kind: BlockKind::Code,
                        text: cut,
                    });
                }
            }
            "tr" => {
                let mut cells = Vec::new();
                for cell in c.children().filter_map(ElementRef::wrap) {
                    if matches!(cell.value().name(), "td" | "th") && !skipped(cell, content) {
                        self.inline(cell, content, false);
                        let t = squash(&self.buf);
                        self.buf.clear();
                        if !t.is_empty() {
                            cells.push(t);
                        }
                    }
                }
                self.link_chars = 0;
                if !cells.is_empty() {
                    self.out.push(Block {
                        kind: BlockKind::Row,
                        text: cells.join(" | "),
                    });
                }
            }
            "li" | "dt" | "dd" => self.walk(c, content, BlockKind::Item),
            _ => self.walk(c, content, BlockKind::Text),
        }
    }
}

fn meta(doc: &Html, css: &str) -> String {
    select(css)
        .and_then(|s| {
            doc.select(&s)
                .next()
                .and_then(|e| e.value().attr("content").map(squash))
        })
        .unwrap_or_default()
}

fn blocks_of(root: ElementRef<'_>) -> Vec<Block> {
    let mut w = Walk::default();
    let content = matches!(root.value().name(), "main" | "article");
    w.walk(root, content, BlockKind::Text);
    w.out
}

fn word_count(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|b| b.text.split_whitespace().count())
        .sum()
}

fn links_of(doc: &Html, base: &Url) -> Vec<Link> {
    let Some(sel) = select("a[href]") else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for a in doc.select(&sel) {
        let Some(href) = a.value().attr("href") else {
            continue;
        };
        let href = href.trim();
        if href.starts_with('#')
            || ["mailto:", "tel:", "javascript:", "data:", "sms:"]
                .iter()
                .any(|p| href.to_ascii_lowercase().starts_with(p))
        {
            continue;
        }
        let Ok(mut u) = base.join(href) else {
            continue;
        };
        if !matches!(u.scheme(), "http" | "https") {
            continue;
        }
        u.set_fragment(None);
        let url = u.to_string();
        if !seen.insert(url.clone()) {
            continue;
        }
        let t: String = squash(&a.text().collect::<String>());
        let label = if t.is_empty() {
            a.value()
                .attr("aria-label")
                .or_else(|| a.value().attr("title"))
                .map(squash)
                .unwrap_or_default()
        } else {
            t
        };
        out.push(Link {
            url,
            text: label.chars().take(120).collect(),
        });
    }
    out
}

fn structured_of(doc: &Html) -> Vec<serde_json::Value> {
    let Some(sel) = select("script[type=\"application/ld+json\"]") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for s in doc.select(&sel) {
        let raw: String = s.text().collect();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(raw.trim()) else {
            continue;
        };
        flatten(v, &mut out);
    }
    out
}

fn flatten(v: serde_json::Value, out: &mut Vec<serde_json::Value>) {
    match v {
        serde_json::Value::Array(a) => a.into_iter().for_each(|x| flatten(x, out)),
        serde_json::Value::Object(mut o) => {
            if let Some(g) = o.remove("@graph") {
                flatten(g, out);
            }
            if o.contains_key("@type") {
                out.push(serde_json::Value::Object(o));
            }
        }
        _ => {}
    }
}

/// The language of the page: the `lang` attribute, else a guess from the most common words.
fn language(doc: &Html, blocks: &[Block]) -> String {
    let attr = select("html")
        .and_then(|s| {
            doc.select(&s)
                .next()
                .and_then(|e| e.value().attr("lang").map(str::to_owned))
        })
        .unwrap_or_default();
    let first = attr
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if (2..=3).contains(&first.len()) && first.chars().all(|c| c.is_ascii_lowercase()) {
        return first;
    }
    let sample: String = blocks
        .iter()
        .take(40)
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    guess_language(&sample).to_owned()
}

const STOPWORDS: &[(&str, &[&str])] = &[
    (
        "en",
        &[
            "the", "and", "of", "to", "in", "is", "for", "with", "that", "are", "you", "your",
        ],
    ),
    (
        "de",
        &[
            "der", "die", "und", "das", "nicht", "mit", "für", "eine", "wir", "dass", "auch",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "et", "des", "est", "pour", "dans", "une", "que", "nous", "vous",
        ],
    ),
    (
        "es",
        &[
            "el", "los", "las", "y", "es", "para", "una", "con", "que", "por", "del", "nuestro",
        ],
    ),
    (
        "it",
        &[
            "il", "che", "di", "per", "una", "con", "sono", "non", "della", "gli", "nostro", "è",
        ],
    ),
    (
        "pt",
        &[
            "o", "os", "as", "e", "para", "uma", "com", "não", "do", "da", "nosso", "são",
        ],
    ),
    (
        "nl",
        &[
            "de", "het", "een", "en", "van", "voor", "met", "niet", "zijn", "wij", "ons", "dat",
        ],
    ),
    (
        "hr",
        &[
            "je", "i", "u", "za", "na", "se", "da", "su", "od", "koji", "naš", "ili",
        ],
    ),
];

/// A guess, from how many of each language's common words the sample has; `und` when no
/// language is clearly ahead.
fn guess_language(sample: &str) -> &'static str {
    let words = text::words(sample);
    if words.len() < 20 {
        return "und";
    }
    let mut best = ("und", 0usize, 0usize);
    for (lang, stops) in STOPWORDS {
        let hits = words.iter().filter(|w| stops.contains(&w.as_str())).count();
        if hits > best.1 {
            best = (lang, hits, best.1);
        } else if hits > best.2 {
            best.2 = hits;
        }
    }
    if best.1 >= 5 && best.1 * 2 >= best.2 * 3 {
        best.0
    } else {
        "und"
    }
}

/// The cleaned page for `html`, fetched from `url`.
#[must_use]
pub fn clean(html: &str, url: &Url) -> Page {
    let doc = Html::parse_document(html);
    let root = select("main, [role=\"main\"]")
        .and_then(|s| doc.select(&s).next())
        .map(blocks_of)
        .unwrap_or_default();
    // A page with no `main`, or one whose `main` holds almost nothing, is read from its body.
    let blocks = if word_count(&root) >= 30 {
        root
    } else {
        let body = select("body")
            .and_then(|s| doc.select(&s).next())
            .map(blocks_of)
            .unwrap_or_default();
        if word_count(&body) > word_count(&root) {
            body
        } else {
            root
        }
    };
    let mut title = select("title")
        .and_then(|s| {
            doc.select(&s)
                .next()
                .map(|e| squash(&e.text().collect::<String>()))
        })
        .unwrap_or_default();
    if title.is_empty() {
        title = blocks
            .iter()
            .find(|b| b.kind == BlockKind::Heading(1))
            .map(|b| b.text.clone())
            .unwrap_or_default();
    }
    let mut description = meta(&doc, "meta[name=\"description\"]");
    if description.is_empty() {
        description = meta(&doc, "meta[property=\"og:description\"]");
    }
    let canonical = select("link[rel=\"canonical\"]").and_then(|s| {
        doc.select(&s)
            .next()
            .and_then(|e| e.value().attr("href"))
            .and_then(|h| url.join(h).ok())
            .map(|u| u.to_string())
    });
    let lang = language(&doc, &blocks);
    Page {
        title,
        description,
        lang,
        canonical,
        site_name: meta(&doc, "meta[property=\"og:site_name\"]"),
        generator: meta(&doc, "meta[name=\"generator\"]"),
        links: links_of(&doc, url),
        structured: structured_of(&doc),
        blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html><html lang="en-GB"><head>
<title>Acme Robots | Warehouse automation</title>
<meta name="description" content="Robots for warehouses.">
<meta property="og:site_name" content="Acme">
<link rel="canonical" href="/products/pickers">
<script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"Organization","name":"Acme"},{"@type":"Product","name":"Picker"}]}</script>
<style>.x{}</style></head><body>
<header><a href="/">Acme</a></header>
<nav><ul><li><a href="/products">Products</a></li><li><a href="/about">About</a></li></ul></nav>
<div class="cookie-banner">We use cookies to improve your visit.</div>
<main>
<h1>Picker</h1>
<p>The Picker moves <b>totes</b> from shelf to dock. It runs on <a href="/tech">Rust</a> and Kubernetes.</p>
<h2>Specs</h2>
<ul><li>Payload: 25 kg</li><li><a href="/x">Menu item link</a></li></ul>
<table><tr><th>Speed</th><td>2 m/s</td></tr></table>
<pre>run --fast
  go</pre>
<div hidden>never shown</div>
<p style="display: none">hidden too</p>
</main>
<aside>Related posts</aside>
<footer>Copyright Acme. All rights reserved.</footer>
<script>var x = 1;</script>
</body></html>"#;

    fn page() -> Page {
        clean(
            PAGE,
            &Url::parse("https://acme.example/products/pickers").unwrap(),
        )
    }

    #[test]
    fn the_main_text_stays_and_the_boilerplate_goes() {
        let p = page();
        let all: Vec<&str> = p.blocks.iter().map(|b| b.text.as_str()).collect();
        let joined = all.join("\n");
        assert!(
            joined.contains("The Picker moves totes from shelf to dock."),
            "{joined}"
        );
        assert!(joined.contains("Payload: 25 kg"));
        assert!(joined.contains("Speed | 2 m/s"));
        assert!(joined.contains("run --fast"));
        for gone in [
            "cookies",
            "Related posts",
            "Copyright",
            "never shown",
            "hidden too",
            "var x",
            "Products",
            "Menu item link",
        ] {
            assert!(!joined.contains(gone), "{gone} should be gone: {joined}");
        }
    }

    #[test]
    fn headings_keep_their_level_and_order() {
        let p = page();
        assert_eq!(p.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(p.blocks[0].text, "Picker");
        assert!(
            p.blocks
                .iter()
                .any(|b| b.kind == BlockKind::Heading(2) && b.text == "Specs")
        );
        assert!(p.blocks.iter().any(|b| b.kind == BlockKind::Item));
    }

    #[test]
    fn the_page_says_what_it_is() {
        let p = page();
        assert_eq!(p.title, "Acme Robots | Warehouse automation");
        assert_eq!(p.description, "Robots for warehouses.");
        assert_eq!(p.lang, "en");
        assert_eq!(p.site_name, "Acme");
        assert_eq!(
            p.canonical.as_deref(),
            Some("https://acme.example/products/pickers")
        );
        let types: Vec<&str> = p
            .structured
            .iter()
            .filter_map(|v| v["@type"].as_str())
            .collect();
        assert_eq!(types, ["Organization", "Product"]);
    }

    #[test]
    fn links_are_absolute_without_fragments_or_mailto() {
        let html = r##"<body><a href="/a#x">A</a><a href="/a">again</a><a href="mailto:x@y.z">m</a>
<a href="#top">t</a><a href="https://other.example/p">Other</a></body>"##;
        let p = clean(html, &Url::parse("https://acme.example/dir/").unwrap());
        let urls: Vec<&str> = p.links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(urls, ["https://acme.example/a", "https://other.example/p"]);
        assert_eq!(p.links[0].text, "A");
    }

    #[test]
    fn a_page_without_main_is_read_from_its_body() {
        let html = "<body><div><p>One paragraph in a div.</p></div><div id=\"sidebar\"><p>Side</p></div></body>";
        let p = clean(html, &Url::parse("https://a.example/").unwrap());
        assert_eq!(p.blocks.len(), 1);
        assert_eq!(p.blocks[0].text, "One paragraph in a div.");
    }

    #[test]
    fn the_language_is_guessed_when_the_page_does_not_say() {
        let de = "<body><p>Wir bauen Roboter für die Lagerhaltung und das war nicht einfach, \
                  aber wir sind sicher, dass die Kunden mit der Lösung das Ziel erreichen und \
                  nutzen das System für eine lange Zeit mit uns und mit der Welt.</p></body>";
        let p = clean(de, &Url::parse("https://a.example/").unwrap());
        assert_eq!(p.lang, "de");
        let none = clean(
            "<body><p>Hi</p></body>",
            &Url::parse("https://a.example/").unwrap(),
        );
        assert_eq!(none.lang, "und");
    }

    #[test]
    fn a_card_link_holding_blocks_still_gives_its_text() {
        let html = "<body><main><a href=\"/p\"><div><h3>Card title</h3><p>Card body text that is long enough to be prose and not a menu entry at all, really.</p></div></a></main></body>";
        let p = clean(html, &Url::parse("https://a.example/").unwrap());
        assert!(p.blocks.iter().any(|b| b.text == "Card title"));
        assert!(p.blocks.iter().any(|b| b.text.starts_with("Card body")));
    }
}
