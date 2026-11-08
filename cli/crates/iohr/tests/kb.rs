//! The local site knowledge base end to end (PRD 0192): a fixture site on a port-0 mock server
//! is read, cleaned, chunked, indexed and modelled by `iohr::kb::build`, then `iohr atlas
//! search|facts|index` (the binary, a private HOME) answer from the index with their sources.
//! Nothing leaves the machine; the fixture is a local server.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use support::kept_env;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn page(title: &str, body: &str) -> ResponseTemplate {
    let html = format!(
        "<html lang=\"en\"><head><title>{title}</title><meta property=\"og:site_name\" content=\"Acme Robotics\">\
<script type=\"application/ld+json\">{{\"@type\":\"Organization\",\"name\":\"Acme Robotics\"}}</script></head>\
<body><nav><a href=\"/\">Home</a> <a href=\"/pricing\">Pricing</a></nav><main>{body}</main>\
<footer>All rights reserved</footer></body></html>"
    );
    ResponseTemplate::new(200).set_body_raw(html.into_bytes(), "text/html; charset=utf-8")
}

fn iohr(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .env_clear()
        .current_dir(cwd)
        .args(args)
        .envs(kept_env())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("NO_COLOR", "1")
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

async fn site() -> MockServer {
    let server = MockServer::start().await;
    let pages = [
        (
            "/",
            "Acme",
            "<h1>Acme Robotics</h1><p>We build warehouse picking robots for logistics teams.</p>",
        ),
        (
            "/pricing",
            "Pricing",
            "<h1>Pricing</h1><h2>Picker plans</h2><p>The Picker robot costs $1,299.00 per month, with service included.</p>",
        ),
        (
            "/contact",
            "Contact",
            "<h1>Contact</h1><p>Write to sales@acme.test or call +49 30 1234 5678 for a demo.</p>",
        ),
    ];
    for (p, title, body) in pages {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(page(title, body))
            .mount(&server)
            .await;
    }
    server
}

#[tokio::test]
async fn a_crawled_site_is_indexed_searched_and_modelled_with_sources() {
    let server = site().await;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(root.join(".inorbit")).unwrap();
    // The project's root is the folder with atlas/, where init's study is written.
    std::fs::create_dir_all(root.join("atlas")).unwrap();

    let input: Vec<iohr::kb::PageIn> = ["/", "/pricing", "/contact"]
        .iter()
        .map(|p| iohr::kb::PageIn {
            url: format!("{}{p}", server.uri()),
            title: String::new(),
            depth: 0,
            rendered: false,
        })
        .collect();
    let opts = opts_for(&server);
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(&root, "acme.test", &input, &opts, &none)
        .await
        .unwrap();
    let report = &built.report;
    assert_eq!(
        (report.pages, report.fetched, report.cached, report.failed),
        (3, 3, 0, 0),
        "{built:?}"
    );
    assert!(
        report.chunks >= 3 && report.tokens > 20 && report.entities >= 4,
        "{built:?}"
    );
    assert!(report.index_error.is_none(), "{built:?}");
    assert!(root.join(".inorbit/crawl/kb.duckdb").is_file());
    assert!(report.lines("acme.test").contains("embeddings: stub"));

    // Search, as the person asks: the pricing chunk is first, with its source.
    let o = iohr(
        &home,
        &root,
        &["atlas", "search", "how much does the picker cost", "--json"],
    );
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let first = &v["hits"][0];
    assert!(first["url"].as_str().unwrap().ends_with("/pricing"), "{v}");
    assert!(first["text"].as_str().unwrap().contains("$1,299.00"), "{v}");
    assert!(first["chunk"].as_str().unwrap().starts_with("ch_"), "{v}");
    assert!(v["embeddings"].as_str().unwrap().starts_with("stub"), "{v}");

    // The same by `?` retrieval: sources are what the model is handed.
    let hits = iohr::kb::sources(&root, "how much does the picker cost", 3);
    assert!(hits[0].url.ends_with("/pricing"));
    let block = iohr::kb::sources_block(&hits);
    assert!(block.starts_with("[1] "), "{block}");
    assert!(block.contains(&hits[0].id));

    // Facts carry the chunk they came from.
    let o = iohr(
        &home,
        &root,
        &["atlas", "facts", "--predicate", "contact_email", "--json"],
    );
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["facts"][0]["value"], "sales@acme.test", "{v}");
    assert!(v["facts"][0]["chunk"].as_str().unwrap().starts_with("ch_"));
    assert!(v["facts"][0]["url"].as_str().unwrap().ends_with("/contact"));

    // The index lists the site, then its pages.
    let o = iohr(&home, &root, &["atlas", "index", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["sites"][0]["site"], "acme.test", "{v}");
    assert_eq!(v["sites"][0]["pages"], 3, "{v}");
    let o = iohr(
        &home,
        &root,
        &["atlas", "index", "--pages", "--site", "ACME.test"],
    );
    let listing = text(&o);
    assert!(
        listing.contains("/pricing") && listing.contains("/contact"),
        "{listing}"
    );

    // A person's table output works and an empty search says so.
    let o = iohr(&home, &root, &["atlas", "search", "zzzqqq"]);
    assert!(o.status.success());
    assert!(text(&o).contains("nothing in the knowledge base matches"));

    // The model is local and rides in the crawl's own fact shape: entities, sections and facts
    // as JSON lines for `facts.jsonl`, and its text for the one "Site crawl" study.
    for kind in ["entity", "section", "fact"] {
        assert!(
            built.jsonl.contains(&format!("\"kind\":\"{kind}\"")),
            "{kind} in {}",
            built.jsonl
        );
    }
    assert!(built.study.contains("## Entities"), "{}", built.study);
    assert!(
        !root.join(".inorbit/crawl/acme.test/model.jsonl").exists(),
        "no second model file"
    );

    // A second build reads nothing again: the pages come from the cache.
    let again = iohr::kb::build(&root, "acme.test", &input, &opts, &none)
        .await
        .unwrap();
    assert_eq!((again.report.fetched, again.report.cached), (0, 3));
    assert_eq!(again.report.chunks, built.report.chunks);
}

/// The options a test uses: the fixture's own host is the scope and nothing waits long.
fn opts_for(server: &MockServer) -> iohr::kb::Opts {
    iohr::kb::Opts {
        deadline: Duration::from_secs(30),
        rate: 50,
        scope: server.address().ip().to_string(),
        also: Vec::new(),
        redirects: Vec::new(),
        crawled: "2026-11-08".into(),
    }
}

fn input_of(server: &MockServer, paths: &[&str]) -> Vec<iohr::kb::PageIn> {
    paths
        .iter()
        .map(|p| iohr::kb::PageIn {
            url: format!("{}{p}", server.uri()),
            title: String::new(),
            depth: 0,
            rendered: false,
        })
        .collect()
}

#[tokio::test]
async fn pages_the_crawler_kept_are_read_without_asking_the_site() {
    // A site that answers nothing: any request would be a 404 and a recorded request.
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    let dir = root.join(".inorbit/crawl/acme.test");
    let input = input_of(&server, &["/", "/pricing"]);
    for (p, body) in [
        (
            "/",
            "<h1>Acme</h1><p>Warehouse robots for logistics teams.</p>",
        ),
        (
            "/pricing",
            "<h1>Pricing</h1><p>The Picker costs $1,299.00 per month.</p>",
        ),
    ] {
        let file = iohr::kb::fetch::cache_path(&dir, &format!("{}{p}", server.uri()));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            file,
            format!("<html><title>{p}</title><body>{body}</body></html>"),
        )
        .unwrap();
    }
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(&root, "acme.test", &input, &opts_for(&server), &none)
        .await
        .unwrap();
    assert_eq!(
        (
            built.report.cached,
            built.report.fetched,
            built.report.pages
        ),
        (2, 0, 2),
        "{built:?}"
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the agent kept the pages, so the site is not asked again"
    );
    assert!(
        built
            .report
            .lines("acme.test")
            .contains("from files on this machine")
    );
}

