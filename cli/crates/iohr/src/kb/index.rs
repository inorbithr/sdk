//! The local index: one `DuckDB` file, `.inorbit/crawl/kb.duckdb`, for every site the project
//! has read. `DuckDB`'s `fts` extension is not part of the bundled build, so search is a token
//! table and Okapi BM25 written as one SQL query (k1 = 1.2, b = 0.75; Robertson and Zaragoza,
//! "The Probabilistic Relevance Framework", 2009). The page text itself stays in the file
//! cache beside it; the database holds the chunks, the terms, the model and the link graph.
//!
//! A terms row is a word of a chunk (lower case, a plural `s` removed, see [`term_of`]) with
//! how often it occurs there; words of the heading path count three times and words of the page
//! title twice, so a page about a thing is found by its name. Indexing a site replaces that
//! site's rows in one transaction, so an index is never half of two crawls.

use std::collections::HashMap;
use std::path::Path;

use duckdb::types::Value;
use duckdb::{Connection, params, params_from_iter};

use super::Doc;
use super::chunk::Chunk;
use super::model::{Fact, Model, Section};
use crate::memory::text;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pages(site TEXT, url TEXT, title TEXT, description TEXT, lang TEXT,
  depth INTEGER, chunks INTEGER, tokens INTEGER, links INTEGER);
CREATE TABLE IF NOT EXISTS chunks(id TEXT, site TEXT, url TEXT, ord INTEGER, heading TEXT,
  path TEXT, start_char INTEGER, end_char INTEGER, tokens INTEGER, est_model_tokens INTEGER,
  text TEXT);
CREATE TABLE IF NOT EXISTS terms(chunk_id TEXT, term TEXT, tf INTEGER);
CREATE TABLE IF NOT EXISTS entities(site TEXT, kind TEXT, name TEXT, mentions INTEGER,
  pages INTEGER, url TEXT, chunk TEXT, source TEXT);
CREATE TABLE IF NOT EXISTS facts(site TEXT, id TEXT, subject TEXT, predicate TEXT, value TEXT,
  url TEXT, chunk TEXT, source TEXT);
CREATE TABLE IF NOT EXISTS sections(site TEXT, name TEXT, pages INTEGER, chunks INTEGER,
  tokens INTEGER, summary TEXT);
CREATE TABLE IF NOT EXISTS links(site TEXT, from_url TEXT, to_url TEXT, text TEXT,
  internal BOOLEAN);
CREATE TABLE IF NOT EXISTS sites(site TEXT, crawled TEXT, built TEXT, embeddings TEXT,
  subject TEXT);
";

/// The index of the words of a query or a chunk: lower case, a plural `s` taken off words of
/// five letters or more (not `ss`), so `pickers` finds `picker`.
#[must_use]
pub fn term_of(word: &str) -> String {
    let w = word.to_lowercase();
    if w.chars().count() >= 5 && w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_owned()
    } else {
        w
    }
}

/// The terms of `s` with how often each occurs.
fn count(s: &str, weight: usize, into: &mut HashMap<String, usize>) {
    for w in text::content_words(s) {
        *into.entry(term_of(&w)).or_default() += weight;
    }
}

/// What a search returns.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub site: String,
    pub url: String,
    pub title: String,
    pub heading: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub tokens: usize,
    pub score: f64,
    pub text: String,
}

/// A fact and the site it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRow {
    pub site: String,
    pub fact: Fact,
}

/// One site's counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteStatus {
    pub site: String,
    pub subject: String,
    pub crawled: String,
    pub built: String,
    pub embeddings: String,
    pub pages: usize,
    pub chunks: usize,
    pub tokens: usize,
    pub entities: usize,
    pub facts: usize,
    pub links: usize,
    pub sections: Vec<Section>,
}

/// One page of the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRow {
    pub site: String,
    pub url: String,
    pub title: String,
    pub lang: String,
    pub chunks: usize,
    pub tokens: usize,
}

