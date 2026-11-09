//! The site knowledge base: what a crawl read, cleaned, cut into chunks and indexed, all on
//! this machine (PRD 0192, PRD 0190.1). Nothing here sends anything anywhere; the files stay
//! under `.inorbit/crawl/` until `iohr sync`.
//!
//! The crawler (iohr-agent) lists pages and their metadata; with `--keep-pages` it also leaves
//! each page's HTML in `pages/`, which [`fetch`] reads. Only a page with no file (an older
//! agent) is requested, within the site's scope, `robots.txt` and the crawl rate. Then, per
//! site:
//!
//! 1. [`clean`]: main text, title, headings, links, language, structured data;
//! 2. [`chunk`]: heading-aware chunks with a word count, an estimate of model tokens, a stable
//!    id and offsets into the page text; text that repeats across the site is dropped;
//! 3. [`entities`] and [`model`]: entities, facts that name their chunk, section summaries and
//!    the link graph, handed back to the init crawl, which writes them beside the crawl's own
//!    facts (`facts.jsonl`) and in its atlas study;
//! 4. `index` (unix): a `DuckDB` file with BM25 search over the chunks;
//! 5. [`embed`]: a labelled stub; there are no vectors yet.
//!
//! [`build`] runs it all and returns a [`Report`] for the init crawl box; [`search`],
//! [`facts`], [`status`] and [`pages`] read the index back for `iohr atlas` and for `?`.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use url::Url;

pub mod chunk;
pub mod clean;
pub mod embed;
pub mod entities;
pub mod fetch;
#[cfg(unix)]
pub mod index;
pub mod model;
pub mod robots;

pub use index_types::{FactRow, Hit, PageRow, SiteStatus};

#[cfg(unix)]
mod index_types {
    pub use super::index::{FactRow, Hit, PageRow, SiteStatus};
}

#[cfg(not(unix))]
mod index_types {
    //! The same shapes, so callers compile everywhere; the index itself is unix-only.
    use super::model::{Fact, Section};

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
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FactRow {
        pub site: String,
        pub fact: Fact,
    }
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
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PageRow {
        pub site: String,
        pub url: String,
        pub title: String,
        pub lang: String,
        pub chunks: usize,
        pub tokens: usize,
    }
}

/// Where a project keeps its crawls.
pub const DIR: &str = ".inorbit/crawl";

/// The message on a build without the `DuckDB` index.
#[cfg(not(unix))]
const UNAVAILABLE: &str = "the local index needs DuckDB, which this build does not include";

/// One crawled page, as the crawler listed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageIn {
    pub url: String,
    pub title: String,
    pub depth: u32,
    /// The crawler read it through a browser: the page's own HTML is an empty shell.
    pub rendered: bool,
}

/// One page, cleaned and cut.
#[derive(Debug, Clone)]
pub struct Doc {
    pub url: String,
    pub title: String,
    pub depth: u32,
    pub page: clean::Page,
    pub chunked: chunk::Chunked,
    pub entities: Vec<entities::Entity>,
}

/// What a build may use.
#[derive(Debug, Clone)]
pub struct Opts {
    /// The most time the page reads may take.
    pub deadline: Duration,
    /// The most requests a second to one host (`--crawl-rate`).
    pub rate: u32,
    /// The crawl's scope, a registrable domain: no page outside it is requested.
    pub scope: String,
    /// Folders besides the site's own with pages the crawler kept (passes over subdomains).
    pub also: Vec<PathBuf>,
    /// Where a redirect the crawler met pointed: `(from, to)`. A link to `from` is a link to
    /// `to`.
    pub redirects: Vec<(String, String)>,
    /// The day the crawl ran.
    pub crawled: String,
}

/// What a build did, for the crawl box and `--json`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub pages: usize,
    pub chunks: usize,
    /// Words in the chunks.
    pub tokens: usize,
    /// An estimate of what a model would count (one per four characters).
    pub est_model_tokens: usize,
    pub entities: usize,
    pub entity_kinds: Vec<(String, usize)>,
    pub facts: usize,
    pub sections: usize,
    pub links: usize,
    /// Pages requested by this build.
    pub fetched: usize,
    /// Pages read from files already on this machine (the crawler's own).
    pub cached: usize,
    pub failed: usize,
    /// Pages not requested: outside the scope or disallowed by `robots.txt`.
    pub refused: usize,
    pub late: usize,
    /// Pages the crawler needed a browser for, whose text this build could only request
    /// plainly: they came back as an empty shell.
    pub shells: usize,
    pub embeddings: String,
    /// Why the index was not written, when it was not.
    pub index_error: Option<String>,
}

