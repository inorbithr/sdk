//! What `init` does with a public site (PRD 0190.1): read it on this machine, with the agent
//! extension's crawler, and keep what was found in the project.
//!
//! Three parts. [`decide`] settles the address and the consent: `--no-crawl` is no, `--crawl
//! <url>` is yes, and with neither a terminal is asked about a URL it found in the
//! description (default no); no terminal and no flag is no, and no request is made. [`run`]
//! starts `iohr-agent atlas crawl` (the agent extension, through the same token channel every
//! extension uses) into `.inorbit/crawl/<host>/` and turns its JSON into local files: a
//! `facts.jsonl` beside it, a study in `atlas/studies/` (so `iohr sync` carries it, RFC 0205)
//! and one project memory entry that points at them. Nothing is sent to InOrbit. [`summary`]
//! is the text init prints.
//!
//! A crawl that fails or is declined never fails init: it is one step in the report.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::Env;
use crate::atlas::{self, Kind as DocKind};
use crate::cli::{Global, InitCrawlArgs};
use crate::error::Error;
use crate::memory::store::{self, Kind as MemKind};
use crate::output::Out;
use crate::project::{self, Step};
use crate::ui::Ui;

mod progress;

/// The extension that holds the crawler.
const EXTENSION: &str = "agent";
/// The hard caps of the crawler (`iohr-agent atlas crawl --help`); init never asks for more.
const MAX_PAGES: u32 = 20_000;
const MAX_MINUTES: u32 = 240;
/// Init's other budgets, smaller than the crawler's own defaults (PRD 0190.1 sets pages and
/// minutes; these three are init's own choice).
const HOSTS: u32 = 10;
const MEGABYTES: u32 = 50;
const PAGE_MEGABYTES: u32 = 2;
/// Pages one study lists; the rest are in `facts.jsonl`.
const LISTED: usize = 500;

/// A crawl init will run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub url: Url,
    /// The folder name under `.inorbit/crawl/`: the host, and its port when it has one.
    pub name: String,
    pub pages: u32,
    pub minutes: u32,
    /// The most requests a second to one host (`--crawl-rate`).
    pub rate: u32,
    /// `--crawl-subdomains`: follow every subdomain without asking.
    pub subdomains: bool,
    /// The crawl folder as a person reads it (`.inorbit/crawl`, or a full path in home).
    pub kept_in: String,
}

impl Plan {
    /// What a dry run says it would do.
    pub(crate) fn would(&self) -> String {
        format!(
            "read {} on this machine with iohr-agent (up to {} pages or {} minutes, at most {} a second; {}) into {}/{}/, keep each page's text for a local search index (an older iohr-agent reads each page once more, at the same pace), then write atlas facts, a local knowledge base and one memory entry; nothing is sent to InOrbit",
            self.url.as_str(),
            self.pages,
            self.minutes,
            self.rate,
            if self.subdomains {
                "its subdomains too"
            } else {
                "no subdomain unless --crawl-subdomains"
            },
            self.kept_in,
            self.name
        )
    }
}

/// The consent init shows before the first request.
pub(crate) fn consent(p: &Plan) -> String {
    let host = p.url.host_str().unwrap_or_default();
    let scope = if p.subdomains {
        format!("{host} and its subdomains")
    } else {
        host.to_owned()
    };
    let more = if p.subdomains {
        ""
    } else {
        " A link to another name in its domain is not followed and is listed as not read (--crawl-subdomains follows them)."
    };
    format!(
        "This reads the public pages of {scope} on this machine, with iohr-agent: at most {} a second to one host (a slower Crawl-delay in robots.txt wins), robots.txt followed, up to {} pages or {} minutes.{more} It keeps the text of each page for a local search index (an older iohr-agent makes one more request per page, at the same pace and under the same rules). What it finds stays in this project, under {}/{}/ and atlas/. Nothing is sent to InOrbit.",
        requests(p.rate),
        p.pages,
        p.minutes,
        p.kept_in,
        p.name
    )
}

/// "1 request" or "10 requests", for the consent.
fn requests(n: u32) -> String {
    if n == 1 {
        "1 request".into()
    } else {
        format!("{n} requests")
    }
}

/// A site address: http or https, a host, no sign-in in it.
///
/// # Errors
///
/// [`Error::Usage`] saying what is wrong with it.
pub(crate) fn parse_site(raw: &str) -> Result<Url, Error> {
    let raw = raw.trim();
    let url = Url::parse(raw).map_err(|e| Error::Usage(format!("`{raw}` is not a URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::Usage(format!(
            "`{raw}` is not a web address: the crawl reads http or https sites"
        )));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(Error::Usage(format!("`{raw}` has no host")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::Usage(
            "the address must not carry a user name or password".into(),
        ));
    }
    Ok(url)
}

/// The first web address in a description, without the punctuation around it.
pub(crate) fn url_in(text: &str) -> Option<&str> {
    let at = text.find("https://").or_else(|| text.find("http://"))?;
    let rest = &text[at..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']'))
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
    (url.len() > "https://".len()).then_some(url)
}