/// The index file, open.
pub struct Store {
    conn: Connection,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

fn err(what: &str, e: &duckdb::Error) -> String {
    format!("the local index: {what}: {e}")
}

fn n(v: usize) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u(v: i64) -> usize {
    usize::try_from(v).unwrap_or(0)
}

/// The model's rows (entities, facts, sections, links) of one site, into the open write.
fn add_model(tx: &duckdb::Transaction<'_>, site: &str, model: &Model) -> Result<(), String> {
    let e = |w: &str| {
        let w = w.to_owned();
        move |x: duckdb::Error| err(&w, &x)
    };
    let mut ents = tx.appender("entities").map_err(e("entities"))?;
    for r in &model.entities {
        ents.append_row(params![
            site,
            r.kind,
            r.name,
            n(r.mentions),
            n(r.pages),
            r.url,
            r.chunk,
            r.source
        ])
        .map_err(e("cannot add an entity"))?;
    }
    ents.flush().map_err(e("entities"))?;
    let mut facts = tx.appender("facts").map_err(e("facts"))?;
    for f in &model.facts {
        facts
            .append_row(params![
                site,
                f.id,
                f.subject,
                f.predicate,
                f.value,
                f.url,
                f.chunk,
                f.source
            ])
            .map_err(e("cannot add a fact"))?;
    }
    facts.flush().map_err(e("facts"))?;
    let mut secs = tx.appender("sections").map_err(e("sections"))?;
    for s in &model.sections {
        secs.append_row(params![
            site,
            s.name,
            n(s.pages),
            n(s.chunks),
            n(s.tokens),
            s.summary
        ])
        .map_err(e("cannot add a section"))?;
    }
    secs.flush().map_err(e("sections"))?;
    let mut links = tx.appender("links").map_err(e("links"))?;
    for l in &model.links {
        links
            .append_row(params![site, l.from, l.to, l.text, l.internal])
            .map_err(e("cannot add a link"))?;
    }
    links.flush().map_err(e("links"))?;
    Ok(())
}

impl Store {
    /// Opens the index, making it when it is not there.
    ///
    /// # Errors
    ///
    /// A message when the file cannot be opened or made.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| err("cannot open it", &e))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| err("cannot make its tables", &e))?;
        Ok(Self { conn })
    }

