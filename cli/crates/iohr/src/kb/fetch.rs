//! The text of the crawled pages. The crawler (iohr-agent) reads every page once and, with
//! `--keep-pages`, leaves its HTML in `pages/<hash>.html` beside its result; this reads those
//! files and requests a page only when no file is there (an agent from before the flag, a
//! page an earlier run did not keep). A file found is never fetched again, until it is a week
//! old, so a second `init` over the same site makes no request for it.
//!
//! The pass that does request is a guest like the crawler: only pages inside the crawl's scope,
//! only paths the host's `robots.txt` allows (one read of it per host), one request at a time
//! per host, no faster than the plan's `--crawl-rate` and no faster than a `Crawl-delay`,
//! with the plan's minutes as its deadline. No redirect is followed (the crawler records each
//! as its own page, and its target is read as that page), a body over 2 MB is refused, and
//! only URLs the crawler read with a 200 answer and an HTML type are asked for.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use url::Url;

use super::robots::Rules;
use crate::commands::upgrade::{fetch, http_builder};
use crate::front;

/// The most a page's body may be.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
/// The most `robots.txt` may be.
const MAX_ROBOTS: usize = 512 * 1024;
/// A file older than this is read again.
const MAX_AGE: Duration = Duration::from_hours(7 * 24);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(20);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
/// The name `robots.txt` rules are read under.
const AGENT: &str = "iohr";

/// Where to look and what to respect.
#[derive(Debug, Clone)]
pub struct Reach<'a> {
    /// The site's folder: its `pages/` holds the files.
    pub dir: &'a Path,
    /// Other folders with a `pages/` the crawler kept (a pass over a subdomain has its own).
    pub also: &'a [PathBuf],
    /// The crawl's scope, a registrable domain; a page on another host is not requested.
    pub scope: &'a str,
    /// The most requests a second to one host (the plan's `--crawl-rate`).
    pub rate: u32,
    /// The most time the requests may take.
    pub deadline: Duration,
}

/// One page whose HTML is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Got {
    pub url: String,
    pub path: PathBuf,
    /// Requested by this pass, not found as a file.
    pub read_now: bool,
}

/// What the pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Pages whose HTML is on disk now, in the order asked for.
    pub pages: Vec<Got>,
    /// Pages found as files, the crawler's or an earlier run's.
    pub cached: usize,
    /// Pages requested now.
    pub fetched: usize,
    /// Pages that did not come, with why.
    pub failed: Vec<(String, String)>,
    /// Pages not requested because the scope or the host's `robots.txt` says no.
    pub refused: usize,
    /// Pages left out because the deadline came first.
    pub late: usize,
}

/// Where the HTML of `url` is kept under `dir`: the name the crawler gives it too (the first 16
/// hex characters of the SHA-256 of the URL).
#[must_use]
pub fn cache_path(dir: &Path, url: &str) -> PathBuf {
    let name: String = front::sha_hex(url).chars().take(16).collect();
    dir.join("pages").join(format!("{name}.html"))
}

fn fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < MAX_AGE)
}

fn in_scope(url: &Url, scope: &str) -> bool {
    let scope = scope.trim_start_matches("www.").to_ascii_lowercase();
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some_and(|h| {
            let h = h.to_ascii_lowercase();
            scope.is_empty() || h == scope || h.ends_with(&format!(".{scope}"))
        })
}

/// What one host has said and when it was last asked.
struct Host {
    rules: Rules,
    last: Option<Instant>,
}

/// The requests of one pass.
struct Guest {
    http: Option<Result<reqwest::Client, String>>,
    hosts: HashMap<String, Host>,
    started: Instant,
    rate: u32,
    deadline: Duration,
}

