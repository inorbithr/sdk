//! The Atlas store (RFC 0218 slice 1): the claim and evidence tables live in the project's one
//! `.inorbit/crawl/kb.duckdb`, and every opener of that file goes through the safe-open helper
//! (owner-only folder, one writer, a lock a crash frees). Fixture sites are port-0 mock servers.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use duckdb::Connection;
use iohr::kb::index::Store;
use iohr_atlas_core::digest::ContentDigest;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// What today's (pre-migration) code wrote: the old tables, no migration table.
const OLD_TABLES: &str = "
CREATE TABLE pages(site TEXT, url TEXT, title TEXT, description TEXT, lang TEXT,
  depth INTEGER, chunks INTEGER, tokens INTEGER, links INTEGER);
CREATE TABLE chunks(id TEXT, site TEXT, url TEXT, ord INTEGER, heading TEXT,
  path TEXT, start_char INTEGER, end_char INTEGER, tokens INTEGER, est_model_tokens INTEGER,
  text TEXT);
CREATE TABLE terms(chunk_id TEXT, term TEXT, tf INTEGER);
CREATE TABLE entities(site TEXT, kind TEXT, name TEXT, mentions INTEGER,
  pages INTEGER, url TEXT, chunk TEXT, source TEXT);
CREATE TABLE facts(site TEXT, id TEXT, subject TEXT, predicate TEXT, value TEXT,
  url TEXT, chunk TEXT, source TEXT);
CREATE TABLE sections(site TEXT, name TEXT, pages INTEGER, chunks INTEGER,
  tokens INTEGER, summary TEXT);
CREATE TABLE links(site TEXT, from_url TEXT, to_url TEXT, text TEXT, internal BOOLEAN);
CREATE TABLE sites(site TEXT, crawled TEXT, built TEXT, embeddings TEXT, subject TEXT);
INSERT INTO pages VALUES ('a.test', 'https://a.test/', 'A', 'd', 'en', 0, 1, 5, 0);
INSERT INTO chunks VALUES ('c1', 'a.test', 'https://a.test/', 0, 'H', 'H', 0, 20, 5, 6, 'We build robots.');
INSERT INTO terms VALUES ('c1', 'robot', 2);
";

fn private(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_store_written_by_today_s_code_opens_after_migration_with_its_pages_and_chunks_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("crawl");
    private(&dir);
    let file = dir.join("kb.duckdb");
    {
        let old = Connection::open(&file).unwrap();
        old.execute_batch(OLD_TABLES).unwrap();
    }
    drop(Store::open(&file).unwrap());
    let conn = Connection::open(&file).unwrap();
    assert_eq!(count(&conn, "pages"), 1);
    assert_eq!(count(&conn, "chunks"), 1);
    assert_eq!(count(&conn, "terms"), 1);
    let title: String = conn
        .query_row("SELECT title FROM pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "A");
    // The claim and evidence tables are there now, empty, with the versions recorded.
    for t in ["claims", "evidence", "evidence_at", "claim_evidence"] {
        assert_eq!(count(&conn, t), 0, "{t}");
    }
    let versions: i64 = conn
        .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(versions, 2);
}

#[test]
fn a_folder_open_to_others_is_refused_with_the_command_that_fixes_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("crawl");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let file = dir.join("kb.duckdb");
    let err = Store::open(&file).unwrap_err();
    assert!(err.contains("chmod 700"), "{err}");
    assert!(!file.exists(), "a refused folder gets no file");
    // And a reader is refused the same way.
    std::fs::write(&file, b"").unwrap();
    assert!(Store::open_read(&file).unwrap_err().contains("chmod 700"));
}

#[test]
fn a_second_writer_is_refused_while_the_first_holds_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("crawl");
    private(&dir);
    let file = dir.join("kb.duckdb");
    let first = Store::open(&file).unwrap();
    let err = Store::open(&file).unwrap_err();
    assert!(err.contains("one process writes"), "{err}");
    drop(first);
    drop(Store::open(&file).unwrap());
}

// Crawl pages as evidence, extracted facts as claims.

fn html(title: &str, body: &str) -> ResponseTemplate {
    let page = format!(
        "<html lang=\"en\"><head><title>{title}</title>\
<meta property=\"og:site_name\" content=\"Acme Robotics\"></head>\
<body><main>{body}</main></body></html>"
    );
    ResponseTemplate::new(200).set_body_raw(page.into_bytes(), "text/html; charset=utf-8")
}

async fn site() -> MockServer {
    let server = MockServer::start().await;
    for (p, title, body) in [
        (
            "/",
            "Acme",
            "<h1>Acme Robotics</h1><p>We build warehouse picking robots for logistics teams.</p>",
        ),
        (
            "/contact",
            "Contact",
            "<h1>Contact</h1><p>Write to sales@acme.test or call +49 30 1234 5678 for a demo.</p>",
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(html(title, body))
            .mount(&server)
            .await;
    }
    server
}

fn opts(server: &MockServer) -> iohr::kb::Opts {
    iohr::kb::Opts {
        deadline: Duration::from_secs(30),
        rate: 50,
        scope: server.address().ip().to_string(),
        also: Vec::new(),
        redirects: Vec::new(),
        crawled: "2026-11-08".into(),
    }
}

async fn crawl(server: &MockServer, root: &Path) {
    let input: Vec<iohr::kb::PageIn> = ["/", "/contact"]
        .iter()
        .map(|p| iohr::kb::PageIn {
            url: format!("{}{p}", server.uri()),
            title: String::new(),
            depth: 0,
            rendered: false,
        })
        .collect();
    let none = |_: usize, _: usize| {};
    let built = iohr::kb::build(root, "acme.test", &input, &opts(server), &none)
        .await
        .unwrap();
    assert!(built.report.index_error.is_none(), "{built:?}");
}

fn project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(root.join(".inorbit")).unwrap();
    std::fs::create_dir_all(root.join("atlas")).unwrap();
    (tmp, root)
}