#[tokio::test]
async fn a_page_without_a_file_is_requested_within_robots_and_scope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/robots.txt"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /private\n"),
        )
        .mount(&server)
        .await;
    for (p, body) in [
        ("/open", "<h1>Open</h1><p>Anyone may read this page.</p>"),
        ("/private/x", "<h1>Private</h1><p>Secret page text.</p>"),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(page("T", body))
            .mount(&server)
            .await;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    let mut input = input_of(&server, &["/open", "/private/x"]);
    input.push(iohr::kb::PageIn {
        url: "http://elsewhere.invalid/z".into(),
        title: String::new(),
        depth: 1,
        rendered: false,
    });
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(&root, "acme.test", &input, &opts_for(&server), &none)
        .await
        .unwrap();
    let r = &built.report;
    assert_eq!((r.fetched, r.refused, r.pages), (1, 2, 1), "{built:?}");
    assert!(r.lines("acme.test").contains("2 not requested"));
    let asked: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|q| q.url.path().to_owned())
        .collect();
    assert!(asked.contains(&"/open".to_owned()), "{asked:?}");
    assert!(
        !asked.iter().any(|p| p.starts_with("/private")),
        "{asked:?}"
    );
}

#[tokio::test]
async fn a_deadline_that_has_passed_leaves_the_rest_out_and_says_so() {
    let server = site().await;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    let mut opts = opts_for(&server);
    opts.deadline = Duration::ZERO;
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(
        &root,
        "acme.test",
        &input_of(&server, &["/", "/pricing", "/contact"]),
        &opts,
        &none,
    )
    .await
    .unwrap();
    assert_eq!((built.report.late, built.report.pages), (3, 0), "{built:?}");
    assert!(
        built
            .report
            .lines("acme.test")
            .contains("3 left out when the time ran out")
    );
}