    /// Opens an index that exists, for reading only.
    ///
    /// # Errors
    ///
    /// A message when the file cannot be opened.
    pub fn open_read(path: &Path) -> Result<Self, String> {
        let cfg = duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .map_err(|e| err("cannot configure it", &e))?;
        let conn = Connection::open_with_flags(path, cfg).map_err(|e| err("cannot open it", &e))?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| err("cannot open it", &e))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| err("cannot make its tables", &e))?;
        Ok(Self { conn })
    }

    /// Replaces everything the index holds for `site` with `docs` and `model`, in one
    /// transaction.
    ///
    /// # Errors
    ///
    /// A message; the index keeps what it had.
    pub fn replace(
        &self,
        site: &str,
        docs: &[Doc],
        model: &Model,
        crawled: &str,
        built: &str,
        embeddings: &str,
    ) -> Result<(), String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| err("cannot start a write", &e))?;
        let e = |w: &str| {
            let w = w.to_owned();
            move |x: duckdb::Error| err(&w, &x)
        };
        tx.execute(
            "DELETE FROM terms WHERE chunk_id IN (SELECT id FROM chunks WHERE site = ?)",
            params![site],
        )
        .map_err(e("cannot clear the old terms"))?;
        for t in [
            "pages", "chunks", "entities", "facts", "sections", "links", "sites",
        ] {
            tx.execute(&format!("DELETE FROM {t} WHERE site = ?"), params![site])
                .map_err(e("cannot clear the old rows"))?;
        }
        {
            let mut pages = tx.appender("pages").map_err(e("pages"))?;
            let mut chunks = tx.appender("chunks").map_err(e("chunks"))?;
            let mut terms = tx.appender("terms").map_err(e("terms"))?;
            for d in docs {
                let tokens: usize = d.chunked.chunks.iter().map(|c| c.tokens).sum();
                pages
                    .append_row(params![
                        site,
                        d.url,
                        d.title,
                        d.page.description,
                        d.page.lang,
                        i64::from(d.depth),
                        n(d.chunked.chunks.len()),
                        n(tokens),
                        n(d.page.links.len())
                    ])
                    .map_err(e("cannot add a page"))?;
                for c in &d.chunked.chunks {
                    add_chunk(&mut chunks, &mut terms, site, &d.url, &d.title, c)?;
                }
            }
            pages.flush().map_err(e("pages"))?;
            chunks.flush().map_err(e("chunks"))?;
            terms.flush().map_err(e("terms"))?;
            add_model(&tx, site, model)?;
        }
        tx.execute(
            "INSERT INTO sites VALUES (?, ?, ?, ?, ?)",
            params![site, crawled, built, embeddings, model.subject],
        )
        .map_err(e("cannot note the site"))?;
        tx.commit().map_err(e("cannot save"))
    }

    /// The chunks that best answer `query`, best first, optionally of one site only.
    ///
    /// # Errors
    ///
    /// A message when the query cannot run.
    pub fn search(
        &self,
        query: &str,
        site: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Hit>, String> {
        let mut terms: Vec<String> = text::content_words(query)
            .iter()
            .map(|w| term_of(w))
            .collect();
        terms.sort();
        terms.dedup();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; terms.len()].join(", ");
        let scope = if site.is_some() { "AND c.site = ?" } else { "" };
        let sql = format!(
            "WITH stats AS (SELECT count(*)::DOUBLE AS n, greatest(avg(tokens), 1.0)::DOUBLE AS avgdl FROM chunks),
d AS (SELECT term, count(*)::DOUBLE AS df FROM terms WHERE term IN ({marks}) GROUP BY term)
SELECT c.id, c.site, c.url, p.title, c.heading, c.path, c.start_char, c.end_char, c.tokens, c.text,
  sum(ln(1 + (stats.n - d.df + 0.5) / (d.df + 0.5))
      * (t.tf * 2.2) / (t.tf + 1.2 * (0.25 + 0.75 * c.tokens / stats.avgdl))) AS score
FROM terms t
JOIN d ON d.term = t.term
JOIN chunks c ON c.id = t.chunk_id
JOIN pages p ON p.site = c.site AND p.url = c.url
CROSS JOIN stats
WHERE t.term IN ({marks}) {scope}
GROUP BY c.id, c.site, c.url, p.title, c.heading, c.path, c.start_char, c.end_char, c.tokens, c.text
ORDER BY score DESC, c.id
LIMIT ?"
        );
        let mut args: Vec<Value> = terms.iter().map(|t| Value::Text(t.clone())).collect();
        args.extend(terms.iter().map(|t| Value::Text(t.clone())));
        if let Some(s) = site {
            args.push(Value::Text(s.to_owned()));
        }
        args.push(Value::BigInt(n(limit)));
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| err("cannot prepare the search", &e))?;
        let rows = stmt
            .query_map(params_from_iter(args), |r| {
                Ok(Hit {
                    id: r.get(0)?,
                    site: r.get(1)?,
                    url: r.get(2)?,
                    title: r.get(3)?,
                    heading: r.get(4)?,
                    path: r.get(5)?,
                    start: u(r.get(6)?),
                    end: u(r.get(7)?),
                    tokens: u(r.get(8)?),
                    text: r.get(9)?,
                    score: r.get(10)?,
                })
            })
            .map_err(|e| err("cannot run the search", &e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| err("cannot read the search", &e))
    }

    /// The facts, optionally of one site, one predicate, or containing a word in the subject
    /// or the value.
    ///
    /// # Errors
    ///
    /// A message when the query cannot run.
    pub fn facts(
        &self,
        site: Option<&str>,
        predicate: Option<&str>,
        grep: Option<&str>,
        limit: usize,
    ) -> Result<Vec<FactRow>, String> {
        let mut sql =
            "SELECT site, id, subject, predicate, value, url, chunk, source FROM facts WHERE true"
                .to_owned();
        let mut args: Vec<Value> = Vec::new();
        if let Some(s) = site {
            sql.push_str(" AND site = ?");
            args.push(Value::Text(s.to_owned()));
        }
        if let Some(p) = predicate {
            sql.push_str(" AND predicate = ?");
            args.push(Value::Text(p.to_owned()));
        }
        if let Some(g) = grep {
            sql.push_str(" AND (subject ILIKE ? OR value ILIKE ?)");
            let like = format!("%{}%", g.replace('%', ""));
            args.push(Value::Text(like.clone()));
            args.push(Value::Text(like));
        }
        sql.push_str(" ORDER BY site, predicate, value, id LIMIT ?");
        args.push(Value::BigInt(n(limit)));
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| err("cannot prepare the facts", &e))?;
        let rows = stmt
            .query_map(params_from_iter(args), |r| {
                Ok(FactRow {
                    site: r.get(0)?,
                    fact: Fact {
                        id: r.get(1)?,
                        subject: r.get(2)?,
                        predicate: r.get(3)?,
                        value: r.get(4)?,
                        url: r.get(5)?,
                        chunk: r.get(6)?,
                        source: r.get(7)?,
                    },
                })
            })
            .map_err(|e| err("cannot run the facts", &e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| err("cannot read the facts", &e))
    }

    /// The sites the index holds, with their counts and sections.
    ///
    /// # Errors
    ///
    /// A message when the query cannot run.
    pub fn status(&self) -> Result<Vec<SiteStatus>, String> {
        let count = |t: &str, site: &str| -> Result<usize, String> {
            self.conn
                .query_row(
                    &format!("SELECT count(*) FROM {t} WHERE site = ?"),
                    params![site],
                    |r| r.get::<_, i64>(0),
                )
                .map(u)
                .map_err(|e| err("cannot count", &e))
        };
        let mut stmt = self
            .conn
            .prepare("SELECT site, crawled, built, embeddings, subject FROM sites ORDER BY site")
            .map_err(|e| err("cannot list the sites", &e))?;
        let rows: Vec<(String, String, String, String, String)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(|e| err("cannot list the sites", &e))?
            .collect::<Result<_, _>>()
            .map_err(|e| err("cannot read the sites", &e))?;
        let mut out = Vec::new();
        for (site, crawled, built, embeddings, subject) in rows {
            let tokens: i64 = self
                .conn
                .query_row(
                    "SELECT coalesce(sum(tokens), 0)::BIGINT FROM chunks WHERE site = ?",
                    params![site],
                    |r| r.get(0),
                )
                .map_err(|e| err("cannot count", &e))?;
            let mut s = self
                .conn
                .prepare("SELECT name, pages, chunks, tokens, summary FROM sections WHERE site = ? ORDER BY tokens DESC, name")
                .map_err(|e| err("cannot list the sections", &e))?;
            let sections = s
                .query_map(params![site], |r| {
                    Ok(Section {
                        name: r.get(0)?,
                        pages: u(r.get(1)?),
                        chunks: u(r.get(2)?),
                        tokens: u(r.get(3)?),
                        summary: r.get(4)?,
                    })
                })
                .map_err(|e| err("cannot list the sections", &e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| err("cannot read the sections", &e))?;
            out.push(SiteStatus {
                pages: count("pages", &site)?,
                chunks: count("chunks", &site)?,
                tokens: u(tokens),
                entities: count("entities", &site)?,
                facts: count("facts", &site)?,
                links: count("links", &site)?,
                sections,
                site,
                subject,
                crawled,
                built,
                embeddings,
            });
        }
        Ok(out)
    }

    /// The pages of one site (or all), with their chunk and word counts.
    ///
    /// # Errors
    ///
    /// A message when the query cannot run.
    pub fn pages(&self, site: Option<&str>, limit: usize) -> Result<Vec<PageRow>, String> {
        let mut args: Vec<Value> = Vec::new();
        let scope = if let Some(s) = site {
            args.push(Value::Text(s.to_owned()));
            "WHERE site = ?"
        } else {
            ""
        };
        args.push(Value::BigInt(n(limit)));
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT site, url, title, lang, chunks, tokens FROM pages {scope} ORDER BY site, url LIMIT ?"
            ))
            .map_err(|e| err("cannot list the pages", &e))?;
        let rows = stmt
            .query_map(params_from_iter(args), |r| {
                Ok(PageRow {
                    site: r.get(0)?,
                    url: r.get(1)?,
                    title: r.get(2)?,
                    lang: r.get(3)?,
                    chunks: u(r.get(4)?),
                    tokens: u(r.get(5)?),
                })
            })
            .map_err(|e| err("cannot list the pages", &e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| err("cannot read the pages", &e))
    }

    /// The entity counts of a site by kind, most first.
    ///
    /// # Errors
    ///
    /// A message when the query cannot run.
    pub fn entity_kinds(&self, site: &str) -> Result<Vec<(String, usize)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, count(*) AS c FROM entities WHERE site = ? GROUP BY kind ORDER BY c DESC, kind")
            .map_err(|e| err("cannot count the entities", &e))?;
        let rows = stmt
            .query_map(params![site], |r| {
                Ok((r.get::<_, String>(0)?, u(r.get(1)?)))
            })
            .map_err(|e| err("cannot count the entities", &e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| err("cannot read the entities", &e))
    }
}