fn rows(root: &Path) -> [i64; 4] {
    let conn = Connection::open_with_flags(
        iohr::kb::index_path(root),
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap();
    ["evidence", "evidence_at", "claims", "claim_evidence"].map(|t| count(&conn, t))
}

#[tokio::test]
async fn reading_the_same_pages_again_adds_no_evidence_and_no_claims() {
    let server = site().await;
    let (_tmp, root) = project();
    crawl(&server, &root).await;
    let first = rows(&root);
    assert!(first[0] >= 2, "one evidence record per page: {first:?}");
    assert!(first[2] >= 2, "the contact page has facts: {first:?}");
    // The links between them are one to one.
    assert_eq!(first[2], first[3], "{first:?}");
    crawl(&server, &root).await;
    assert_eq!(
        rows(&root),
        first,
        "a re-run on unchanged input adds nothing"
    );
}

#[tokio::test]
async fn every_claims_proof_tree_resolves_to_evidence_whose_digest_is_the_page_on_disk() {
    let server = site().await;
    let (_tmp, root) = project();
    crawl(&server, &root).await;
    let store = Store::open_read(&iohr::kb::index_path(&root)).unwrap();
    let claims = store.claims(Some("acme.test"), 1000).unwrap();
    assert!(claims.len() >= 2, "{claims:?}");
    for row in &claims {
        let proof = store.proof(&row.id).unwrap();
        assert!(proof.tree.is_complete(), "{row:?}");
        assert!(proof.integrity, "{row:?}");
        assert!(!proof.stale, "{row:?}");
        // Verified means the page bytes hold the value, and the row says so, not "true".
        assert_eq!(row.support, "verified", "{row:?}");
        assert!(!proof.evidence.is_empty(), "{row:?}");
        for e in &proof.evidence {
            let (url, cache) = e.at.first().expect("evidence says where it was read");
            assert!(url.starts_with("http"), "{url}");
            let file = iohr::kb::crawl_dir(&root).join(cache);
            let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            assert_eq!(
                e.digest,
                ContentDigest::of_bytes(&bytes).to_hex(),
                "{row:?}"
            );
            assert_eq!(e.bytes, bytes.len() as u64);
        }
        // The value is in the page the proof ends at.
        let (_, cache) = &proof.evidence[0].at[0];
        let page = std::fs::read_to_string(iohr::kb::crawl_dir(&root).join(cache)).unwrap();
        assert!(page.contains(&row.value), "{row:?}");
    }
    assert!(claims.iter().any(|c| c.value.contains("sales@acme.test")));
}

#[tokio::test]
async fn a_page_that_changed_makes_its_old_claims_stale_and_new_ones_current() {
    let server = site().await;
    let (_tmp, root) = project();
    crawl(&server, &root).await;
    let before = Store::open_read(&iohr::kb::index_path(&root))
        .unwrap()
        .claims(Some("acme.test"), 1000)
        .unwrap();
    let old = before
        .iter()
        .find(|c| c.value.contains("sales@acme.test"))
        .unwrap()
        .id
        .clone();
    let cache = {
        let s = Store::open_read(&iohr::kb::index_path(&root)).unwrap();
        s.proof(&old).unwrap().evidence[0].at[0].1.clone()
    };
    let file = iohr::kb::crawl_dir(&root).join(cache);
    let changed = std::fs::read_to_string(&file)
        .unwrap()
        .replace("sales@acme.test", "hello@acme.test");
    std::fs::write(&file, changed).unwrap();
    crawl(&server, &root).await;
    let store = Store::open_read(&iohr::kb::index_path(&root)).unwrap();
    let now = store.claims(Some("acme.test"), 1000).unwrap();
    let was = now.iter().find(|c| c.id == old).unwrap();
    assert!(was.stale, "the old claim is kept and marked stale: {was:?}");
    assert!(store.proof(&old).unwrap().stale);
    let fresh = now
        .iter()
        .find(|c| c.value.contains("hello@acme.test"))
        .unwrap();
    assert!(!fresh.stale, "{fresh:?}");
    assert_ne!(fresh.id, old);
}

#[tokio::test]
async fn the_command_lists_claims_and_proves_one_as_json() {
    let server = site().await;
    let (tmp, root) = project();
    crawl(&server, &root).await;
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let run = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_iohr"))
            .env_clear()
            .current_dir(&root)
            .args(args)
            .envs(support::kept_env())
            .env("HOME", &home)
            .env("NO_COLOR", "1")
            .env("IOHR_CONFIG_DIR", home.join("config"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap()
    };
    let list = run(&["atlas", "claims", "--json"]);
    let claims = list["claims"].as_array().unwrap();
    let email = claims
        .iter()
        .find(|c| c["value"].as_str().unwrap().contains("sales@acme.test"))
        .unwrap();
    assert_eq!(email["support"], "verified");
    let id = email["id"].as_str().unwrap();
    let proof = run(&["atlas", "claims", "--prove", id, "--json"]);
    assert_eq!(proof["complete"], true, "{proof}");
    assert_eq!(proof["integrity"], true, "{proof}");
    let e = &proof["evidence"][0];
    assert!(
        e["digest"].as_str().unwrap().starts_with("sha256:"),
        "{proof}"
    );
    assert!(
        e["read_at"][0]["url"]
            .as_str()
            .unwrap()
            .ends_with("/contact"),
        "{proof}"
    );
}
