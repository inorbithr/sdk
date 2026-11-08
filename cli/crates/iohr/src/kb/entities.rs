//! Entities: the things a site says about itself, found without a model.
//!
//! Three sources, each named on every entity:
//! - `json-ld`: the page's own structured data (an Organization, a Product, a Person, a Place,
//!   an Event, an Article), the most exact source there is;
//! - `meta`: the site name and the generator the page declares;
//! - `text` and `link`: patterns in a chunk (an email, an international or US phone number, a
//!   price) and links to known profile hosts.
//!
//! The patterns are heuristics and the kinds are a short fixed list, so a person or a model
//! can build on them but should read the source chunk before trusting one.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use url::Url;

use super::chunk::Chunk;
use super::clean::Page;

/// One thing found on one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    /// `organization`, `product`, `person`, `place`, `event`, `article`, `technology`,
    /// `email`, `phone`, `price` or `profile`.
    pub kind: &'static str,
    pub name: String,
    pub url: String,
    /// The chunk it came from; for page-level sources, the page's first chunk.
    pub chunk: String,
    /// `json-ld`, `meta`, `text` or `link`.
    pub source: &'static str,
}

static EMAIL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}").ok()
});
static PHONE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\+\d[\d\s().-]{7,17}\d|\(?\b\d{3}\)?[\s.-]\d{3}[\s.-]\d{4}\b").ok()
});
static PRICE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"[$€£]\s?\d[\d.,]*|\b\d[\d.,]*\s?(?:EUR|USD|GBP|€)").ok());

const PROFILE_HOSTS: &[&str] = &[
    "linkedin.com",
    "github.com",
    "x.com",
    "twitter.com",
    "youtube.com",
    "facebook.com",
    "instagram.com",
];

/// The kind a JSON-LD `@type` stands for.
fn kind_of_type(t: &str) -> Option<&'static str> {
    Some(match t {
        "Organization"
        | "Corporation"
        | "LocalBusiness"
        | "NGO"
        | "EducationalOrganization"
        | "GovernmentOrganization"
        | "Store"
        | "Restaurant"
        | "OnlineStore" => "organization",
        "Product" | "SoftwareApplication" | "MobileApplication" | "WebApplication" | "Service" => {
            "product"
        }
        "Person" => "person",
        "Place" | "PostalAddress" | "City" | "Country" => "place",
        "Event" => "event",
        "Article" | "BlogPosting" | "NewsArticle" | "TechArticle" => "article",
        _ => return None,
    })
}