/// A folder name for a site: its host, and `-port` when it has one, as safe characters only.
fn folder_name(url: &Url) -> String {
    let mut name = url.host_str().unwrap_or("site").to_ascii_lowercase();
    if let Some(port) = url.port() {
        let _ = write!(name, "-{port}");
    }
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The policy the agent runs under: this host and, only when `subdomains` is set, its
/// subdomains; nothing else. An address that is an IP or `localhost` has no subdomains, so it
/// is the one name.
fn policy_text(host: &str, subdomains: bool) -> String {
    let names = if !subdomains
        || host == "localhost"
        || host.parse::<std::net::IpAddr>().is_ok()
        || host.contains(':')
    {
        format!("\"{host}\"")
    } else {
        format!("\"{host}\", \"*.{host}\"")
    };
    format!(
        "# Written by `iohr project init`: the crawler may reach this site and nothing else.\nenvironment = \"staging\"\n\n[domains]\nbound = []\n\n[networks]\nallow = [{names}]\n"
    )
}

/// Settles whether init crawls and what. `None` is no crawl, with nothing printed.
///
/// # Errors
///
/// [`Error::Usage`] for an address that is not one; the person cancelled the question.
pub(crate) fn decide(
    args: &InitCrawlArgs,
    describe: Option<&str>,
    kept_in: &str,
    interactive: bool,
    ui: Ui,
    out: Out,
) -> Result<Option<Plan>, Error> {
    if args.no_crawl {
        return Ok(None);
    }
    let flagged = args.crawl.is_some();
    let raw = match (&args.crawl, describe.and_then(url_in)) {
        (Some(u), _) => u.as_str(),
        // Only a person at a terminal is asked; anywhere else a URL in a description is not
        // consent.
        (None, Some(u)) if interactive && !out.json => u,
        _ => return Ok(None),
    };
    let url = parse_site(raw)?;
    // Only here, where a crawl would run: a bad IOHR_CRAWL_RATE never stops `--no-crawl`.
    let rate = args.crawl_rate.get().map_err(Error::Usage)?;
    let plan = Plan {
        name: folder_name(&url),
        url,
        pages: args.crawl_pages.clamp(1, MAX_PAGES),
        minutes: args.crawl_minutes.clamp(1, MAX_MINUTES),
        rate,
        subdomains: args.crawl_subdomains,
        kept_in: kept_in.to_owned(),
    };
    if !out.json {
        ui.note("Crawl", &consent(&plan));
    }
    if flagged {
        return Ok(Some(plan));
    }
    let yes = ui.confirm(
        &format!("Read {} on this machine now?", plan.url.as_str()),
        false,
    )?;
    Ok(yes.then_some(plan))
}

// ---------------------------------------------------------------- the crawler's JSON

/// The part of the crawler's JSON init reads. Every field is optional, so a newer crawler
/// that adds fields, or an older one that lacks some, still reads.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Crawl {
    pub start: String,
    pub scope: String,
    pub complete: bool,
    pub stopped_by: Vec<String>,
    pub pages: Vec<Page>,
    pub hosts: Vec<HostRow>,
    /// Links the crawler met and did not follow, each with the reason.
    pub blocked: Vec<Blocked>,
    pub used: Used,
    /// Names in the site's domain that were not read: declined, or past a limit. Not in the
    /// crawler's JSON; init fills it.
    #[serde(skip)]
    pub not_followed: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Blocked {
    pub url: String,
    pub reason: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Page {
    pub url: String,
    pub status: u16,
    pub title: String,
    pub content_type: String,
    pub size: u64,
    pub depth: u32,
    pub error: Option<String>,
    /// The address this one redirected to; the crawler records the target as a page of its own.
    pub redirected_to: Option<String>,
    /// The page was drawn by the crawler's browser (a page built by script), not just fetched.
    pub rendered: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct HostRow {
    pub host: String,
    pub disallowed: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Used {
    pub seconds: u64,
    pub bytes: u64,
}

impl Page {
    /// A page that was read: a 2xx answer and no error.
    fn read(&self) -> bool {
        (200..300).contains(&self.status) && self.error.is_none()
    }

    /// A page that was asked for and did not come: an error, no answer, or a 4xx or 5xx.
    fn failed(&self) -> bool {
        self.error.is_some() || self.status == 0 || self.status >= 400
    }
}

/// What the summary counts, from the crawler's JSON.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub read: usize,
    pub failed: usize,
    pub robots: usize,
    pub seconds: u64,
    pub stopped: Vec<String>,
    pub not_followed: Vec<String>,
}

impl Crawl {
    pub(crate) fn counts(&self) -> Counts {
        Counts {
            read: self.pages.iter().filter(|p| p.read()).count(),
            failed: self.pages.iter().filter(|p| p.failed()).count(),
            robots: self.hosts.iter().map(|h| h.disallowed).sum(),
            seconds: self.used.seconds,
            stopped: self.stopped_by.clone(),
            not_followed: self.not_followed.clone(),
        }
    }
}

/// One fact per page, a JSON line. The crawler records no fetch time per page, so a fact
/// carries the day the crawl ran (`crawled`), which is the evidence time.
fn fact_line(p: &Page, site: &str, crawled: &str) -> String {
    serde_json::json!({
        "kind": "page",
        "site": site,
        "url": p.url,
        "status": p.status,
        "title": p.title,
        "content_type": p.content_type,
        "size": p.size,
        "depth": p.depth,
        "error": p.error,
        "crawled": crawled,
    })
    .to_string()
}

/// The study's text: what was read, the site model when the knowledge base was built, then one
/// line per page, up to [`LISTED`].
fn study_text(
    c: &Crawl,
    n: &Counts,
    name: &str,
    crawled: &str,
    model: Option<&str>,
    kept_in: &str,
) -> String {
    let mut t = String::new();
    let _ = writeln!(
        t,
        "Read on {crawled} on this machine by iohr-agent from {}. The crawler records no time per page, so every page below carries that day.\n",
        if c.start.is_empty() { name } else { &c.start }
    );
    let _ = writeln!(
        t,
        "- pages read: {}\n- pages that failed: {}\n- links skipped by robots.txt: {}\n- {}\n- the full record is in `{kept_in}/{name}/` (`crawl.json`, `facts.jsonl`)\n",
        n.read,
        n.failed,
        n.robots,
        stopped_line(&n.stopped, false),
    );
    if !n.not_followed.is_empty() {
        let _ = writeln!(
            t,
            "Names in the same domain that were not read: {}.\n",
            n.not_followed.join(", ")
        );
    }
    if let Some(m) = model {
        // The site model's own headings sit one level down, under this one.
        let _ = writeln!(
            t,
            "## Site model\n\n{}\n",
            m.trim_end().replace("\n## ", "\n### ")
        );
    }
    let _ = writeln!(t, "## Pages\n");
    for p in c.pages.iter().take(LISTED) {
        let title = if p.title.trim().is_empty() {
            "(no title)"
        } else {
            p.title.trim()
        };
        let state = match &p.error {
            Some(e) => format!("failed: {}", one_line(e)),
            None => p.status.to_string(),
        };
        let _ = writeln!(
            t,
            "- {} ({state}, depth {}): {}",
            p.url,
            p.depth,
            one_line(title)
        );
    }
    if c.pages.len() > LISTED {
        let _ = writeln!(
            t,
            "\n{} more pages are in `{kept_in}/{name}/facts.jsonl`.",
            c.pages.len() - LISTED
        );
    }
    t
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How the crawl ended, in words; `hint` adds how to raise the limit.
fn stopped_line(stopped: &[String], hint: bool) -> String {
    if stopped.is_empty() {
        return "stopped: no limit reached".to_owned();
    }
    let what: Vec<&str> = stopped
        .iter()
        .map(|b| match b.as_str() {
            "pages" => "the page limit",
            "minutes" => "the time limit",
            "hosts" => "the host limit",
            "bytes" => "the size limit",
            other => other,
        })
        .collect();
    let mut s = format!("stopped at {}", what.join(" and "));
    if hint {
        s.push_str("; raise it with --crawl-pages / --crawl-minutes");
    }
    s
}

/// What init prints when a crawl finished: the PRD's lines, ending with where it stands.
pub(crate) fn summary(
    name: &str,
    n: &Counts,
    facts: usize,
    memories: usize,
    kept_in: &str,
) -> String {
    let robots = n.robots;
    let failed = n.failed;
    let left_out = if n.not_followed.is_empty() {
        String::new()
    } else {
        format!(
            "\n  not read: {} (another name in the site's domain; `--crawl-subdomains` reads them without asking)",
            n.not_followed.join(", ")
        )
    };
    format!(
        "Crawl   {name}   on this machine, by iohr-agent\n  read {} pages in {} s, {robots} skipped by robots.txt, {failed} failed\n  {}{left_out}\n  facts: {facts} in atlas/   memory: {memories} entr{}   local files: {kept_in}/{name}/\nNothing was sent to InOrbit. Run iohr sync when you want it there.",
        n.read,
        n.seconds,
        stopped_line(&n.stopped, true),
        if memories == 1 { "y" } else { "ies" },
    )
}

// ---------------------------------------------------------------- the run

/// What the crawl left in the project.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Kept {
    pub facts: usize,
    pub study: String,
    pub memories: usize,
}

/// Turns a crawl's JSON into the project's own files: `facts.jsonl` beside it, a study in the
/// atlas and one memory entry. Running it again over the same site rewrites the same files.
///
/// # Errors
///
/// A message saying which file could not be written.
pub(crate) fn keep(
    root: &Path,
    name: &str,
    c: &Crawl,
    crawled: &str,
    model: Option<&crate::kb::Built>,
) -> Result<Kept, String> {
    let n = c.counts();
    let dir = crate::kb::crawl_dir(root).join(name);
    let kept_in = crate::kb::crawl_shown(root);
    let mut facts = String::new();
    for p in &c.pages {
        facts.push_str(&fact_line(p, name, crawled));
        facts.push('\n');
    }
    // The site model's entities, sections and facts go in the same file, one record per line,
    // told apart by `kind`.
    if let Some(m) = model {
        facts.push_str(&m.jsonl);
    }
    let path = dir.join("facts.jsonl");
    std::fs::write(&path, facts).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let title = format!("Site crawl {name}");
    let summary = format!(
        "{} pages of {name}, read on this machine on {crawled}.",
        n.read
    );
    let (study, number, _) = atlas::put_document(
        root,
        DocKind::Study,
        &title,
        &summary,
        crawled,
        &study_text(
            c,
            &n,
            name,
            crawled,
            model.map(|m| m.study.as_str()),
            &kept_in,
        ),
    )?;
    let store = store::Store::project(root);
    let text = format!(
        "The public site {name} was crawled on this machine; its pages are in study {number:04} ({study}) and {kept_in}/{name}/facts.jsonl."
    );
    store::remember(
        &store,
        &text,
        MemKind::Fact,
        "written by iohr project init after reading the site",
        &[study.clone(), format!("{kept_in}/{name}/facts.jsonl")],
        false,
    )
    .map_err(|e| format!("cannot write the memory entry: {e}"))?;
    Ok(Kept {
        facts: c.pages.len() + model.map_or(0, crate::kb::Built::fact_lines),
        study,
        memories: 1,
    })
}

/// What one pass of the crawler may use: what is left of the plan's limits.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Budget {
    pages: u32,
    minutes: u32,
    hosts: u32,
    megabytes: u32,
    rate: u32,
}

impl Budget {
    /// The first pass gets the plan's limits whole.
    fn of(p: &Plan) -> Self {
        Self {
            pages: p.pages,
            minutes: p.minutes,
            hosts: HOSTS,
            megabytes: MEGABYTES,
            rate: p.rate,
        }
    }

    /// What is left of the plan after `so_far`, or the limit that is used up. Every pass the
    /// person agrees to stays inside the same pages, minutes, hosts and megabytes as the first.
    fn left(p: &Plan, so_far: &Crawl) -> Result<Self, &'static str> {
        let pages = u32::try_from(so_far.pages.len()).unwrap_or(u32::MAX);
        let hosts = u32::try_from(so_far.hosts.len()).unwrap_or(u32::MAX);
        if pages >= p.pages {
            return Err("pages");
        }
        if so_far.used.seconds >= u64::from(p.minutes) * 60 {
            return Err("minutes");
        }
        if hosts >= HOSTS {
            return Err("hosts");
        }
        if so_far.used.bytes >= u64::from(MEGABYTES) * 1_048_576 {
            return Err("bytes");
        }
        let minutes_used = u32::try_from(so_far.used.seconds / 60).unwrap_or(u32::MAX);
        let mb_used = u32::try_from(so_far.used.bytes / 1_048_576).unwrap_or(u32::MAX);
        Ok(Self {
            pages: p.pages - pages,
            minutes: p.minutes.saturating_sub(minutes_used).max(1),
            hosts: HOSTS - hosts,
            megabytes: MEGABYTES.saturating_sub(mb_used).max(1),
            rate: p.rate,
        })
    }
}

/// The arguments of one pass, as the agent extension reads them. `paced` adds `--rate` and
/// `--progress`, which an agent from before RFC 0163 refuses (see [`progress::is_old_agent`]);
/// `keep` adds `--keep-pages`, so the page files are left for the knowledge base to read, which
/// an agent from before it refuses (see [`progress::is_no_keep`]).
fn crawler_args(
    start: &str,
    b: &Budget,
    policy: &Path,
    out: &Path,
    paced: bool,
    keep: bool,
) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> =
        ["atlas", "crawl", start, "--policy"].map(Into::into).into();
    a.push(policy.into());
    a.push("--out".into());
    a.push(out.into());
    for (flag, value) in [
        ("--max-pages", b.pages),
        ("--max-hosts", b.hosts),
        ("--max-minutes", b.minutes),
        ("--max-mb", b.megabytes),
        ("--max-page-mb", PAGE_MEGABYTES),
    ] {
        a.push(flag.into());
        a.push(value.to_string().into());
    }
    if paced {
        a.push("--rate".into());
        a.push(b.rate.to_string().into());
        a.push("--progress".into());
    }
    if keep {
        a.push("--keep-pages".into());
    }
    a
}

