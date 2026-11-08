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

/// The extension that holds the crawler.
const EXTENSION: &str = "agent";
/// Where a crawl keeps its files, under the project.
const DIR: &str = ".inorbit/crawl";
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
}

impl Plan {
    /// What a dry run says it would do.
    pub(crate) fn would(&self) -> String {
        format!(
            "read {} on this machine with iohr-agent (up to {} pages or {} minutes) into {DIR}/{}/, then write atlas facts and one memory entry; nothing is sent to InOrbit",
            self.url.as_str(),
            self.pages,
            self.minutes,
            self.name
        )
    }
}

/// The consent init shows before the first request.
pub(crate) fn consent(p: &Plan) -> String {
    let host = p.url.host_str().unwrap_or_default();
    format!(
        "This reads the public pages of {host} and its subdomains on this machine, with iohr-agent: at most one request a second, robots.txt followed, up to {} pages or {} minutes. What it finds stays in this project, under {DIR}/{}/ and atlas/. Nothing is sent to InOrbit.",
        p.pages, p.minutes, p.name
    )
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

/// The policy the agent runs under: this host and its subdomains, nothing else. An address
/// that is an IP or `localhost` has no subdomains, so it is the one name.
fn policy_text(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let names =
        if host == "localhost" || host.parse::<std::net::IpAddr>().is_ok() || host.contains(':') {
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
    let plan = Plan {
        name: folder_name(&url),
        url,
        pages: args.crawl_pages.clamp(1, MAX_PAGES),
        minutes: args.crawl_minutes.clamp(1, MAX_MINUTES),
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
    pub used: Used,
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
}

impl Crawl {
    pub(crate) fn counts(&self) -> Counts {
        Counts {
            read: self.pages.iter().filter(|p| p.read()).count(),
            failed: self.pages.iter().filter(|p| p.failed()).count(),
            robots: self.hosts.iter().map(|h| h.disallowed).sum(),
            seconds: self.used.seconds,
            stopped: self.stopped_by.clone(),
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

/// The study's text: what was read, then one line per page, up to [`LISTED`].
fn study_text(c: &Crawl, n: &Counts, name: &str, crawled: &str) -> String {
    let mut t = String::new();
    let _ = writeln!(
        t,
        "Read on {crawled} on this machine by iohr-agent from {}. The crawler records no time per page, so every page below carries that day.\n",
        if c.start.is_empty() { name } else { &c.start }
    );
    let _ = writeln!(
        t,
        "- pages read: {}\n- pages that failed: {}\n- links skipped by robots.txt: {}\n- {}\n- the full record is in `{DIR}/{name}/` (`crawl.json`, `facts.jsonl`)\n",
        n.read,
        n.failed,
        n.robots,
        stopped_line(&n.stopped, false),
    );
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
            "\n{} more pages are in `{DIR}/{name}/facts.jsonl`.",
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
pub(crate) fn summary(name: &str, n: &Counts, facts: usize, memories: usize) -> String {
    let robots = n.robots;
    let failed = n.failed;
    format!(
        "Crawl   {name}   on this machine, by iohr-agent\n  read {} pages in {} s, {robots} skipped by robots.txt, {failed} failed\n  {}\n  facts: {facts} in atlas/   memory: {memories} entr{}   local files: {DIR}/{name}/\nNothing was sent to InOrbit. Run iohr sync when you want it there.",
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
pub(crate) fn keep(root: &Path, name: &str, c: &Crawl, crawled: &str) -> Result<Kept, String> {
    let n = c.counts();
    let dir = root.join(DIR).join(name);
    let mut facts = String::new();
    for p in &c.pages {
        facts.push_str(&fact_line(p, name, crawled));
        facts.push('\n');
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
        &study_text(c, &n, name, crawled),
    )?;
    let store = store::Store::project(root);
    let text = format!(
        "The public site {name} was crawled on this machine; its pages are in study {number:04} ({study}) and {DIR}/{name}/facts.jsonl."
    );
    store::remember(
        &store,
        &text,
        MemKind::Fact,
        "written by iohr project init after reading the site",
        &[study.clone(), format!("{DIR}/{name}/facts.jsonl")],
        false,
    )
    .map_err(|e| format!("cannot write the memory entry: {e}"))?;
    Ok(Kept {
        facts: c.pages.len(),
        study,
        memories: 1,
    })
}

/// The arguments of the crawl, as the agent extension reads them.
fn crawler_args(p: &Plan, policy: &Path, out: &Path) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = ["atlas", "crawl", p.url.as_str(), "--policy"]
        .map(Into::into)
        .into();
    a.push(policy.into());
    a.push("--out".into());
    a.push(out.into());
    for (flag, value) in [
        ("--max-pages", p.pages),
        ("--max-hosts", HOSTS),
        ("--max-minutes", p.minutes),
        ("--max-mb", MEGABYTES),
        ("--max-page-mb", PAGE_MEGABYTES),
    ] {
        a.push(flag.into());
        a.push(value.to_string().into());
    }
    a
}

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

/// Runs the crawl and keeps what it found. The report step says how it went; the summary is
/// printed here.
pub(crate) async fn run(
    g: &Global,
    env: &Env,
    ui: Ui,
    out: Out,
    interactive: bool,
    root: &Path,
    p: &Plan,
) -> Step {
    let bin = super::shell::running_bin();
    let id = format!("inorbit/{EXTENSION}");
    if !have_agent(g) {
        let line = format!("The crawler is part of the {id} extension, which is not installed.");
        let install = format!("{bin} ext install {id}");
        let yes = interactive
            && !out.json
            && ui
                .confirm(&format!("{line} Install it now?"), false)
                .unwrap_or(false);
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
    let dir: PathBuf = root.join(DIR).join(&p.name);
    if let Err(e) = prepare_dir(root, &dir, p) {
        return failed(e);
    }
    let crawl_json = dir.join("crawl.json");
    let sp = (!out.json).then(|| ui.step(&format!("Reading {} on this machine", p.name)));
    let store = match super::ext::store(g) {
        Ok(s) => s,
        Err(e) => return finish_failed(sp, e.to_string()),
    };
    let args = crawler_args(p, &dir.join("policy.toml"), &crawl_json);
    let limit = Duration::from_secs(u64::from(p.minutes) * 60 + 60);
    let ran = tokio::time::timeout(
        limit,
        crate::ext::run::run_quiet(g, env, &store, EXTENSION, &args),
    )
    .await;
    let quiet = match ran {
        Err(_) => {
            return finish_failed(
                sp,
                format!(
                    "the crawler did not finish in {} minutes and was stopped",
                    p.minutes + 1
                ),
            );
        }
        Ok(Err(e)) => return finish_failed(sp, e.to_string()),
        Ok(Ok(q)) => q,
    };
    // Exit 0 is a complete crawl and 1 one that a budget stopped: both leave the JSON.
    let parsed = std::fs::read(&crawl_json)
        .ok()
        .and_then(|b| serde_json::from_slice::<Crawl>(&b).ok());
    let crawl = match (quiet.code, parsed) {
        (0 | 1, Some(c)) => c,
        (code, _) => {
            let tail = quiet.stderr.trim();
            let said = if tail.is_empty() {
                format!("the crawler exited with {code} and left no result")
            } else {
                format!("the crawler exited with {code}: {}", one_line(tail))
            };
            return finish_failed(sp, said);
        }
    };
    let crawled = crate::front::today();
    let kept = match keep(root, &p.name, &crawl, &crawled) {
        Ok(k) => k,
        Err(e) => return finish_failed(sp, e),
    };
    let n = crawl.counts();
    let text = summary(&p.name, &n, kept.facts, kept.memories);
    if let Some(sp) = sp {
        sp.clear();
    }
    if !out.json {
        ui.note("Crawl", &text);
    }
    project::step(
        "crawl",
        "Crawl",
        "done",
        format!(
            "read {} pages of {} on this machine; {} facts in {}, 1 memory entry; nothing was sent to InOrbit",
            n.read, p.name, kept.facts, kept.study
        ),
        "",
    )
}

fn finish_failed(sp: Option<crate::ui::Step>, said: String) -> Step {
    if let Some(sp) = sp {
        sp.fail(&format!("The crawl failed: {said}"));
    }
    failed(said)
}

/// Makes the crawl's folder with its policy, and a `.gitignore` for the raw record so the
/// project's own git does not commit a crawl (the study and the memory entry are what the
/// project keeps). A result of an earlier run is removed, so a crawl that fails is never
/// read as the last one that worked.
fn prepare_dir(root: &Path, dir: &Path, p: &Plan) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let ignore = root.join(DIR).join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n")
            .map_err(|e| format!("cannot write {}: {e}", ignore.display()))?;
    }
    let policy = dir.join("policy.toml");
    std::fs::write(&policy, policy_text(&p.url))
        .map_err(|e| format!("cannot write {}: {e}", policy.display()))?;
    let _ = std::fs::remove_file(dir.join("crawl.json"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(crawl: Option<&str>, no_crawl: bool) -> InitCrawlArgs {
        InitCrawlArgs {
            crawl: crawl.map(str::to_owned),
            no_crawl,
            crawl_pages: 200,
            crawl_minutes: 5,
        }
    }

    fn plan(url: &str) -> Plan {
        let url = parse_site(url).expect("url");
        Plan {
            name: folder_name(&url),
            url,
            pages: 200,
            minutes: 5,
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
    fn the_policy_allows_the_host_and_its_subdomains_only() {
        let p = policy_text(&parse_site("https://acme.example/").unwrap());
        assert!(
            p.contains("allow = [\"acme.example\", \"*.acme.example\"]"),
            "{p}"
        );
        let l = policy_text(&parse_site("http://127.0.0.1:9/").unwrap());
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
            true,
            ui,
            out,
        );
        assert_eq!(r.unwrap(), None);
    }

    #[test]
    fn the_flag_is_consent_and_the_budgets_are_clamped() {
        let ui = Ui::detect(true);
        let out = Out { json: true };
        let mut a = args(Some("https://acme.example/"), false);
        a.crawl_pages = 1_000_000;
        a.crawl_minutes = 0;
        let p = decide(&a, None, false, ui, out).unwrap().unwrap();
        assert_eq!((p.pages, p.minutes), (MAX_PAGES, 1));
        assert!(consent(&p).contains("Nothing is sent to InOrbit"));
    }

    #[test]
    fn a_bad_address_is_a_usage_error() {
        let ui = Ui::detect(true);
        let out = Out { json: true };
        let r = decide(&args(Some("acme.example"), false), None, false, ui, out);
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
            used: Used { seconds: 7 },
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
        };
        let s = summary("acme.example", &n, 13, 1);
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
        let s = summary("acme.example", &n, 13, 1);
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
        let k = keep(t.path(), name, &c, "2026-11-08").unwrap();
        assert_eq!(k.facts, 2);
        assert_eq!(k.study, "atlas/studies/0001-site-crawl-acme-example.md");
        let facts =
            std::fs::read_to_string(t.path().join(DIR).join(name).join("facts.jsonl")).unwrap();
        assert_eq!(facts.lines().count(), 2);
        assert!(facts.contains("\"crawled\":\"2026-11-08\""));
        let again = keep(t.path(), name, &c, "2026-11-09").unwrap();
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
    fn the_crawler_gets_the_budgets_and_the_policy() {
        let p = plan("https://acme.example/");
        let a: Vec<String> =
            crawler_args(&p, Path::new("/p/policy.toml"), Path::new("/p/crawl.json"))
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
        let t = study_text(&c, &c.counts(), "a", "2026-11-08");
        assert_eq!(t.matches("\n- https://a/").count(), LISTED);
        assert!(
            t.contains("3 more pages are in `.inorbit/crawl/a/facts.jsonl`"),
            "{t}"
        );
    }
}
