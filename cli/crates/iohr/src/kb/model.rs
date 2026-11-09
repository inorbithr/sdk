//! The site model: what the chunks and entities say about the site as a whole. Entities with how
//! often and on how many pages they appear, facts that name their source chunk, one summary per
//! section of the site, and the link graph between pages.
//!
//! Nothing here calls a model. A section is the first segment of the URL's path; its summary is
//! extractive (counts, the longest pages' titles, the words that come up most) and says so in
//! its `source`. Facts use the shape the crawl's `facts.jsonl` already has: one JSON object per
//! line with a `kind`, the `site`, a `url` and the `crawled` day.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use serde_json::json;
use url::Url;

use super::Doc;
use super::entities::Entity;
use crate::front;
use crate::memory::text;

/// An entity with its counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityRow {
    pub kind: String,
    pub name: String,
    pub mentions: usize,
    pub pages: usize,
    /// Where it was first seen.
    pub url: String,
    pub chunk: String,
    pub source: String,
}

/// A statement about the site with the chunk that backs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub value: String,
    pub url: String,
    pub chunk: String,
    pub source: String,
}

/// A part of the site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub pages: usize,
    pub chunks: usize,
    pub tokens: usize,
    pub summary: String,
}

/// A link from one page to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRow {
    pub from: String,
    pub to: String,
    pub text: String,
    pub internal: bool,
}

/// The whole model of one site.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Model {
    /// Who the facts are about: the site's organization, else its host.
    pub subject: String,
    pub entities: Vec<EntityRow>,
    pub facts: Vec<Fact>,
    pub sections: Vec<Section>,
    pub links: Vec<LinkRow>,
}

/// The first segment of a URL's path, or `home`.
#[must_use]
pub fn section_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next().map(str::to_owned))
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "home".to_owned())
}

fn host_port(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    let host = u.host_str()?.trim_start_matches("www.").to_owned();
    Some(u.port().map_or(host.clone(), |p| format!("{host}:{p}")))
}

fn predicate_of(kind: &str) -> &'static str {
    match kind {
        "email" => "contact_email",
        "phone" => "phone",
        "price" => "lists_price",
        "product" => "offers",
        "place" => "located_in",
        "profile" => "has_profile",
        "person" => "names_person",
        "event" => "hosts_event",
        "article" => "published",
        "technology" => "built_with",
        _ => "mentions_organization",
    }
}

fn fact_id(site: &str, subject: &str, predicate: &str, value: &str, url: &str) -> String {
    front::sha_hex(&format!(
        "{site}\u{0}{subject}\u{0}{predicate}\u{0}{value}\u{0}{url}"
    ))
    .chars()
    .take(16)
    .collect()
}

fn entity_rows(docs: &[Doc]) -> Vec<EntityRow> {
    let mut by: BTreeMap<(String, String), (EntityRow, HashSet<&str>)> = BTreeMap::new();
    for d in docs {
        for e in &d.entities {
            let Entity {
                kind,
                name,
                url,
                chunk,
                source,
            } = e;
            let slot = by
                .entry(((*kind).to_owned(), name.to_lowercase()))
                .or_insert_with(|| {
                    (
                        EntityRow {
                            kind: (*kind).to_owned(),
                            name: name.clone(),
                            mentions: 0,
                            pages: 0,
                            url: url.clone(),
                            chunk: chunk.clone(),
                            source: (*source).to_owned(),
                        },
                        HashSet::new(),
                    )
                });
            slot.0.mentions += 1;
            slot.1.insert(d.url.as_str());
        }
    }
    let mut rows: Vec<EntityRow> = by
        .into_values()
        .map(|(mut r, pages)| {
            r.pages = pages.len();
            r
        })
        .collect();
    rows.sort_by(|a, b| {
        b.pages
            .cmp(&a.pages)
            .then(b.mentions.cmp(&a.mentions))
            .then(a.kind.cmp(&b.kind))
            .then(a.name.cmp(&b.name))
    });
    rows
}

