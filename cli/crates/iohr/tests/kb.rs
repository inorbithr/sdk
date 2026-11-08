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

    let input: Vec<iohr::kb::PageIn> = ["/", "/pricing", "/contact"]
        .iter()
        .map(|p| iohr::kb::PageIn {
            url: format!("{}{p}", server.uri()),
            title: String::new(),
            depth: 0,
        })
        .collect();
    let opts = iohr::kb::Opts {
        deadline: Duration::from_secs(30),
        pace: Duration::ZERO,
        crawled: "2026-11-08".into(),
    };
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(&root, "acme.test", &input, &opts, &none)
        .await
        .unwrap();
    assert_eq!(
        (built.pages, built.fetched, built.cached, built.failed),
        (3, 3, 0, 0),
        "{built:?}"
    );
    assert!(
        built.chunks >= 3 && built.tokens > 20 && built.entities >= 4,
        "{built:?}"
    );
    assert!(built.index_error.is_none(), "{built:?}");
    assert!(root.join(".inorbit/crawl/kb.duckdb").is_file());
    assert!(built.lines("acme.test").contains("embeddings: stub"));

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

    // The model is local: the model file lists entities, sections and facts, in fact shape.
    let model_file =
        std::fs::read_to_string(root.join(".inorbit/crawl/acme.test/model.jsonl")).unwrap();
    for kind in ["entity", "section", "fact"] {
        assert!(
            model_file.contains(&format!("\"kind\":\"{kind}\"")),
            "{kind} in {model_file}"
        );
    }

    // A second build reads nothing again: the pages come from the cache.
    let again = iohr::kb::build(&root, "acme.test", &input, &opts, &none)
        .await
        .unwrap();
    assert_eq!((again.fetched, again.cached), (0, 3));
    assert_eq!(again.chunks, built.chunks);
}

#[test]
fn without_an_index_search_says_how_to_make_one() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(root.join(".inorbit")).unwrap();
    let o = iohr(&home, &root, &["atlas", "search", "anything"]);
    assert!(!o.status.success());
    assert!(!text(&o).is_empty(), "the failure says why");
    // `?` never fails for a missing index: it just adds nothing.
    assert!(iohr::kb::sources(&root, "anything", 5).is_empty());
}