fn types(v: &Value) -> Vec<&str> {
    match v.get("@type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The name an object goes by: `name`, else `headline`, else (a postal address) its locality.
fn name_of(v: &Value) -> Option<String> {
    str_of(v, "name")
        .or_else(|| str_of(v, "headline"))
        .or_else(|| str_of(v, "addressLocality"))
        .map(str::to_owned)
}

/// Walks a JSON-LD value, at most `depth` objects deep.
fn walk(v: &Value, depth: u8, out: &mut Vec<(&'static str, String)>) {
    match v {
        Value::Array(a) => {
            for x in a {
                walk(x, depth, out);
            }
        }
        Value::Object(o) => {
            for t in types(v) {
                if let (Some(kind), Some(name)) = (kind_of_type(t), name_of(v)) {
                    out.push((kind, name));
                    break;
                }
            }
            if let Some(e) = str_of(v, "email") {
                out.push(("email", e.trim_start_matches("mailto:").to_owned()));
            }
            if let Some(t) = str_of(v, "telephone") {
                out.push(("phone", t.to_owned()));
            }
            if depth > 0 {
                for (k, x) in o {
                    if !k.starts_with('@') && (x.is_object() || x.is_array()) {
                        walk(x, depth - 1, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn profile_host(link: &str) -> Option<String> {
    let u = Url::parse(link).ok()?;
    let host = u.host_str()?.trim_start_matches("www.");
    let known = PROFILE_HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")));
    // A profile has a path: the bare host is the service's own page.
    (known && u.path().trim_matches('/').len() > 1)
        .then(|| format!("{host}{}", u.path().trim_end_matches('/')))
}

fn digits(s: &str) -> usize {
    s.chars().filter(char::is_ascii_digit).count()
}

/// Every entity on one page, once each. `chunks` are the page's chunks.
#[must_use]
pub fn of_page(url: &str, page: &Page, chunks: &[Chunk]) -> Vec<Entity> {
    let first = chunks.first().map(|c| c.id.clone()).unwrap_or_default();
    let mut found: Vec<Entity> = Vec::new();
    let mut add = |kind: &'static str, name: &str, chunk: &str, source: &'static str| {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if !name.is_empty() && name.chars().count() <= 200 {
            found.push(Entity {
                kind,
                name,
                url: url.to_owned(),
                chunk: chunk.to_owned(),
                source,
            });
        }
    };
    for v in &page.structured {
        let mut got = Vec::new();
        walk(v, 4, &mut got);
        for (kind, name) in got {
            add(kind, &name, &first, "json-ld");
        }
    }
    add("organization", &page.site_name, &first, "meta");
    add("technology", &page.generator, &first, "meta");
    for l in &page.links {
        if let Some(p) = profile_host(&l.url) {
            add("profile", &p, &first, "link");
        }
    }
    for c in chunks {
        let mut each =
            |re: &LazyLock<Option<Regex>>, kind: &'static str, keep: fn(&str) -> bool| {
                if let Some(re) = re.as_ref() {
                    for m in re.find_iter(&c.text) {
                        let s = m.as_str().trim().trim_end_matches(['.', ',', '-']);
                        if keep(s) {
                            add(kind, s, &c.id, "text");
                        }
                    }
                }
            };
        each(&EMAIL, "email", |_| true);
        each(&PHONE, "phone", |s| (9..=15).contains(&digits(s)));
        each(&PRICE, "price", |s| digits(s) > 0);
    }
    let mut seen = HashSet::new();
    found.retain(|e| seen.insert((e.kind, e.name.to_lowercase())));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::clean::{Block, BlockKind};

    fn chunks_of(text: &str) -> Vec<Chunk> {
        let blocks = vec![
            Block {
                kind: BlockKind::Heading(1),
                text: "Contact".to_owned(),
            },
            Block {
                kind: BlockKind::Text,
                text: text.to_owned(),
            },
        ];
        crate::kb::chunk::chunk("https://a.test/c", &blocks, &HashSet::new()).chunks
    }

    #[test]
    fn json_ld_gives_organizations_products_people_and_places() {
        let page = Page {
            structured: vec![serde_json::json!({
                "@type": "Organization",
                "name": "Acme Robotics",
                "email": "mailto:hello@acme.test",
                "telephone": "+385 1 234 5678",
                "address": {"@type": "PostalAddress", "addressLocality": "Zagreb"},
                "founder": {"@type": "Person", "name": "Ada Lovelace"},
                "makesOffer": [{"@type": "Product", "name": "Picker 3"}]
            })],
            ..Page::default()
        };
        let e = of_page("https://a.test/", &page, &chunks_of("About us."));
        let has = |k: &str, n: &str| e.iter().any(|x| x.kind == k && x.name == n);
        assert!(has("organization", "Acme Robotics"));
        assert!(has("email", "hello@acme.test"));
        assert!(has("phone", "+385 1 234 5678"));
        assert!(has("place", "Zagreb"));
        assert!(has("person", "Ada Lovelace"));
        assert!(has("product", "Picker 3"));
        assert!(e.iter().all(|x| x.source == "json-ld"));
    }

    #[test]
    fn patterns_in_a_chunk_name_the_chunk_they_came_from() {
        let chunks = chunks_of(
            "Write to sales@acme.test or call +49 30 1234 5678. The Picker costs $1,299.00 per month. Order 12345 now.",
        );
        let e = of_page("https://a.test/c", &Page::default(), &chunks);
        let get = |k: &str| {
            e.iter()
                .find(|x| x.kind == k)
                .map(|x| (x.name.clone(), x.chunk.clone()))
        };
        assert_eq!(
            get("email").map(|x| x.0).as_deref(),
            Some("sales@acme.test")
        );
        assert_eq!(
            get("phone").map(|x| x.0).as_deref(),
            Some("+49 30 1234 5678")
        );
        assert_eq!(get("price").map(|x| x.0).as_deref(), Some("$1,299.00"));
        assert_eq!(get("email").map(|x| x.1), Some(chunks[0].id.clone()));
        assert!(e.iter().all(|x| x.source == "text"));
        assert_eq!(
            e.iter().filter(|x| x.kind == "phone").count(),
            1,
            "12345 is not a phone"
        );
    }

    #[test]
    fn meta_and_profile_links_and_repeats() {
        let page = Page {
            site_name: "Acme".to_owned(),
            generator: "WordPress 6.5".to_owned(),
            links: vec![
                crate::kb::clean::Link {
                    url: "https://www.linkedin.com/company/acme/".to_owned(),
                    text: "LinkedIn".to_owned(),
                },
                crate::kb::clean::Link {
                    url: "https://github.com/".to_owned(),
                    text: "GitHub".to_owned(),
                },
            ],
            structured: vec![serde_json::json!({"@type": "Organization", "name": "Acme"})],
            ..Page::default()
        };
        let e = of_page("https://a.test/", &page, &chunks_of("Hello there."));
        assert_eq!(
            e.iter().filter(|x| x.kind == "organization").count(),
            1,
            "json-ld and meta say the same: one entity"
        );
        assert!(
            e.iter()
                .any(|x| x.kind == "technology" && x.name == "WordPress 6.5")
        );
        assert!(
            e.iter()
                .any(|x| x.kind == "profile" && x.name == "linkedin.com/company/acme")
        );
        assert!(
            !e.iter().any(|x| x.name == "github.com"),
            "a bare host is no profile"
        );
    }

    #[test]
    fn a_page_with_nothing_has_nothing() {
        assert_eq!(
            of_page("https://a.test/", &Page::default(), &[]),
            Vec::new()
        );
    }
}