impl Guest {
    fn client(&mut self, url: &str) -> Result<&reqwest::Client, String> {
        self.http
            .get_or_insert_with(|| {
                http_builder(url, CONNECT_TIMEOUT, Some(READ_TIMEOUT), TOTAL_TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| format!("cannot start the HTTP client: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Waits until `key`'s host may be asked again; `false` when the deadline would pass first.
    async fn wait(&mut self, key: &str, delay: Option<Duration>) -> bool {
        let gap = Duration::from_secs(1) / self.rate.max(1);
        let gap = delay.map_or(gap, |d| d.max(gap));
        let Some(last) = self.hosts.get(key).and_then(|h| h.last) else {
            return self.started.elapsed() < self.deadline;
        };
        let wait = gap.saturating_sub(last.elapsed());
        if self.started.elapsed() + wait >= self.deadline {
            return false;
        }
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        true
    }

    fn touch(&mut self, key: &str) {
        if let Some(h) = self.hosts.get_mut(key) {
            h.last = Some(Instant::now());
        }
    }

    /// The rules of `url`'s host, read once.
    async fn rules(&mut self, url: &Url) -> Result<Rules, String> {
        let key = host_key(url);
        if let Some(h) = self.hosts.get(&key) {
            return Ok(h.rules.clone());
        }
        let robots = format!("{}://{key}/robots.txt", url.scheme());
        let got = {
            let c = self.client(&robots)?.clone();
            fetch(&c, &robots, MAX_ROBOTS).await
        };
        let rules = match got {
            Ok(r) if (200..300).contains(&r.status) => {
                Rules::parse(&String::from_utf8_lossy(&r.body), AGENT)
            }
            Ok(r) if (400..500).contains(&r.status) => Rules::open(),
            _ => Rules::shut(),
        };
        self.hosts.insert(
            key,
            Host {
                rules: rules.clone(),
                last: Some(Instant::now()),
            },
        );
        Ok(rules)
    }

    /// One page's body, or why not.
    async fn page(&mut self, url: &str) -> Result<Vec<u8>, String> {
        let c = self.client(url)?.clone();
        let r = fetch(&c, url, MAX_BYTES).await.map_err(|e| {
            let said = e.to_string();
            if said.contains("larger than") {
                "over 2 MB".to_owned()
            } else if said.contains("timed out") {
                "timed out".to_owned()
            } else {
                "no answer".to_owned()
            }
        })?;
        if r.status != 200 {
            return Err(format!("answered {}", r.status));
        }
        if !r.content_type.is_empty() && !r.content_type.contains("html") {
            return Err("not HTML".to_owned());
        }
        Ok(r.body)
    }
}

/// `host` with its port, as a key and as the authority of the host's `robots.txt` URL.
fn host_key(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host,
    }
}

/// The file of `url`, when one is on disk and fresh.
fn found(r: &Reach<'_>, url: &str) -> Option<PathBuf> {
    std::iter::once(r.dir)
        .chain(r.also.iter().map(PathBuf::as_path))
        .map(|d| cache_path(d, url))
        .find(|p| fresh(p))
}

/// Makes sure the HTML of every URL in `urls` is on disk: the file when there is one, else one
/// request, within the scope, the host's `robots.txt` and the rate, until the deadline has
/// passed. `progress` is told `(done, total)` after each page.
pub async fn collect(
    r: &Reach<'_>,
    urls: &[String],
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Fetched {
    let mut out = Fetched::default();
    let total = urls.len();
    let mut guest = Guest {
        http: None,
        hosts: HashMap::new(),
        started: Instant::now(),
        rate: r.rate,
        deadline: r.deadline,
    };
    for (i, url) in urls.iter().enumerate() {
        match one(r, &mut guest, url, &mut out).await {
            Step::Done | Step::Skipped => progress(i + 1, total),
            Step::Late => out.late += 1,
        }
    }
    out
}

enum Step {
    Done,
    Skipped,
    Late,
}

async fn one(r: &Reach<'_>, g: &mut Guest, url: &str, out: &mut Fetched) -> Step {
    if let Some(path) = found(r, url) {
        out.cached += 1;
        out.pages.push(Got {
            url: url.to_owned(),
            path,
            read_now: false,
        });
        return Step::Done;
    }
    let Some(parsed) = Url::parse(url).ok().filter(|u| in_scope(u, r.scope)) else {
        out.refused += 1;
        out.failed
            .push((url.to_owned(), "outside the crawl's scope".to_owned()));
        return Step::Skipped;
    };
    let key = host_key(&parsed);
    if g.started.elapsed() >= g.deadline {
        return Step::Late;
    }
    // The host's rules come first (one request, itself paced), then the page.
    if !g.hosts.contains_key(&key) && !g.wait(&key, None).await {
        return Step::Late;
    }
    let rules = match g.rules(&parsed).await {
        Ok(rules) => rules,
        Err(why) => {
            out.failed.push((url.to_owned(), why));
            return Step::Skipped;
        }
    };
    let mut target = parsed.path().to_owned();
    if let Some(q) = parsed.query() {
        target.push('?');
        target.push_str(q);
    }
    if !rules.allows(&target) {
        out.refused += 1;
        out.failed
            .push((url.to_owned(), "robots.txt does not allow it".to_owned()));
        return Step::Skipped;
    }
    if !g.wait(&key, rules.delay).await {
        return Step::Late;
    }
    let got = g.page(url).await;
    g.touch(&key);
    match got {
        Ok(body) => {
            let path = cache_path(r.dir, url);
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, body));
            match written {
                Ok(()) => {
                    out.fetched += 1;
                    out.pages.push(Got {
                        url: url.to_owned(),
                        path,
                        read_now: true,
                    });
                }
                Err(e) => out
                    .failed
                    .push((url.to_owned(), format!("cannot keep it: {e}"))),
            }
        }
        Err(why) => out.failed.push((url.to_owned(), why)),
    }
    Step::Done
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn html(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_raw(body.as_bytes().to_vec(), "text/html; charset=utf-8")
    }

    fn reach<'a>(dir: &'a Path, also: &'a [PathBuf], rate: u32, deadline: Duration) -> Reach<'a> {
        Reach {
            dir,
            also,
            scope: "",
            rate,
            deadline,
        }
    }

    async fn robots(server: &MockServer, text: &str) {
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(text.to_owned()))
            .mount(server)
            .await;
    }

    fn urls(server: &MockServer, names: &[&str]) -> Vec<String> {
        names
            .iter()
            .map(|p| format!("{}/{p}", server.uri()))
            .collect()
    }

    const NONE: &(dyn Fn(usize, usize) + Sync) = &|_, _| {};
    const LONG: Duration = Duration::from_secs(30);

    #[tokio::test]
    async fn a_page_is_read_once_and_then_comes_from_the_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(html("<p>hello</p>"))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let u = urls(&server, &["a"]);
        let r = reach(dir.path(), &[], 50, LONG);
        let first = collect(&r, &u, NONE).await;
        assert_eq!((first.fetched, first.cached), (1, 0));
        assert!(first.pages[0].read_now);
        assert_eq!(
            std::fs::read_to_string(&first.pages[0].path).unwrap(),
            "<p>hello</p>"
        );
        let second = collect(&r, &u, NONE).await;
        assert_eq!((second.fetched, second.cached), (0, 1));
        assert!(!second.pages[0].read_now);
        server.verify().await;
    }

    #[tokio::test]
    async fn a_page_the_crawler_kept_is_read_from_its_file_with_no_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(html("<p>from the net</p>"))
            .expect(0)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let sub = tempfile::tempdir().unwrap();
        let u = urls(&server, &["a", "b"]);
        // The crawler's names: its own folder, and the folder of a pass over a subdomain.
        for (d, url, text) in [
            (dir.path(), &u[0], "<p>one</p>"),
            (sub.path(), &u[1], "<p>two</p>"),
        ] {
            let p = cache_path(d, url);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let also = [sub.path().to_path_buf()];
        let got = collect(&reach(dir.path(), &also, 50, LONG), &u, NONE).await;
        assert_eq!((got.fetched, got.cached, got.pages.len()), (0, 2, 2));
        assert_eq!(
            std::fs::read_to_string(&got.pages[1].path).unwrap(),
            "<p>two</p>"
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn what_does_not_come_is_counted_with_its_reason() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gone"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/moved"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", "/a"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/img"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/png"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(html(&"x".repeat(MAX_BYTES + 10)))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let u = urls(&server, &["gone", "moved", "img", "big"]);
        let got = collect(&reach(dir.path(), &[], 50, LONG), &u, NONE).await;
        let why: Vec<&str> = got.failed.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(
            why,
            ["answered 404", "answered 301", "not HTML", "over 2 MB"]
        );
        assert!(got.pages.is_empty());
    }

    #[tokio::test]
    async fn the_deadline_leaves_the_rest_out_and_says_how_many() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(html("<p>x</p>"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let u = urls(&server, &["p0", "p1", "p2"]);
        let got = collect(&reach(dir.path(), &[], 50, Duration::ZERO), &u, NONE).await;
        assert_eq!((got.fetched, got.late), (0, 3));
    }

    #[tokio::test]
    async fn requests_keep_to_the_rate() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(html("<p>x</p>"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let u = urls(&server, &["p0", "p1", "p2"]);
        // 8 a second is 125 ms apart: robots.txt, then three pages, three gaps at least.
        let t = Instant::now();
        let got = collect(&reach(dir.path(), &[], 8, LONG), &u, NONE).await;
        assert_eq!(got.fetched, 3);
        assert!(
            t.elapsed() >= Duration::from_millis(3 * 125 - 20),
            "{:?}",
            t.elapsed()
        );
    }

    #[tokio::test]
    async fn a_path_robots_disallows_is_never_requested() {
        let server = MockServer::start().await;
        robots(&server, "User-agent: *\nDisallow: /private\n").await;
        Mock::given(method("GET"))
            .and(path("/private/plans"))
            .respond_with(html("<p>secret</p>"))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/open"))
            .respond_with(html("<p>fine</p>"))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let u = urls(&server, &["private/plans", "open"]);
        let got = collect(&reach(dir.path(), &[], 50, LONG), &u, NONE).await;
        assert_eq!((got.fetched, got.refused), (1, 1));
        assert_eq!(got.failed[0].1, "robots.txt does not allow it");
        server.verify().await;
    }

    #[tokio::test]
    async fn a_robots_file_that_cannot_be_read_closes_the_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(html("<p>x</p>"))
            .expect(0)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let got = collect(
            &reach(dir.path(), &[], 50, LONG),
            &urls(&server, &["a"]),
            NONE,
        )
        .await;
        assert_eq!((got.fetched, got.refused), (0, 1));
        server.verify().await;
    }

    #[tokio::test]
    async fn a_slower_crawl_delay_wins_over_the_rate() {
        let server = MockServer::start().await;
        robots(&server, "User-agent: *\nCrawl-delay: 0.3\n").await;
        Mock::given(method("GET"))
            .respond_with(html("<p>x</p>"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let u = urls(&server, &["p0", "p1", "p2"]);
        // 50 a second alone is 20 ms; the 0.3 s delay makes the three pages and robots.txt
        // at least 0.9 s.
        let t = Instant::now();
        let got = collect(&reach(dir.path(), &[], 50, LONG), &u, NONE).await;
        assert_eq!(got.fetched, 3);
        assert!(
            t.elapsed() >= Duration::from_millis(850),
            "{:?}",
            t.elapsed()
        );
    }

    #[tokio::test]
    async fn a_page_outside_the_scope_is_not_requested() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(html("<p>x</p>"))
            .expect(0)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let mut r = reach(dir.path(), &[], 50, LONG);
        r.scope = "acme.test";
        let got = collect(&r, &urls(&server, &["a"]), NONE).await;
        assert_eq!((got.fetched, got.refused), (0, 1));
        assert_eq!(got.failed[0].1, "outside the crawl's scope");
        assert!(in_scope(
            &Url::parse("https://docs.acme.test/x").unwrap(),
            "www.acme.test"
        ));
        assert!(!in_scope(
            &Url::parse("https://evilacme.test/x").unwrap(),
            "acme.test"
        ));
        assert!(!in_scope(
            &Url::parse("ftp://acme.test/x").unwrap(),
            "acme.test"
        ));
        server.verify().await;
    }
}