// ---------------------------------------------------------------- subdomains

/// What the person says about a subdomain the crawl met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Yes,
    No,
    /// Yes, and every other one too.
    All,
}

/// A name in the crawl's own domain that the policy kept the crawler from: the host and the
/// first address on it. The crawler visits such a link and records it as blocked with a
/// `policy` reason; it reads nothing from it. The domain is the crawl's `scope` (the
/// registrable one), so a site started on `www.` also offers its apex and its siblings.
fn blocked_hosts(c: &Crawl, site_host: &str) -> Vec<(String, String)> {
    let scope = if c.scope.is_empty() {
        site_host
    } else {
        c.scope.as_str()
    }
    .to_ascii_lowercase();
    let mut found: Vec<(String, String)> = Vec::new();
    for b in &c.blocked {
        if !b.reason.starts_with("policy") {
            continue;
        }
        let Ok(url) = Url::parse(&b.url) else {
            continue;
        };
        // The name the policy refused: for a redirect that is the target (the entry's own
        // URL is the page that redirected), so it is read from the reason when it says.
        let refused = b
            .reason
            .split_once(" is neither named in")
            .and_then(|(head, _)| head.rsplit(' ').next())
            .map(str::to_ascii_lowercase);
        let (host, start) = match (refused, url.host_str()) {
            (Some(h), Some(own)) if h != own.to_ascii_lowercase() => {
                let start = format!("{}://{h}/", url.scheme());
                (h, start)
            }
            (_, Some(own)) => (own.to_ascii_lowercase(), b.url.clone()),
            _ => continue,
        };
        let inside = host == scope || host.ends_with(&format!(".{scope}"));
        if inside && !found.iter().any(|(h, _)| *h == host) {
            found.push((host, start));
        }
    }
    found
}

/// Joins a further pass into the crawl so far: one crawl, one study, one memory entry.
fn absorb(into: &mut Crawl, more: Crawl) {
    into.pages.extend(more.pages);
    into.hosts.extend(more.hosts);
    into.blocked.extend(more.blocked);
    into.complete &= more.complete;
    for s in more.stopped_by {
        if !into.stopped_by.contains(&s) {
            into.stopped_by.push(s);
        }
    }
    into.used.seconds += more.used.seconds;
    into.used.bytes += more.used.bytes;
}

/// What a pass needs to be run: the name it is for, where it starts and what it may use.
type PassOf<'a> = (&'a str, &'a str, Budget);