impl Report {
    /// The lines the init crawl box adds, ending with how to ask.
    #[must_use]
    pub fn lines(&self, site: &str) -> String {
        use std::fmt::Write as _;
        let kinds = if self.entity_kinds.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                self.entity_kinds
                    .iter()
                    .take(5)
                    .map(|(k, n)| format!("{k} {n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let mut s = format!(
            "knowledge base: {} pages, {} chunks, {} words (about {} model tokens, estimated)\n  entities: {}{kinds}   facts: {}   sections: {}   links: {}\n  page text: {} from files on this machine, {} requested now, {} failed{}",
            self.pages,
            self.chunks,
            self.tokens,
            self.est_model_tokens,
            self.entities,
            self.facts,
            self.sections,
            self.links,
            self.cached,
            self.fetched,
            self.failed,
            self.extra(),
        );
        match &self.index_error {
            None => {
                let _ = write!(
                    s,
                    "\n  index: {DIR}/kb.duckdb   ask: iohr atlas search \"...\" --site {site}   embeddings: {}",
                    self.embeddings
                );
            }
            Some(e) => {
                let _ = write!(s, "\n  index: not written ({e})");
            }
        }
        s
    }

    /// What was left out or came back without text, as the tail of the page-text line.
    fn extra(&self) -> String {
        let mut s = String::new();
        if self.refused > 0 {
            let _ = write!(
                s,
                ", {} not requested (outside the scope or robots.txt)",
                self.refused
            );
        }
        if self.late > 0 {
            let _ = write!(s, ", {} left out when the time ran out", self.late);
        }
        if self.shells > 0 {
            let _ = write!(
                s,
                ", {} came back as an empty shell (the crawler needed a browser for them; an agent with --keep-pages keeps what it saw)",
                self.shells
            );
        }
        s
    }

    /// `box_text` (the crawl box) with these lines before its last line, which says nothing
    /// was sent and stays last.
    #[must_use]
    pub fn within(&self, site: &str, box_text: &str) -> String {
        match box_text.rsplit_once('\n') {
            Some((head, last)) => format!("{head}\n{}\n{last}", self.lines(site)),
            None => format!("{box_text}\n{}", self.lines(site)),
        }
    }
}

fn fold_boilerplate(pages: &[clean::Page]) -> HashSet<String> {
    if pages.len() < 5 {
        return HashSet::new();
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in pages {
        let keys: HashSet<String> = p.blocks.iter().map(|b| chunk::block_key(&b.text)).collect();
        for k in keys {
            *seen.entry(k).or_default() += 1;
        }
    }
    let at_least = (pages.len() * 2).div_ceil(5).max(3);
    seen.into_iter()
        .filter(|(_, n)| *n >= at_least)
        .map(|(k, _)| k)
        .collect()
}

/// Cleans and cuts pages that are already read: `html` is the URL and its HTML.
#[must_use]
pub fn docs_of(input: &[PageIn], html: &[(String, String)]) -> Vec<Doc> {
    let by: HashMap<&str, &PageIn> = input.iter().map(|p| (p.url.as_str(), p)).collect();
    let mut cleaned: Vec<(&PageIn, clean::Page)> = Vec::new();
    for (url, body) in html {
        let (Some(p), Ok(u)) = (by.get(url.as_str()), Url::parse(url)) else {
            continue;
        };
        cleaned.push((p, clean::clean(body, &u)));
    }
    let pages: Vec<clean::Page> = cleaned.iter().map(|(_, c)| c.clone()).collect();
    let boiler = fold_boilerplate(&pages);
    cleaned
        .into_iter()
        .map(|(p, page)| {
            let chunked = chunk::chunk(&p.url, &page.blocks, &boiler);
            let entities = entities::of_page(&p.url, &page, &chunked.chunks);
            let title = if page.title.is_empty() {
                p.title.clone()
            } else {
                page.title.clone()
            };
            Doc {
                url: p.url.clone(),
                title,
                depth: p.depth,
                page,
                chunked,
                entities,
            }
        })
        .collect()
}

/// A link's address with the crawler's redirects followed (at most 10 hops, so a loop ends).
fn resolved<'a>(redirects: &'a HashMap<&str, &str>, url: &'a str) -> &'a str {
    let mut at = url;
    for _ in 0..10 {
        match redirects.get(at) {
            Some(next) => at = next,
            None => break,
        }
    }
    at
}

/// Points each link at the page it ends on: a link to an address that redirected is a link to
/// where it went, which is a page of its own.
fn follow_redirects(docs: &mut [Doc], redirects: &[(String, String)]) {
    if redirects.is_empty() {
        return;
    }
    let map: HashMap<&str, &str> = redirects
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    for d in docs {
        for l in &mut d.page.links {
            let to = resolved(&map, &l.url).to_owned();
            l.url = to;
        }
    }
}

/// What [`build`] made: the report, and the site model for the init crawl to keep.
#[derive(Debug)]
pub struct Built {
    pub report: Report,
    /// The model as JSON lines in the crawl's fact shape (`entity`, `section`, `fact`), for
    /// `facts.jsonl`; [`Built::fact_lines`] counts them.
    pub jsonl: String,
    /// The model as study text, for the "Site crawl" study.
    pub study: String,
}

impl Built {
    /// How many fact lines [`Built::jsonl`] holds.
    #[must_use]
    pub fn fact_lines(&self) -> usize {
        self.jsonl.lines().count()
    }
}

/// Reads the pages' text (the crawler's files, else once each within the scope, `robots.txt`
/// and rate), then builds the knowledge base of `site`: the index, and the model text and fact
/// lines for the caller to keep with the crawl's own. `progress` is told `(done, total)`
/// while the text is read.
///
/// # Errors
///
/// A message when the crawl's folder cannot be made. An index that cannot be written is in
/// [`Report::index_error`]; the rest stands.
pub async fn build(
    root: &Path,
    site: &str,
    input: &[PageIn],
    o: &Opts,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Built, String> {
    let dir = root.join(DIR).join(site);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let urls: Vec<String> = input.iter().map(|p| p.url.clone()).collect();
    let reach = fetch::Reach {
        dir: &dir,
        also: &o.also,
        scope: &o.scope,
        rate: o.rate,
        deadline: o.deadline,
    };
    let got = fetch::collect(&reach, &urls, progress).await;
    let html: Vec<(String, String)> = got
        .pages
        .iter()
        .filter_map(|g| {
            std::fs::read(&g.path)
                .ok()
                .map(|b| (g.url.clone(), String::from_utf8_lossy(&b).into_owned()))
        })
        .collect();
    let mut docs = docs_of(input, &html);
    follow_redirects(&mut docs, &o.redirects);
    let shells = docs
        .iter()
        .filter(|d| d.chunked.chunks.is_empty())
        .filter(|d| input.iter().any(|p| p.url == d.url && p.rendered))
        .count();
    let m = model::build(site, &docs);
    let chunks: usize = docs.iter().map(|d| d.chunked.chunks.len()).sum();
    let all = docs.iter().flat_map(|d| d.chunked.chunks.iter());
    let tokens: usize = all.clone().map(|c| c.tokens).sum();
    let est: usize = all.map(|c| c.est_model_tokens).sum();
    let index_error = write_index(root, site, &docs, &m, &o.crawled).err();
    Ok(Built {
        jsonl: model::jsonl(site, &o.crawled, &m),
        study: model::study(site, &o.crawled, docs.len(), chunks, tokens, &m),
        report: Report {
            pages: docs.len(),
            chunks,
            tokens,
            est_model_tokens: est,
            entities: m.entities.len(),
            entity_kinds: model::by_kind(&m),
            facts: m.facts.len(),
            sections: m.sections.len(),
            links: m.links.len(),
            fetched: got.fetched,
            cached: got.cached,
            failed: got.failed.len(),
            refused: got.refused,
            late: got.late,
            shells,
            embeddings: embed::STATUS.to_owned(),
            index_error,
        },
    })
}

/// Where the index is.
#[must_use]
pub fn index_path(root: &Path) -> PathBuf {
    root.join(DIR).join("kb.duckdb")
}

#[cfg(unix)]
fn write_index(
    root: &Path,
    site: &str,
    docs: &[Doc],
    m: &model::Model,
    crawled: &str,
) -> Result<(), String> {
    index::Store::open(&index_path(root))?.replace(
        site,
        docs,
        m,
        crawled,
        &crate::front::today(),
        embed::STATUS,
    )
}

#[cfg(not(unix))]
fn write_index(
    _root: &Path,
    _site: &str,
    _docs: &[Doc],
    _m: &model::Model,
    _crawled: &str,
) -> Result<(), String> {
    Err(UNAVAILABLE.to_owned())
}

/// The message when nothing has been indexed.
#[cfg(unix)]
const NONE_YET: &str = "no site has been read yet; `iohr project init --crawl <url>` reads one";

#[cfg(unix)]
fn reader(root: &Path) -> Result<index::Store, String> {
    let p = index_path(root);
    if !p.exists() {
        return Err(NONE_YET.to_owned());
    }
    index::Store::open_read(&p)
}

/// The chunks that best answer `query`, best first.
///
/// # Errors
///
/// A message when there is no index or it cannot be read.
#[cfg(unix)]
pub fn search(
    root: &Path,
    query: &str,
    site: Option<&str>,
    limit: usize,
) -> Result<Vec<Hit>, String> {
    reader(root)?.search(query, site, limit)
}

/// The facts, filtered.
///
/// # Errors
///
/// A message when there is no index or it cannot be read.
#[cfg(unix)]
pub fn facts(
    root: &Path,
    site: Option<&str>,
    predicate: Option<&str>,
    grep: Option<&str>,
    limit: usize,
) -> Result<Vec<FactRow>, String> {
    reader(root)?.facts(site, predicate, grep, limit)
}

/// The sites in the index with their counts.
///
/// # Errors
///
/// A message when there is no index or it cannot be read.
#[cfg(unix)]
pub fn status(root: &Path) -> Result<Vec<SiteStatus>, String> {
    reader(root)?.status()
}

/// The indexed pages.
///
/// # Errors
///
/// A message when there is no index or it cannot be read.
#[cfg(unix)]
pub fn pages(root: &Path, site: Option<&str>, limit: usize) -> Result<Vec<PageRow>, String> {
    reader(root)?.pages(site, limit)
}

/// The entity kinds of a site with their counts.
///
/// # Errors
///
/// A message when there is no index or it cannot be read.
#[cfg(unix)]
pub fn entity_kinds(root: &Path, site: &str) -> Result<Vec<(String, usize)>, String> {
    reader(root)?.entity_kinds(site)
}

/// Same names on a build without the index.
#[cfg(not(unix))]
macro_rules! unavailable {
    ($name:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $ret:ty) => {
        /// Not available on this build.
        ///
        /// # Errors
        ///
        /// Always: this build has no local index.
        pub fn $name($($arg: $ty),*) -> Result<$ret, String> {
            let _ = ($(&$arg,)*);
            Err(UNAVAILABLE.to_owned())
        }
    };
}
#[cfg(not(unix))]
unavailable!(search(root: &Path, query: &str, site: Option<&str>, limit: usize) -> Vec<Hit>);
#[cfg(not(unix))]
unavailable!(
    facts(
        root: &Path,
        site: Option<&str>,
        predicate: Option<&str>,
        grep: Option<&str>,
        limit: usize,
    ) -> Vec<FactRow>
);
#[cfg(not(unix))]
unavailable!(status(root: &Path) -> Vec<SiteStatus>);
#[cfg(not(unix))]
unavailable!(pages(root: &Path, site: Option<&str>, limit: usize) -> Vec<PageRow>);
#[cfg(not(unix))]
unavailable!(entity_kinds(root: &Path, site: &str) -> Vec<(String, usize)>);

/// The chunks to hand a model with a question: the best few across every site, or none when
/// nothing is indexed or the index cannot be read right now (a crawl may be writing it).
#[must_use]
pub fn sources(root: &Path, question: &str, limit: usize) -> Vec<Hit> {
    search(root, question, None, limit).unwrap_or_default()
}

/// The most of one chunk's text `<sources>` carries.
const SOURCE_CHARS: usize = 1600;

/// A header field on one line: a page's title or heading path cannot start a new line of the
/// block.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `hits` as the text of `<sources>`: one entry per chunk with its page, heading path, offsets
/// and text, cut at `SOURCE_CHARS`. Page text is untrusted: the header fields go onto one
/// line, and the message that carries the block escapes `<`, `>` and `&` in all of it (see
/// `headless`), so nothing a page says can close the tag. Empty for no hits.
#[must_use]
pub fn sources_block(hits: &[Hit]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        let text: String = h.text.chars().take(SOURCE_CHARS).collect();
        let cut = if h.text.chars().count() > SOURCE_CHARS {
            " ..."
        } else {
            ""
        };
        let path = one_line(&h.path);
        let _ = writeln!(
            out,
            "[{n}] {title} | {url} | {path} | chars {s}-{e} | {id}\n{text}{cut}\n",
            n = i + 1,
            title = one_line(&h.title),
            url = one_line(&h.url),
            path = if path.is_empty() { "-" } else { &path },
            s = h.start,
            e = h.end,
            id = one_line(&h.id),
        );
    }
    out
}

/// A site's name as the crawl folders have it, for `--site` (case-insensitive, `www.` is
/// optional).
#[must_use]
pub fn site_name(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_lowercase()
}