fn add_chunk(
    chunks: &mut duckdb::Appender<'_>,
    terms: &mut duckdb::Appender<'_>,
    site: &str,
    url: &str,
    title: &str,
    c: &Chunk,
) -> Result<(), String> {
    chunks
        .append_row(params![
            c.id,
            site,
            url,
            n(c.ord),
            c.heading,
            c.path,
            n(c.start),
            n(c.end),
            n(c.tokens),
            n(c.est_model_tokens),
            c.text
        ])
        .map_err(|e| err("cannot add a chunk", &e))?;
    let mut tf: HashMap<String, usize> = HashMap::new();
    count(&c.text, 1, &mut tf);
    count(&c.path, 3, &mut tf);
    count(title, 2, &mut tf);
    let mut rows: Vec<(String, usize)> = tf.into_iter().collect();
    rows.sort();
    for (term, f) in rows {
        terms
            .append_row(params![c.id, term, n(f)])
            .map_err(|e| err("cannot add a term", &e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::kb::chunk::chunk;
    use crate::kb::clean::{Block, BlockKind, Page};
    use crate::kb::{entities, model};

    fn doc(url: &str, title: &str, body: &str) -> Doc {
        let blocks = vec![
            Block {
                kind: BlockKind::Heading(1),
                text: title.to_owned(),
            },
            Block {
                kind: BlockKind::Text,
                text: body.to_owned(),
            },
        ];
        let page = Page {
            title: title.to_owned(),
            lang: "en".to_owned(),
            ..Page::default()
        };
        let chunked = chunk(url, &blocks, &HashSet::new());
        let entities = entities::of_page(url, &page, &chunked.chunks);
        Doc {
            url: url.to_owned(),
            title: title.to_owned(),
            depth: 1,
            page,
            chunked,
            entities,
        }
    }

    fn docs() -> Vec<Doc> {
        vec![
            doc(
                "https://a.test/battery",
                "Battery care",
                "Charge the battery of every picker before a shift. A full battery lasts eight hours. Mail ops@a.test.",
            ),
            doc(
                "https://a.test/pricing",
                "Pricing",
                "A picker costs $1,299 per month with support included for every robot.",
            ),
            doc(
                "https://a.test/about",
                "About us",
                "We are a small team building robots for warehouses in Zagreb.",
            ),
        ]
    }

    fn filled() -> Store {
        let s = Store::in_memory().unwrap();
        let d = docs();
        let m = model::build("a.test", &d);
        s.replace("a.test", &d, &m, "2026-11-08", "2026-11-08", "stub: none")
            .unwrap();
        s
    }

    #[test]
    fn the_best_chunk_comes_first_with_its_source() {
        let s = filled();
        let hits = s.search("how long does the battery last", None, 3).unwrap();
        assert_eq!(hits[0].url, "https://a.test/battery");
        assert!(hits[0].text.contains("eight hours"));
        assert!(hits[0].score > 0.0);
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
        let price = s.search("pickers price per month", None, 3).unwrap();
        assert_eq!(
            price[0].url, "https://a.test/pricing",
            "a plural finds the singular"
        );
    }

    #[test]
    fn heading_words_and_title_words_count_more() {
        let s = filled();
        let hits = s.search("pricing", Some("a.test"), 5).unwrap();
        assert_eq!(hits[0].url, "https://a.test/pricing");
        assert!(s.search("zzzz nothing", None, 5).unwrap().is_empty());
        assert!(
            s.search("the of and", None, 5).unwrap().is_empty(),
            "only stop words"
        );
        assert!(
            s.search("battery", Some("other.test"), 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn indexing_a_site_again_replaces_its_rows_and_leaves_other_sites_alone() {
        let s = filled();
        let other = vec![doc(
            "https://b.test/x",
            "Other",
            "Battery of another site entirely.",
        )];
        s.replace(
            "b.test",
            &other,
            &model::build("b.test", &other),
            "2026-11-08",
            "2026-11-08",
            "stub: none",
        )
        .unwrap();
        let d = docs();
        let again = model::build("a.test", &d);
        s.replace(
            "a.test",
            &d,
            &again,
            "2026-11-09",
            "2026-11-09",
            "stub: none",
        )
        .unwrap();
        let st = s.status().unwrap();
        assert_eq!(st.len(), 2);
        let a = st.iter().find(|x| x.site == "a.test").unwrap();
        assert_eq!((a.pages, a.chunks), (3, 3));
        assert_eq!(a.crawled, "2026-11-09");
        assert!(a.tokens > 20);
        assert_eq!(st.iter().find(|x| x.site == "b.test").unwrap().pages, 1);
        let all = s.search("battery", None, 10).unwrap();
        assert_eq!(all.len(), 2, "no duplicate chunks after a second index");
    }

    #[test]
    fn facts_filter_by_site_predicate_and_word_and_status_lists_pages() {
        let s = filled();
        let mail = s
            .facts(Some("a.test"), Some("contact_email"), None, 10)
            .unwrap();
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].fact.value, "ops@a.test");
        assert!(mail[0].fact.chunk.starts_with("ch_"));
        let price = s.facts(None, None, Some("1,299"), 10).unwrap();
        assert_eq!(price.len(), 1);
        assert_eq!(price[0].fact.predicate, "lists_price");
        assert_eq!(s.pages(Some("a.test"), 10).unwrap().len(), 3);
        let kinds = s.entity_kinds("a.test").unwrap();
        assert!(kinds.iter().any(|(k, c)| k == "email" && *c == 1));
    }

    #[test]
    fn the_index_survives_being_closed_and_opened_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("crawl").join("kb.duckdb");
        {
            let s = Store::open(&file).unwrap();
            let d = docs();
            s.replace(
                "a.test",
                &d,
                &model::build("a.test", &d),
                "2026-11-08",
                "2026-11-08",
                "stub: none",
            )
            .unwrap();
        }
        let s = Store::open(&file).unwrap();
        assert_eq!(s.search("battery", None, 1).unwrap().len(), 1);
    }

    #[test]
    fn terms_drop_a_plural_s_only_from_longer_words() {
        assert_eq!(term_of("Pickers"), "picker");
        assert_eq!(term_of("class"), "class");
        assert_eq!(term_of("bus"), "bus");
        assert_eq!(term_of("robots"), "robot");
    }
}