/// The crawl starts on the one host. For each subdomain it met (blocked by the policy) this
/// asks once: `No` leaves it out and says so in the result, `Yes` runs a further pass on that
/// host alone, `All` does that for it and every later one without asking. Every pass is inside
/// the plan's remaining limits; when one is used up the rest are left out and the limit is
/// named in `stopped_by`.
async fn explore<F, Fut>(
    p: &Plan,
    mut crawl: Crawl,
    mut ask: impl FnMut(&str) -> Answer,
    mut pass: F,
) -> Crawl
where
    F: FnMut(String, String, Budget) -> Fut,
    Fut: std::future::Future<Output = Result<Crawl, String>>,
{
    let site = p.url.host_str().unwrap_or_default().to_ascii_lowercase();
    let mut seen: Vec<String> = vec![site.clone()];
    seen.extend(crawl.hosts.iter().map(|h| h.host.to_ascii_lowercase()));
    let mut queue = std::collections::VecDeque::new();
    let offer = |c: &Crawl, seen: &mut Vec<String>, queue: &mut std::collections::VecDeque<_>| {
        for (host, url) in blocked_hosts(c, &site) {
            if !seen.contains(&host) {
                seen.push(host.clone());
                queue.push_back((host, url));
            }
        }
    };
    offer(&crawl, &mut seen, &mut queue);
    let mut all = false;
    while let Some((host, start)) = queue.pop_front() {
        let budget = match Budget::left(p, &crawl) {
            Ok(b) => b,
            Err(limit) => {
                if !crawl.stopped_by.iter().any(|s| s == limit) {
                    crawl.stopped_by.push(limit.to_owned());
                }
                crawl.not_followed.push(host);
                crawl
                    .not_followed
                    .extend(queue.drain(..).map(|(h, _): (String, String)| h));
                break;
            }
        };
        let answer = if all { Answer::Yes } else { ask(&host) };
        if answer == Answer::No {
            crawl.not_followed.push(host);
            continue;
        }
        all |= answer == Answer::All;
        match pass(host.clone(), start, budget).await {
            Ok(more) => {
                offer(&more, &mut seen, &mut queue);
                absorb(&mut crawl, more);
            }
            Err(_) => crawl.not_followed.push(host),
        }
    }
    crawl
}

// ---------------------------------------------------------------- the passes

/// Whether the agent extension is installed.
fn have_agent(g: &Global) -> bool {
    super::ext::store(g)
        .ok()
        .and_then(|s| s.installed(EXTENSION).ok().flatten())
        .is_some()
}

fn failed(said: impl Into<String>) -> Step {
    project::step("crawl", "Crawl", "failed", said, "")
}

fn skipped(said: impl Into<String>) -> Step {
    project::step("crawl", "Crawl", "skipped", said, "")
}

/// What every pass of one run shares.
struct Ctx<'a> {
    g: &'a Global,
    env: &'a Env,
    ui: Ui,
    out: Out,
    root: &'a Path,
    /// The agent refused `--rate` and `--progress`: it is from before RFC 0163, so the later
    /// passes of this run go without them, straight to the spinner and the agent's own pace.
    old_agent: std::sync::atomic::AtomicBool,
    /// The agent refused `--keep-pages`: the page text is read from the site, once, by iohr.
    no_keep: std::sync::atomic::AtomicBool,
}