fn sections(docs: &[Doc]) -> Vec<Section> {
    let mut by: BTreeMap<String, Vec<&Doc>> = BTreeMap::new();
    for d in docs {
        by.entry(section_of(&d.url)).or_default().push(d);
    }
    let mut out: Vec<Section> = by
        .into_iter()
        .map(|(name, ds)| {
            let chunks: usize = ds.iter().map(|d| d.chunked.chunks.len()).sum();
            let tokens: usize = ds
                .iter()
                .flat_map(|d| d.chunked.chunks.iter())
                .map(|c| c.tokens)
                .sum();
            let mut by_size: Vec<(&Doc, usize)> = ds
                .iter()
                .map(|d| (*d, d.chunked.chunks.iter().map(|c| c.tokens).sum()))
                .collect();
            by_size.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.url.cmp(&b.0.url)));
            let titles: Vec<&str> = by_size
                .iter()
                .map(|(d, _)| d.title.as_str())
                .filter(|t| !t.is_empty())
                .take(3)
                .collect();
            let mut count: HashMap<String, usize> = HashMap::new();
            for c in ds.iter().flat_map(|d| d.chunked.chunks.iter()) {
                for w in text::content_words(&format!("{} {}", c.path, c.text)) {
                    if w.chars().count() >= 4 && !w.chars().all(|c| c.is_ascii_digit()) {
                        *count.entry(w).or_default() += 1;
                    }
                }
            }
            let mut terms: Vec<(String, usize)> = count.into_iter().collect();
            terms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let terms: Vec<String> = terms.into_iter().take(6).map(|(w, _)| w).collect();
            let mut summary = format!(
                "{} page{}, {chunks} chunks, {tokens} tokens.",
                ds.len(),
                if ds.len() == 1 { "" } else { "s" }
            );
            if !titles.is_empty() {
                let _ = write!(summary, " Longest: {}.", titles.join("; "));
            }
            if !terms.is_empty() {
                let _ = write!(summary, " Most used words: {}.", terms.join(", "));
            }
            Section {
                name,
                pages: ds.len(),
                chunks,
                tokens,
                summary,
            }
        })
        .collect();
    out.sort_by(|a, b| b.tokens.cmp(&a.tokens).then(a.name.cmp(&b.name)));
    out
}

fn links(site: &str, docs: &[Doc]) -> Vec<LinkRow> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for d in docs {
        for l in &d.page.links {
            if l.url == d.url || !seen.insert((d.url.as_str(), l.url.as_str())) {
                continue;
            }
            let internal = host_port(&l.url).is_some_and(|h| {
                let s = site.trim_start_matches("www.");
                h == s || h.ends_with(&format!(".{s}"))
            });
            out.push(LinkRow {
                from: d.url.clone(),
                to: l.url.clone(),
                text: l.text.clone(),
                internal,
            });
        }
    }
    out
}

/// The model of `site` (the crawl's folder name) from its pages.
#[must_use]
pub fn build(site: &str, docs: &[Doc]) -> Model {
    let entities = entity_rows(docs);
    let subject = entities
        .iter()
        .filter(|e| e.kind == "organization")
        .max_by(|a, b| {
            let rank = |e: &EntityRow| (e.source == "json-ld", e.pages, e.mentions);
            rank(a).cmp(&rank(b)).then(b.name.cmp(&a.name))
        })
        .map_or_else(|| site.to_owned(), |e| e.name.clone());
    let mut facts: Vec<Fact> = Vec::new();
    let mut put =
        |subject: &str, predicate: &str, value: &str, url: &str, chunk: &str, source: &str| {
            facts.push(Fact {
                id: fact_id(site, subject, predicate, value, url),
                subject: subject.to_owned(),
                predicate: predicate.to_owned(),
                value: value.to_owned(),
                url: url.to_owned(),
                chunk: chunk.to_owned(),
                source: source.to_owned(),
            });
        };
    for e in &entities {
        if e.kind == "organization" && e.name == subject {
            continue;
        }
        put(
            &subject,
            predicate_of(&e.kind),
            &e.name,
            &e.url,
            &e.chunk,
            &e.source,
        );
    }
    for d in docs {
        let said = d.page.description.trim();
        if !said.is_empty() {
            let who = if d.title.is_empty() { &d.url } else { &d.title };
            let chunk = d.chunked.chunks.first().map_or("", |c| c.id.as_str());
            put(who, "description", said, &d.url, chunk, "meta");
        }
    }
    let mut langs: HashMap<&str, usize> = HashMap::new();
    for d in docs.iter().filter(|d| d.page.lang != "und") {
        *langs.entry(d.page.lang.as_str()).or_default() += 1;
    }
    if let Some((lang, _)) = langs
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        && let Some(d) = docs.first()
    {
        let chunk = d.chunked.chunks.first().map_or("", |c| c.id.as_str());
        put(&subject, "language", lang, &d.url, chunk, "html");
    }
    let mut seen = HashSet::new();
    facts.retain(|f| seen.insert(f.id.clone()));
    Model {
        subject,
        entities,
        facts,
        sections: sections(docs),
        links: links(site, docs),
    }
}