#[tokio::test]
async fn a_link_to_a_redirected_address_ends_at_the_page_it_went_to() {
    let server = site().await;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    let mut opts = opts_for(&server);
    opts.redirects = vec![(
        format!("{}/old-pricing", server.uri()),
        format!("{}/pricing", server.uri()),
    )];
    let input = input_of(&server, &["/pricing"]);
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(&root, "acme.test", &input, &opts, &none)
        .await
        .unwrap();
    assert_eq!(built.report.pages, 1, "{built:?}");
}

#[tokio::test]
async fn a_rendered_page_that_has_no_text_is_counted_as_a_shell() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    let dir = root.join(".inorbit/crawl/acme.test");
    let url = format!("{}/app", server.uri());
    let file = iohr::kb::fetch::cache_path(&dir, &url);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        file,
        "<html><body><div id=\"root\"></div><script src=\"/a.js\"></script></body></html>",
    )
    .unwrap();
    let input = vec![iohr::kb::PageIn {
        url,
        title: String::new(),
        depth: 0,
        rendered: true,
    }];
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(&root, "acme.test", &input, &opts_for(&server), &none)
        .await
        .unwrap();
    assert_eq!(built.report.shells, 1, "{built:?}");
    assert!(built.report.lines("acme.test").contains("empty shell"));
}

#[test]
fn without_an_index_search_says_how_to_make_one() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(root.join(".inorbit")).unwrap();
    std::fs::create_dir_all(root.join("atlas")).unwrap();
    let o = iohr(&home, &root, &["atlas", "search", "anything"]);
    assert!(!o.status.success());
    assert!(
        text(&o).contains("iohr project init --crawl"),
        "{}",
        text(&o)
    );
    // `?` never fails for a missing index: it just adds nothing.
    assert!(iohr::kb::sources(&root, "anything", 5).is_empty());
}