impl Ctx<'_> {
    /// One run of the crawler over `start` under `policy`, into `dir`: its folder with the
    /// policy and the JSON it left. A crawl that fails leaves no result behind. At a terminal
    /// the step is a spinner with a live count until the crawler knows how much there is, then
    /// a bar.
    async fn pass(
        &self,
        (label, start, b): PassOf<'_>,
        policy: &str,
        dir: &Path,
    ) -> Result<Crawl, String> {
        prepare_dir(self.root, dir, policy)?;
        let crawl_json = dir.join("crawl.json");
        let mut sp = (!self.out.json).then(|| {
            self.ui.step_every(
                &format!("Reading {label} on this machine"),
                crate::ui::REDRAW,
            )
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<progress::Line>();
        let ran = {
            let run = self.run_agent(start, &b, dir, &crawl_json, tx);
            tokio::pin!(run);
            loop {
                tokio::select! {
                    r = &mut run => break r,
                    Some(line) = rx.recv() => {
                        if let Some(sp) = sp.as_mut() {
                            sp.progress(line.done(), line.length(), &line.message(label));
                        }
                    }
                }
            }
        };
        match (&ran, sp) {
            (Ok(_), Some(sp)) => sp.clear(),
            (Err(e), Some(sp)) => sp.fail(&format!("The crawl failed: {e}")),
            _ => {}
        }
        ran
    }

    /// Starts the crawler once and waits for it, within the plan's minutes and a minute more.
    /// Its progress lines go to `tx`.
    async fn launch(
        &self,
        args: &[std::ffi::OsString],
        minutes: u32,
        tx: tokio::sync::mpsc::UnboundedSender<progress::Line>,
    ) -> Result<crate::ext::run::Quiet, String> {
        let store = super::ext::store(self.g).map_err(|e| e.to_string())?;
        let limit = Duration::from_secs(u64::from(minutes) * 60 + 60);
        let ran = tokio::time::timeout(
            limit,
            crate::ext::run::run_quiet(self.g, self.env, &store, EXTENSION, args, move |l| {
                if let Some(p) = progress::parse(l) {
                    let _ = tx.send(p);
                }
            }),
        )
        .await;
        match ran {
            Err(_) => Err(format!(
                "the crawler did not finish in {} minutes and was stopped",
                minutes + 1
            )),
            Ok(Err(e)) => Err(e.to_string()),
            Ok(Ok(q)) => Ok(q),
        }
    }

    async fn run_agent(
        &self,
        start: &str,
        b: &Budget,
        dir: &Path,
        crawl_json: &Path,
        tx: tokio::sync::mpsc::UnboundedSender<progress::Line>,
    ) -> Result<Crawl, String> {
        use std::sync::atomic::Ordering::Relaxed;
        let policy = dir.join("policy.toml");
        let mut paced = !self.old_agent.load(Relaxed);
        let mut keep = !self.no_keep.load(Relaxed);
        let mut quiet;
        let mut tries = 0;
        loop {
            let args = crawler_args(start, b, &policy, crawl_json, paced, keep);
            quiet = self.launch(&args, b.minutes, tx.clone()).await?;
            // An agent refuses a flag it does not know before it does anything, so running it
            // again without the flag repeats nothing. One from before RFC 0163 knows neither
            // `--rate` nor `--keep-pages`; a later one lacks only `--keep-pages`.
            tries += 1;
            if tries < 3 && paced && progress::is_old_agent(quiet.code, &quiet.stderr) {
                self.old_agent.store(true, Relaxed);
                self.no_keep.store(true, Relaxed);
                (paced, keep) = (false, false);
            } else if tries < 3 && keep && progress::is_no_keep(quiet.code, &quiet.stderr) {
                self.no_keep.store(true, Relaxed);
                keep = false;
            } else {
                break;
            }
        }
        // Exit 0 is a complete crawl and 1 one that a budget stopped: both leave the JSON.
        let parsed = std::fs::read(crawl_json)
            .ok()
            .and_then(|b| serde_json::from_slice::<Crawl>(&b).ok());
        match (quiet.code, parsed) {
            (0 | 1, Some(c)) => Ok(c),
            (code, _) => {
                let tail = quiet.stderr.trim();
                Err(if tail.is_empty() {
                    format!("the crawler exited with {code} and left no result")
                } else {
                    format!("the crawler exited with {code}: {}", one_line(tail))
                })
            }
        }
    }
}

/// The question about a crawler that is not installed: whether to install it. Asked with the
/// crawl's own consent, before the review, so the run after Create asks nothing. `false`
/// without a terminal, under `--json`, or when it is installed already.
///
/// # Errors
///
/// The person cancelled the question.
pub(crate) fn ask_install(g: &Global, ui: Ui, out: Out, interactive: bool) -> Result<bool, Error> {
    if !interactive || out.json || have_agent(g) {
        return Ok(false);
    }
    let id = format!("inorbit/{EXTENSION}");
    ui.confirm(
        &format!(
            "The crawler is part of the {id} extension, which is not installed. Install it now?"
        ),
        false,
    )
}

/// Runs the crawl and keeps what it found. The report step says how it went; the summary is
/// printed here. `install` is the answer [`ask_install`] took before the review; nothing is
/// asked here.
pub(crate) async fn run(
    g: &Global,
    env: &Env,
    ui: Ui,
    out: Out,
    install: bool,
    root: &Path,
    p: &Plan,
) -> Step {
    let bin = super::shell::running_bin();
    let id = format!("inorbit/{EXTENSION}");
    if !have_agent(g) {
        let line = format!("The crawler is part of the {id} extension, which is not installed.");
        let install_cmd = format!("{bin} ext install {id}");
        let yes = install && !out.json;
        let install = install_cmd;
        if yes {
            if let Err(e) = super::init_extensions::install_one(g, env, out, &id).await {
                return failed(format!(
                    "could not install {id}: {e}; `{install}` tries again"
                ));
            }
        } else {
            if !out.json {
                ui.warn(&format!("{line} `{install}` installs it; then `{bin} project init --crawl {}` reads the site.", p.url.as_str()));
            }
            return skipped(format!("{id} is not installed; `{install}` installs it"));
        }
    }
    let ctx = Ctx {
        g,
        env,
        ui,
        out,
        root,
        old_agent: std::sync::atomic::AtomicBool::new(false),
        no_keep: std::sync::atomic::AtomicBool::new(false),
    };
    let dir: PathBuf = crate::kb::crawl_dir(root).join(&p.name);
    let host = p.url.host_str().unwrap_or_default();
    let first = match ctx
        .pass(
            (&p.name, p.url.as_str(), Budget::of(p)),
            &policy_text(host, p.subdomains),
            &dir,
        )
        .await
    {
        Ok(c) => c,
        Err(e) => return failed(e),
    };
    // With the flag the first pass already read the subdomains; without it none is followed
    // (the crawl is under way, so nothing is asked now) and the result lists them as not read.
    let crawl = if p.subdomains {
        first
    } else {
        let ask = |_: &str| Answer::No;
        let more_dir = dir.join("more");
        explore(p, first, ask, |sub, start, b| {
            let (ctx, dir) = (&ctx, more_dir.join(folder_part(&sub)));
            async move {
                ctx.pass((&sub, &start, b), &policy_text(&sub, false), &dir)
                    .await
            }
        })
        .await
    };
    let crawled = crate::front::today();
    // The page text is read first, from the files the crawler kept, so the model is in the
    // facts and the study that `keep` writes.
    let model = read_text(ui, out, root, p, &dir, &crawl, &crawled).await;
    let kept = match keep(root, &p.name, &crawl, &crawled, model.as_ref()) {
        Ok(k) => k,
        Err(e) => return failed(e),
    };
    let n = crawl.counts();
    let mut text = summary(&p.name, &n, kept.facts, kept.memories, &p.kept_in);
    if let Some(m) = &model {
        text = m.report.within(&p.name, &text);
    }
    if !out.json {
        ui.note("Crawl", &text);
    }
    project::step(
        "crawl",
        "Crawl",
        "done",
        format!(
            "read {} pages of {} on this machine; {} facts in {}, 1 memory entry{}; nothing was sent to InOrbit",
            n.read,
            p.name,
            kept.facts,
            kept.study,
            model.as_ref().map_or_else(String::new, |m| format!(
                "; knowledge base: {} chunks, {} words, {} entities",
                m.report.chunks, m.report.tokens, m.report.entities
            )),
        ),
        "",
    )
}

/// The local knowledge base of the crawl (PRD 0192): the text of each page, cut into chunks,
/// indexed, with the site model. The pages come from the files the crawler kept under `dir`
/// (and under a followed subdomain's folder); only a page with no file is requested, within
/// the plan's scope, `robots.txt` and rate. It never fails the crawl: what went wrong is said,
/// and the crawl's own record stands.
async fn read_text(
    ui: Ui,
    out: Out,
    root: &Path,
    p: &Plan,
    dir: &Path,
    crawl: &Crawl,
    crawled: &str,
) -> Option<crate::kb::Built> {
    let input: Vec<crate::kb::PageIn> = crawl
        .pages
        .iter()
        .filter(|x| x.status == 200 && x.error.is_none())
        .filter(|x| x.content_type.is_empty() || x.content_type.to_lowercase().contains("html"))
        .map(|x| crate::kb::PageIn {
            url: x.url.clone(),
            title: x.title.clone(),
            depth: x.depth,
            rendered: x.rendered,
        })
        .collect();
    let opts = crate::kb::Opts {
        deadline: Duration::from_secs(u64::from(p.minutes) * 60),
        rate: p.rate,
        scope: p.url.host_str().unwrap_or_default().to_owned(),
        also: subfolders(&dir.join("more")),
        redirects: crawl
            .pages
            .iter()
            .filter_map(|x| Some((x.url.clone(), x.redirected_to.clone()?)))
            .collect(),
        crawled: crawled.to_owned(),
    };
    let mut sp = (!out.json && !input.is_empty()).then(|| {
        ui.step_every(
            &format!("Reading the text of {} pages", input.len()),
            crate::ui::REDRAW,
        )
    });
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, usize)>();
    let tell = move |done: usize, total: usize| {
        let _ = tx.send((done, total));
    };
    let built = {
        let run = crate::kb::build(root, &p.name, &input, &opts, &tell);
        tokio::pin!(run);
        loop {
            tokio::select! {
                r = &mut run => break r,
                Some((done, total)) = rx.recv() => {
                    if let Some(sp) = sp.as_mut() {
                        sp.progress(
                            done as u64,
                            Some(total as u64),
                            &format!("Reading the text of {total} pages: {done} done"),
                        );
                    }
                }
            }
        }
    };
    match (built, sp) {
        (Ok(r), sp) => {
            if let Some(sp) = sp {
                sp.clear();
            }
            Some(r)
        }
        (Err(e), sp) => {
            let line = format!("The knowledge base was not built: {e}");
            match sp {
                Some(sp) => sp.fail(&line),
                None if !out.json => ui.warn(&line),
                None => {}
            }
            None
        }
    }
}

/// The folders directly under `dir` (one per followed subdomain), each of which may hold the
/// crawler's `pages/`.
fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