/// The model as JSON lines in the crawl's fact shape: `entity`, `section` and `fact` records.
#[must_use]
pub fn jsonl(site: &str, crawled: &str, m: &Model) -> String {
    let mut out = String::new();
    let mut line = |v: serde_json::Value| {
        out.push_str(&v.to_string());
        out.push('\n');
    };
    for e in &m.entities {
        line(json!({
            "kind": "entity", "site": site, "entity": e.kind, "name": e.name,
            "mentions": e.mentions, "pages": e.pages, "url": e.url, "chunk": e.chunk,
            "source": e.source, "crawled": crawled,
        }));
    }
    for s in &m.sections {
        line(json!({
            "kind": "section", "site": site, "name": s.name, "pages": s.pages,
            "chunks": s.chunks, "tokens": s.tokens, "summary": s.summary,
            "source": "extractive", "crawled": crawled,
        }));
    }
    for f in &m.facts {
        line(json!({
            "kind": "fact", "site": site, "id": f.id, "subject": f.subject,
            "predicate": f.predicate, "value": f.value, "url": f.url, "chunk": f.chunk,
            "source": f.source, "crawled": crawled,
        }));
    }
    out
}

/// The entity counts by kind, most first.
#[must_use]
pub fn by_kind(m: &Model) -> Vec<(String, usize)> {
    let mut c: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &m.entities {
        *c.entry(e.kind.as_str()).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = c.into_iter().map(|(k, n)| (k.to_owned(), n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// The atlas study for the site: what the model holds and how to ask it.
#[must_use]
pub fn study(
    site: &str,
    crawled: &str,
    pages: usize,
    chunks: usize,
    tokens: usize,
    m: &Model,
) -> String {
    let mut s = format!(
        "The model of {site}, built on this machine on {crawled} from {pages} pages: {chunks} chunks, {tokens} words, {} entities, {} facts, {} sections. It is about {}.\n\n",
        m.entities.len(),
        m.facts.len(),
        m.sections.len(),
        m.subject
    );
    s.push_str("## Sections\n\n");
    for sec in &m.sections {
        let _ = writeln!(s, "- `/{}`: {}", sec.name, sec.summary);
    }
    s.push_str("\n## Entities\n\n");
    for (kind, n) in by_kind(m) {
        let top: Vec<&str> = m
            .entities
            .iter()
            .filter(|e| e.kind == kind)
            .take(5)
            .map(|e| e.name.as_str())
            .collect();
        let _ = writeln!(s, "- {kind} ({n}): {}", top.join("; "));
    }
    let _ = write!(
        s,
        "\n## Use it\n\n`iohr atlas search \"<question>\" --site {site}` returns the chunks that answer it, each with its page and place in the page. `iohr atlas facts --site {site}` lists the facts with their source chunks. The chunks, their word counts and the index are under `.inorbit/crawl/{site}/`, which stays on this machine until `iohr sync`.\n\nSummaries here are extractive (counts, titles, frequent words), not written by a model.\n"
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::chunk::chunk;
    use crate::kb::clean::{Block, BlockKind, Link, Page};
    use crate::kb::entities;

    fn doc(url: &str, title: &str, text: &str, mut page: Page) -> Doc {
        let blocks = vec![
            Block {
                kind: BlockKind::Heading(1),
                text: title.to_owned(),
            },
            Block {
                kind: BlockKind::Text,
                text: text.to_owned(),
            },
        ];
        page.title = title.to_owned();
        let chunked = chunk(url, &blocks, &HashSet::new());
        let entities = entities::of_page(url, &page, &chunked.chunks);
        Doc {
            url: url.to_owned(),
            title: title.to_owned(),
            depth: 0,
            page,
            chunked,
            entities,
        }
    }

    fn site() -> Vec<Doc> {
        let home = Page {
            description: "Robots for warehouses.".to_owned(),
            lang: "en".to_owned(),
            site_name: "Acme".to_owned(),
            links: vec![
                Link {
                    url: "https://acme.test/docs/start".to_owned(),
                    text: "Start".to_owned(),
                },
                Link {
                    url: "https://other.test/".to_owned(),
                    text: "Partner".to_owned(),
                },
            ],
            ..Page::default()
        };
        vec![
            doc(
                "https://acme.test/",
                "Acme",
                "We build warehouse robots. Mail hello@acme.test today.",
                home,
            ),
            doc(
                "https://acme.test/docs/start",
                "Start",
                "Install the picker and charge the battery before the first shift begins.",
                Page {
                    lang: "en".to_owned(),
                    site_name: "Acme".to_owned(),
                    ..Page::default()
                },
            ),
            doc(
                "https://acme.test/docs/api",
                "API",
                "The picker api returns battery and position for every picker.",
                Page {
                    lang: "en".to_owned(),
                    ..Page::default()
                },
            ),
        ]
    }

    #[test]
    fn the_model_has_entities_facts_with_sources_sections_and_links() {
        let docs = site();
        let m = build("acme.test", &docs);
        assert_eq!(m.subject, "Acme");
        let org = m
            .entities
            .iter()
            .find(|e| e.kind == "organization")
            .unwrap();
        assert_eq!((org.mentions, org.pages), (2, 2));
        let mail = m
            .facts
            .iter()
            .find(|f| f.predicate == "contact_email")
            .unwrap();
        assert_eq!(
            (mail.subject.as_str(), mail.value.as_str()),
            ("Acme", "hello@acme.test")
        );
        let home_chunk = &docs[0].chunked.chunks[0].id;
        assert_eq!(
            &mail.chunk, home_chunk,
            "the fact names the chunk that holds it"
        );
        assert!(
            m.facts
                .iter()
                .any(|f| f.predicate == "description" && f.value == "Robots for warehouses.")
        );
        assert!(
            m.facts
                .iter()
                .any(|f| f.predicate == "language" && f.value == "en")
        );
        let names: Vec<&str> = m.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["docs", "home"]);
        assert_eq!(m.sections[0].pages, 2);
        assert!(
            m.sections[0].summary.contains("picker"),
            "{}",
            m.sections[0].summary
        );
        let internal: Vec<bool> = m.links.iter().map(|l| l.internal).collect();
        assert_eq!(internal, [true, false]);
    }

    #[test]
    fn facts_have_stable_ids_and_the_jsonl_uses_the_fact_shape() {
        let a = build("acme.test", &site());
        let b = build("acme.test", &site());
        assert_eq!(a, b);
        let ids: HashSet<&str> = a.facts.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids.len(), a.facts.len());
        let text = jsonl("acme.test", "2026-11-08", &a);
        for l in text.lines() {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            assert_eq!(v["site"], "acme.test");
            assert_eq!(v["crawled"], "2026-11-08");
            assert!(["entity", "section", "fact"].contains(&v["kind"].as_str().unwrap()));
        }
        assert!(
            text.lines()
                .any(|l| l.contains("\"kind\":\"fact\"") && l.contains("\"chunk\":\"ch_"))
        );
    }

    #[test]
    fn with_no_pages_the_model_is_empty_and_about_the_host() {
        let m = build("acme.test", &[]);
        assert_eq!(m.subject, "acme.test");
        assert!(m.facts.is_empty() && m.entities.is_empty() && m.sections.is_empty());
    }

    #[test]
    fn the_study_names_the_sections_the_entities_and_how_to_ask() {
        let m = build("acme.test", &site());
        let s = study("acme.test", "2026-11-08", 3, 3, 40, &m);
        assert!(s.contains("## Sections") && s.contains("`/docs`"));
        assert!(s.contains("organization (1): Acme"));
        assert!(s.contains("iohr atlas search"));
    }
}
