//! The text of the crawled pages. The crawler (iohr-agent) records what it found about each
//! page, never the page, so this reads each HTML page it listed once and keeps the file in the
//! crawl's folder: `pages/<hash>.html`. A file that is there is never fetched again (until it
//! is a week old), so a second `init` over the same site makes no request for it.
//!
//! This is a second pass, after the crawl, and does not touch the crawl's own rate. It is
//! polite on purpose: one request at a time, one a second, the plan's minutes as its deadline,
//! no redirect followed (the crawler already resolved them), a body over 2 MB refused, and
//! only URLs the crawler read with a 200 answer and an HTML type. Every page was allowed by
//! the site's robots.txt when the crawler read it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::front;

/// The most a page's body may be.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
/// The wait between two requests.
pub const PACE: Duration = Duration::from_secs(1);
/// A cached page older than this is read again.
const MAX_AGE: Duration = Duration::from_hours(7 * 24);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// What the pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Pages whose HTML is on disk now, in the order asked for: the URL and the file.
    pub pages: Vec<(String, PathBuf)>,
    /// Pages found in the cache.
    pub cached: usize,
    /// Pages requested now.
    pub fetched: usize,
    /// Pages that did not come, with why.
    pub failed: Vec<(String, String)>,
    /// Pages left out because the deadline came first.
    pub late: usize,
}

/// Where the HTML of `url` is kept under `dir`.
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

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("iohr/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("cannot start the HTTP client: {e}"))
}

/// One page's body, or why not.
async fn get(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut r = http.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            "timed out".to_owned()
        } else {
            "no answer".to_owned()
        }
    })?;
    if r.status().as_u16() != 200 {
        return Err(format!("answered {}", r.status().as_u16()));
    }
    let html = r
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|t| t.to_ascii_lowercase().contains("html"));
    if !html {
        return Err("not HTML".to_owned());
    }
    let mut body = Vec::new();
    while let Some(part) = r
        .chunk()
        .await
        .map_err(|_| "the body broke off".to_owned())?
    {
        body.extend_from_slice(&part);
        if body.len() > MAX_BYTES {
            return Err("over 2 MB".to_owned());
        }
    }
    Ok(body)
}

/// Makes sure the HTML of every URL in `urls` is under `dir/pages/`: from the cache when it is
/// there, else one request each, `pace` apart, until `deadline` has passed. `progress` is told
/// `(done, total)` after each page.
pub async fn collect(
    dir: &Path,
    urls: &[String],
    deadline: Duration,
    pace: Duration,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Fetched {
    let mut out = Fetched::default();
    let started = Instant::now();
    let total = urls.len();
    let mut http: Option<Result<reqwest::Client, String>> = None;
    let mut last: Option<Instant> = None;
    for (i, url) in urls.iter().enumerate() {
        let path = cache_path(dir, url);
        if fresh(&path) {
            out.cached += 1;
            out.pages.push((url.clone(), path));
            progress(i + 1, total);
            continue;
        }
        if started.elapsed() >= deadline {
            out.late += 1;
            continue;
        }
        if let Some(t) = last {
            let wait = pace.saturating_sub(t.elapsed());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
        let client = http.get_or_insert_with(client);
        let got = match client {
            Ok(c) => get(c, url).await,
            Err(e) => Err(e.clone()),
        };
        last = Some(Instant::now());
        match got {
            Ok(body) => {
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, body));
                match written {
                    Ok(()) => {
                        out.fetched += 1;
                        out.pages.push((url.clone(), path));
                    }
                    Err(e) => out
                        .failed
                        .push((url.clone(), format!("cannot keep it: {e}"))),
                }
            }
            Err(why) => out.failed.push((url.clone(), why)),
        }
        progress(i + 1, total);
    }
    out
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
        let urls = vec![format!("{}/a", server.uri())];
        let none = |_: usize, _: usize| {};
        let first = collect(
            dir.path(),
            &urls,
            Duration::from_secs(30),
            Duration::ZERO,
            &none,
        )
        .await;
        assert_eq!((first.fetched, first.cached), (1, 0));
        assert_eq!(
            std::fs::read_to_string(&first.pages[0].1).unwrap(),
            "<p>hello</p>"
        );
        let second = collect(
            dir.path(),
            &urls,
            Duration::from_secs(30),
            Duration::ZERO,
            &none,
        )
        .await;
        assert_eq!((second.fetched, second.cached), (0, 1));
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
        let urls: Vec<String> = ["gone", "moved", "img", "big"]
            .iter()
            .map(|p| format!("{}/{p}", server.uri()))
            .collect();
        let none = |_: usize, _: usize| {};
        let got = collect(
            dir.path(),
            &urls,
            Duration::from_secs(30),
            Duration::ZERO,
            &none,
        )
        .await;
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
        let urls: Vec<String> = (0..3).map(|i| format!("{}/p{i}", server.uri())).collect();
        let none = |_: usize, _: usize| {};
        let got = collect(dir.path(), &urls, Duration::ZERO, Duration::ZERO, &none).await;
        assert_eq!((got.fetched, got.late), (0, 3));
    }

    #[tokio::test]
    async fn requests_are_paced() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(html("<p>x</p>"))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let urls: Vec<String> = (0..3).map(|i| format!("{}/p{i}", server.uri())).collect();
        let none = |_: usize, _: usize| {};
        let t = Instant::now();
        let got = collect(
            dir.path(),
            &urls,
            Duration::from_secs(30),
            Duration::from_millis(120),
            &none,
        )
        .await;
        assert_eq!(got.fetched, 3);
        assert!(
            t.elapsed() >= Duration::from_millis(240),
            "{:?}",
            t.elapsed()
        );
    }
}