/// A host as a folder name: safe characters only.
fn folder_part(host: &str) -> String {
    host.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Makes the crawl's folder with its policy, and a `.gitignore` for the raw record so the
/// project's own git does not commit a crawl (the study and the memory entry are what the
/// project keeps). A result of an earlier run is removed, so a crawl that fails is never
/// read as the last one that worked.
fn prepare_dir(root: &Path, dir: &Path, policy: &str) -> Result<(), String> {
    crate::kb::ensure_crawl_dir(root)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let ignore = crate::kb::crawl_dir(root).join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n")
            .map_err(|e| format!("cannot write {}: {e}", ignore.display()))?;
    }
    let path = dir.join("policy.toml");
    std::fs::write(&path, policy).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let _ = std::fs::remove_file(dir.join("crawl.json"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{CRAWL_RATE_DEFAULT, CRAWL_RATE_MAX, CRAWL_RATE_MIN, CrawlRate};
    use crate::kb::DIR;

    fn args(crawl: Option<&str>, no_crawl: bool) -> InitCrawlArgs {
        InitCrawlArgs {
            crawl: crawl.map(str::to_owned),
            no_crawl,
            crawl_pages: 200,
            crawl_minutes: 5,
            crawl_rate: CrawlRate::default(),
            crawl_subdomains: false,
        }
    }

    fn plan(url: &str) -> Plan {
        let url = parse_site(url).expect("url");
        Plan {
            name: folder_name(&url),
            url,
            pages: 200,
            minutes: 5,
            rate: 10,
            subdomains: false,
            kept_in: DIR.to_owned(),
        }
    }

    fn blocked(url: &str, reason: &str) -> Blocked {
        Blocked {
            url: url.into(),
            reason: reason.into(),
        }
    }

    /// What the crawler leaves after reading `acme.example`, which links to `shop` and `blog`
    /// (kept out by the policy) and to a page robots.txt forbids.
    fn first_pass() -> Crawl {
        Crawl {
            start: "https://acme.example/".into(),
            scope: "acme.example".into(),
            complete: true,
            pages: vec![page("https://acme.example/", 200, None)],
            hosts: vec![HostRow {
                host: "acme.example".into(),
                disallowed: 0,
            }],
            blocked: vec![
                blocked("https://shop.acme.example/", "policy: host not allowed"),
                blocked("https://blog.acme.example/a", "policy: host not allowed"),
                blocked("https://blog.acme.example/b", "policy: host not allowed"),
                blocked("https://acme.example/private", "robots.txt"),
                blocked("https://other.example/", "policy: host not allowed"),
            ],
            used: Used {
                seconds: 3,
                bytes: 100,
            },
            ..Crawl::default()
        }
    }

    /// A pass over `host`: one page on it.
    fn pass_over(host: &str) -> Crawl {
        Crawl {
            start: format!("https://{host}/"),
            scope: "acme.example".into(),
            complete: true,
            pages: vec![page(&format!("https://{host}/"), 200, None)],
            hosts: vec![HostRow {
                host: host.into(),
                disallowed: 0,
            }],
            used: Used {
                seconds: 2,
                bytes: 50,
            },
            ..Crawl::default()
        }
    }

    fn page(url: &str, status: u16, error: Option<&str>) -> Page {
        Page {
            url: url.into(),
            status,
            title: "A title".into(),
            content_type: "text/html".into(),
            size: 10,
            depth: 1,
            error: error.map(str::to_owned),
            redirected_to: None,
            rendered: false,
        }
    }

    #[test]
    fn addresses_are_checked() {
        assert!(parse_site("https://example.com/").is_ok());
        assert!(parse_site("example.com").is_err());
        assert!(parse_site("ftp://example.com").is_err());
        assert!(parse_site("https://user:pw@example.com").is_err());
        assert!(parse_site("https://user@example.com").is_err());
    }

    #[test]
    fn a_url_is_found_in_a_description() {
        assert_eq!(
            url_in("a shop, see https://acme.example/shop, for the look"),
            Some("https://acme.example/shop")
        );
        assert_eq!(
            url_in("(https://acme.example)"),
            Some("https://acme.example")
        );
        assert_eq!(url_in("no address here"), None);
        assert_eq!(url_in("just https:// alone"), None);
    }

    #[test]
    fn the_folder_is_the_host_and_its_port() {
        assert_eq!(
            folder_name(&parse_site("https://Acme.example/x").unwrap()),
            "acme.example"
        );
        assert_eq!(
            folder_name(&parse_site("http://127.0.0.1:8080/").unwrap()),
            "127.0.0.1-8080"
        );
    }

    #[test]
    fn the_policy_is_the_host_alone_until_subdomains_are_asked_for() {
        let p = policy_text("acme.example", false);
        assert!(p.contains("allow = [\"acme.example\"]"), "{p}");
        assert!(!p.contains('*'), "{p}");
        let s = policy_text("acme.example", true);
        assert!(
            s.contains("allow = [\"acme.example\", \"*.acme.example\"]"),
            "{s}"
        );
        let l = policy_text("127.0.0.1", true);
        assert!(l.contains("allow = [\"127.0.0.1\"]"), "{l}");
        assert!(!l.contains('*'), "{l}");
    }

    #[test]
    fn no_flag_and_no_terminal_is_no_crawl_even_with_a_url_in_the_description() {
        let ui = Ui::detect(true);
        let out = Out { json: false };
        let r = decide(
            &args(None, false),
            Some("see https://acme.example"),
            DIR,
            false,
            ui,
            out,
        );
        assert_eq!(r.unwrap(), None);
    }

    #[test]
    fn no_crawl_beats_the_flag_and_the_description() {
        let ui = Ui::detect(true);
        let out = Out { json: false };
        let r = decide(
            &args(Some("https://acme.example"), true),
            None,
            DIR,
            true,
            ui,
            out,
        );
        assert_eq!(r.unwrap(), None);
    }

    #[test]
    fn a_new_agent_gets_the_rate_and_progress_an_old_one_neither() {
        let mut p = plan("https://acme.example/");
        p.rate = 7;
        let line = |paced| {
            crawler_args(
                p.url.as_str(),
                &Budget::of(&p),
                Path::new("/p/policy.toml"),
                Path::new("/p/crawl.json"),
                paced,
                false,
            )
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
        };
        assert!(
            line(true).ends_with("--max-page-mb 2 --rate 7 --progress"),
            "{}",
            line(true)
        );
        assert!(!line(false).contains("--rate") && !line(false).contains("--progress"));
    }

    #[test]
    fn the_crawler_is_asked_to_keep_the_pages_unless_it_is_too_old() {
        let p = plan("https://acme.example/");
        let line = |keep| {
            crawler_args(
                p.url.as_str(),
                &Budget::of(&p),
                Path::new("/p/policy.toml"),
                Path::new("/p/crawl.json"),
                true,
                keep,
            )
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
        };
        assert!(
            line(true).ends_with("--progress --keep-pages"),
            "{}",
            line(true)
        );
        assert!(!line(false).contains("--keep-pages"));
    }

    #[test]
    fn the_consent_and_the_dry_run_say_the_rate() {
        let mut p = plan("https://acme.example/");
        assert!(
            consent(&p).contains("at most 10 requests a second to one host"),
            "{}",
            consent(&p)
        );
        assert!(consent(&p).contains("a slower Crawl-delay in robots.txt wins"));
        assert!(p.would().contains("at most 10 a second"), "{}", p.would());
        p.rate = 1;
        assert!(
            consent(&p).contains("at most 1 request a second"),
            "{}",
            consent(&p)
        );
    }

    #[test]
    fn the_rate_is_the_crawlers_bounds_or_a_usage_error_when_a_crawl_would_run() {
        let ui = Ui::detect(true);
        let out = Out { json: true };
        for (asked, kept) in [("1", 1), ("10", 10), ("50", 50), (" 7 ", 7)] {
            let mut a = args(Some("https://acme.example/"), false);
            a.crawl_rate = asked.parse::<CrawlRate>().unwrap();
            let p = decide(&a, None, DIR, false, ui, out).unwrap().unwrap();
            assert_eq!(p.rate, kept, "asked {asked}");
        }
        for bad in ["0", "51", "-1", "1000", "fast", "", "2.5"] {
            let mut a = args(Some("https://acme.example/"), false);
            a.crawl_rate = bad.parse::<CrawlRate>().unwrap();
            let e = decide(&a, None, DIR, false, ui, out).unwrap_err();
            assert!(matches!(e, Error::Usage(_)), "{bad}: {e:?}");
            assert!(e.to_string().contains("1 to 50"), "{bad}: {e}");
        }
        // Nothing is asked of the rate when no crawl runs.
        let mut a = args(None, true);
        a.crawl_rate = "fast".parse::<CrawlRate>().unwrap();
        assert!(decide(&a, None, DIR, false, ui, out).unwrap().is_none());
        let mut a = args(None, false);
        a.crawl_rate = "fast".parse::<CrawlRate>().unwrap();
        assert!(decide(&a, None, DIR, false, ui, out).unwrap().is_none());
    }

    #[test]
    fn the_default_rate_is_the_one_const_and_in_range() {
        assert_eq!(
            CrawlRate::default().get(),
            Ok(crate::cli::CRAWL_RATE_DEFAULT)
        );
        assert!((CRAWL_RATE_MIN..=CRAWL_RATE_MAX).contains(&CRAWL_RATE_DEFAULT));
    }

    #[test]
    fn the_flag_is_consent_and_the_budgets_are_clamped() {
        let ui = Ui::detect(true);
        let out = Out { json: true };
        let mut a = args(Some("https://acme.example/"), false);
        a.crawl_pages = 1_000_000;
        a.crawl_minutes = 0;
        let p = decide(&a, None, DIR, false, ui, out).unwrap().unwrap();
        assert_eq!((p.pages, p.minutes), (MAX_PAGES, 1));
        assert!(consent(&p).contains("Nothing is sent to InOrbit"));
    }

    #[test]
    fn a_bad_address_is_a_usage_error() {
        let ui = Ui::detect(true);
        let out = Out { json: true };
        let r = decide(
            &args(Some("acme.example"), false),
            None,
            DIR,
            false,
            ui,
            out,
        );
        assert!(matches!(r, Err(Error::Usage(_))));
    }

    #[test]
    fn pages_are_counted_the_way_the_summary_says() {
        let c = Crawl {
            pages: vec![
                page("https://a/1", 200, None),
                page("https://a/2", 301, None),
                page("https://a/3", 404, None),
                page("https://a/4", 0, Some("timeout")),
                page("https://a/5", 200, Some("too big")),
            ],
            hosts: vec![
                HostRow {
                    host: "a".into(),
                    disallowed: 2,
                },
                HostRow {
                    host: "b".into(),
                    disallowed: 1,
                },
            ],
            stopped_by: vec!["pages".into()],
            used: Used {
                seconds: 7,
                bytes: 0,
            },
            ..Crawl::default()
        };
        let n = c.counts();
        assert_eq!((n.read, n.failed, n.robots, n.seconds), (1, 3, 3, 7));
    }

    #[test]
    fn the_summary_ends_with_what_did_not_happen() {
        let n = Counts {
            read: 12,
            failed: 1,
            robots: 3,
            seconds: 9,
            stopped: vec![],
            not_followed: vec![],
        };
        let s = summary("acme.example", &n, 13, 1, DIR);
        assert!(
            s.starts_with("Crawl   acme.example   on this machine, by iohr-agent"),
            "{s}"
        );
        assert!(
            s.contains("read 12 pages in 9 s, 3 skipped by robots.txt, 1 failed"),
            "{s}"
        );
        assert!(s.contains("stopped: no limit reached"), "{s}");
        assert!(s.contains("facts: 13 in atlas/   memory: 1 entry"), "{s}");
        assert!(
            s.ends_with("Nothing was sent to InOrbit. Run iohr sync when you want it there."),
            "{s}"
        );
        let n = Counts {
            stopped: vec!["pages".into(), "minutes".into()],
            ..n
        };
        let s = summary("acme.example", &n, 13, 1, DIR);
        assert!(s.contains("stopped at the page limit and the time limit; raise it with --crawl-pages / --crawl-minutes"), "{s}");
    }

    #[test]
    fn keeping_a_crawl_writes_facts_a_study_and_a_memory_once() {
        let t = tempfile::tempdir().unwrap();
        let c = Crawl {
            start: "https://acme.example/".into(),
            pages: vec![
                page("https://acme.example/", 200, None),
                page("https://acme.example/x", 404, None),
            ],
            ..Crawl::default()
        };
        let name = "acme.example";
        std::fs::create_dir_all(t.path().join(DIR).join(name)).unwrap();
        let k = keep(t.path(), name, &c, "2026-11-08", None).unwrap();
        assert_eq!(k.facts, 2);
        assert_eq!(k.study, "atlas/studies/0001-site-crawl-acme-example.md");
        let facts =
            std::fs::read_to_string(t.path().join(DIR).join(name).join("facts.jsonl")).unwrap();
        assert_eq!(facts.lines().count(), 2);
        assert!(facts.contains("\"crawled\":\"2026-11-08\""));
        let again = keep(t.path(), name, &c, "2026-11-09", None).unwrap();
        assert_eq!(again.study, k.study, "a second crawl rewrites the study");
        let studies: Vec<_> = std::fs::read_dir(t.path().join("atlas/studies"))
            .unwrap()
            .collect();
        assert_eq!(studies.len(), 1);
        let mem = store::Store::project(t.path()).load().unwrap();
        assert_eq!(mem.len(), 1, "a second crawl confirms the same memory");
        assert!(mem[0].text.contains("acme.example"));
    }

    #[test]
    fn the_site_model_is_folded_into_the_facts_and_the_one_study() {
        let t = tempfile::tempdir().unwrap();
        let c = Crawl {
            start: "https://acme.example/".into(),
            pages: vec![page("https://acme.example/", 200, None)],
            ..Crawl::default()
        };
        let name = "acme.example";
        std::fs::create_dir_all(t.path().join(DIR).join(name)).unwrap();
        let model = crate::kb::Built {
            report: crate::kb::Report::default(),
            jsonl: "{\"kind\":\"entity\",\"name\":\"Acme\"}\n{\"kind\":\"fact\",\"value\":\"x\"}\n"
                .into(),
            study: "The model of acme.example.\n\n## Sections\n\n- `/a`: A\n".into(),
        };
        let k = keep(t.path(), name, &c, "2026-11-08", Some(&model)).unwrap();
        assert_eq!(k.facts, 3, "the page and the model's two lines");
        let dir = t.path().join(DIR).join(name);
        let facts = std::fs::read_to_string(dir.join("facts.jsonl")).unwrap();
        assert_eq!(facts.lines().count(), 3);
        assert!(facts.contains("\"kind\":\"entity\""));
        assert!(!dir.join("model.jsonl").exists(), "one facts file, not two");
        let studies: Vec<_> = std::fs::read_dir(t.path().join("atlas/studies"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(studies.len(), 1, "one study, not a second \"Site model\"");
        let text = std::fs::read_to_string(studies[0].path()).unwrap();
        let (model_at, pages_at) = (
            text.find("## Site model").unwrap(),
            text.find("## Pages").unwrap(),
        );
        assert!(model_at < pages_at, "{text}");
        assert!(text.contains("### Sections"), "{text}");
    }

    #[test]
    fn the_crawler_gets_the_budgets_and_the_policy() {
        let p = plan("https://acme.example/");
        let a: Vec<String> = crawler_args(
            p.url.as_str(),
            &Budget::of(&p),
            Path::new("/p/policy.toml"),
            Path::new("/p/crawl.json"),
            false,
            false,
        )
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
        let line = a.join(" ");
        assert!(
            line.starts_with(
                "atlas crawl https://acme.example/ --policy /p/policy.toml --out /p/crawl.json"
            ),
            "{line}"
        );
        assert!(line.contains("--max-pages 200"), "{line}");
        assert!(line.contains("--max-minutes 5"), "{line}");
        assert!(
            line.contains("--max-hosts 10 --max-minutes 5 --max-mb 50 --max-page-mb 2"),
            "{line}"
        );
    }

    #[test]
    fn a_long_crawl_lists_the_first_pages_and_points_at_the_rest() {
        let c = Crawl {
            pages: (0..LISTED + 3)
                .map(|i| page(&format!("https://a/{i}"), 200, None))
                .collect(),
            ..Crawl::default()
        };
        let t = study_text(&c, &c.counts(), "a", "2026-11-08", None, DIR);
        assert_eq!(t.matches("\n- https://a/").count(), LISTED);
        assert!(
            t.contains("3 more pages are in `.inorbit/crawl/a/facts.jsonl`"),
            "{t}"
        );
    }

    use std::cell::RefCell;

    /// Runs `explore` over `first_pass()` with scripted answers (by host); returns the crawl
    /// and the hosts a further pass was run for, each with its budget.
    async fn explored(
        p: &Plan,
        answers: &[(&str, Answer)],
        first: Crawl,
    ) -> (Crawl, Vec<(String, Budget)>, Vec<String>) {
        let passes = RefCell::new(Vec::new());
        let asked = RefCell::new(Vec::new());
        let crawl = explore(
            p,
            first,
            |h| {
                asked.borrow_mut().push(h.to_owned());
                answers
                    .iter()
                    .find(|(host, _)| *host == h)
                    .map_or(Answer::No, |(_, a)| *a)
            },
            |host, _start, b| {
                passes.borrow_mut().push((host.clone(), b));
                async move { Ok(pass_over(&host)) }
            },
        )
        .await;
        (crawl, passes.into_inner(), asked.into_inner())
    }

    #[test]
    fn only_names_the_policy_kept_out_inside_the_domain_are_offered() {
        let found = blocked_hosts(&first_pass(), "acme.example");
        let hosts: Vec<&str> = found.iter().map(|(h, _)| h.as_str()).collect();
        // robots.txt is not a policy block, other.example is outside the domain, and
        // blog.acme.example is offered once, at its first address.
        assert_eq!(hosts, ["shop.acme.example", "blog.acme.example"]);
        assert_eq!(found[1].1, "https://blog.acme.example/a");
    }

    #[test]
    fn a_redirect_the_policy_refused_offers_the_name_it_went_to() {
        // `www.` redirects to the apex: the entry's URL is the page that redirected, the reason
        // names the refused host (the text the real crawler wrote for www.rust-lang.org).
        let c = Crawl {
            scope: "rust-lang.org".into(),
            blocked: vec![blocked(
                "https://www.rust-lang.org/",
                "policy: refused: rust-lang.org:443: rust-lang.org is neither named in networks.allow nor inside a bound domain",
            )],
            ..Crawl::default()
        };
        assert_eq!(
            blocked_hosts(&c, "www.rust-lang.org"),
            [(
                "rust-lang.org".to_owned(),
                "https://rust-lang.org/".to_owned()
            )]
        );
    }

    #[tokio::test]
    async fn a_subdomain_is_not_followed_without_a_yes() {
        let p = plan("https://acme.example/");
        let (crawl, passes, asked) = explored(&p, &[], first_pass()).await;
        assert_eq!(
            asked,
            ["shop.acme.example", "blog.acme.example"],
            "asked once each"
        );
        assert!(passes.is_empty(), "no pass was run: {passes:?}");
        assert_eq!(crawl.pages.len(), 1);
        assert_eq!(
            crawl.not_followed,
            ["shop.acme.example", "blog.acme.example"]
        );
        let s = summary("acme.example", &crawl.counts(), 1, 1, DIR);
        assert!(
            s.contains("not read: shop.acme.example, blog.acme.example"),
            "{s}"
        );
    }

    #[tokio::test]
    async fn a_subdomain_is_followed_after_a_yes_and_only_that_one() {
        let p = plan("https://acme.example/");
        let (crawl, passes, _) =
            explored(&p, &[("shop.acme.example", Answer::Yes)], first_pass()).await;
        let hosts: Vec<&str> = passes.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(hosts, ["shop.acme.example"]);
        assert_eq!(crawl.pages.len(), 2);
        assert_eq!(crawl.not_followed, ["blog.acme.example"]);
        assert_eq!(crawl.used.seconds, 5, "the passes' time adds up");
        assert!(crawl.complete);
        // The further pass gets what is left: 199 pages, 9 hosts, 5 minutes (3 s used).
        let b = &passes[0].1;
        assert_eq!((b.pages, b.hosts, b.minutes, b.megabytes), (199, 9, 5, 50));
    }

    #[tokio::test]
    async fn yes_to_all_follows_every_name_and_asks_once() {
        let p = plan("https://acme.example/");
        let (crawl, passes, asked) =
            explored(&p, &[("shop.acme.example", Answer::All)], first_pass()).await;
        assert_eq!(asked, ["shop.acme.example"]);
        assert_eq!(passes.len(), 2);
        assert_eq!(crawl.pages.len(), 3);
        assert_eq!(crawl.not_followed, Vec::<String>::new());
    }

    #[tokio::test]
    async fn the_limits_cover_every_pass_together() {
        let mut p = plan("https://acme.example/");
        p.pages = 2;
        let (crawl, passes, _) =
            explored(&p, &[("shop.acme.example", Answer::All)], first_pass()).await;
        // One page from the first pass, one from shop; the page limit is then used up, so
        // blog is left out and the limit is named.
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].1.pages, 1);
        assert_eq!(crawl.not_followed, ["blog.acme.example"]);
        assert!(crawl.stopped_by.contains(&"pages".to_owned()));
    }

    #[tokio::test]
    async fn a_pass_that_fails_leaves_the_name_out_and_the_rest_stands() {
        let p = plan("https://acme.example/");
        let crawl = explore(
            &p,
            first_pass(),
            |_| Answer::Yes,
            |_, _, _| async { Err("the crawler exited with 2".to_owned()) },
        )
        .await;
        assert_eq!(crawl.pages.len(), 1);
        assert_eq!(
            crawl.not_followed,
            ["shop.acme.example", "blog.acme.example"]
        );
    }

    #[tokio::test]
    async fn a_followed_name_can_offer_its_own_neighbours() {
        let p = plan("https://acme.example/");
        let first = Crawl {
            blocked: vec![blocked(
                "https://a.acme.example/",
                "policy: host not allowed",
            )],
            ..first_pass()
        };
        let crawl = explore(
            &p,
            first,
            |_| Answer::Yes,
            |host, _, _| async move {
                let mut c = pass_over(&host);
                if host == "a.acme.example" {
                    c.blocked = vec![
                        blocked("https://b.acme.example/", "policy: host not allowed"),
                        blocked("https://a.acme.example/", "policy: host not allowed"),
                    ];
                }
                Ok(c)
            },
        )
        .await;
        let hosts: Vec<&str> = crawl.hosts.iter().map(|h| h.host.as_str()).collect();
        assert_eq!(hosts, ["acme.example", "a.acme.example", "b.acme.example"]);
    }
}
