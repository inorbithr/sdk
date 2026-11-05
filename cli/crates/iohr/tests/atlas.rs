//! `iohr project init`'s atlas, `iohr atlas new` and `iohr sync` (RFC 0205): the binary with a
//! private HOME, and a mock Decisions API for the sync. Nothing of the machine is read.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SPACE: &str = "lab_1";
const DOCS: &str = "/v1/decisions/spaces/lab_1/documents";

fn token() -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": ["decisions:read", "decisions:write"], "org": "acc_test1",
            "plan": "free", "key": "key_1"
        }))
    )
}

fn iohr(home: &Path, cwd: &Path, base: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_iohr"));
    cmd.env_clear()
        .current_dir(cwd)
        .args(args)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("NO_COLOR", "1")
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .stdin(Stdio::null());
    if let Some(b) = base {
        cmd.env("IOHR_BASE_URL", b).env("IOHR_TOKEN", token());
    }
    cmd.output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A project made by `iohr project init`, in a private HOME.
fn project() -> (tempfile::TempDir, PathBuf) {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    let o = iohr(
        h.path(),
        h.path(),
        None,
        &[
            "project",
            "init",
            "made",
            "--dir",
            dir.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    (h, dir)
}

fn doc(id: &str, kind: &str, number: i32, version: i32) -> serde_json::Value {
    serde_json::json!({
        "document_id": id, "space_id": SPACE, "kind": kind, "number": number,
        "slug": "x", "title": "T", "status": "draft",
        "public": false, "summary": "", "current_version": version, "superseded_by": "",
        "path": "", "created_by": "ops:nevio",
        "created_at": "2026-10-06T00:00:00Z", "updated_at": "2026-10-07T00:00:00Z",
        "parent_id": "", "child_index": 0, "display_number": format!("{number:04}"),
        "children": 0, "headline": "", "headline_note": "", "supersedes_id": "",
        "measures_id": "", "access": "team", "stage": "", "source": "", "outward": false,
        "decider": "", "approved_by": "", "review_by": "", "review_overdue": false,
        "linked": false
    })
}

fn kinds() -> [(&'static str, &'static str); 4] {
    [
        ("prd", "ldoc_prd"),
        ("rfc", "ldoc_rfc"),
        ("adr", "ldoc_adr"),
        ("study", "ldoc_study"),
    ]
}

/// Create answers version 1, save answers version 2, a read answers `read_version`.
async fn mount(server: &MockServer, read_version: i32, save_status: u16) {
    for (kind, id) in kinds() {
        Mock::given(method("POST"))
            .and(path(DOCS))
            .and(body_partial_json(serde_json::json!({"kind": kind})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": doc(id, kind, 7, 1), "findings": [], "text": "template"
            })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("{DOCS}/{id}/versions")))
            .respond_with(if save_status == 200 {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "document": doc(id, kind, 7, 2), "findings": [], "version": 2
                }))
            } else {
                ResponseTemplate::new(save_status).set_body_json(serde_json::json!({
                    "code": "conflict", "message": "someone saved"
                }))
            })
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{DOCS}/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document": doc(id, kind, 7, read_version), "text": "remote",
                "version": read_version, "message": "m", "author": "ops:nevio",
                "saved_at": "2026-10-07T00:00:00Z", "findings": [], "children": [],
                "mentions": [], "mentioned_in": [], "successors": [], "measured_by": [],
                "context": [], "context_for": [], "diagrams": [], "implements": [],
                "implemented_by": []
            })))
            .mount(server)
            .await;
    }
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

#[test]
fn init_makes_the_atlas_and_a_second_run_keeps_every_edit() {
    let (h, dir) = project();
    for rel in [
        "atlas/README.md",
        "atlas/prd/0001-what-made-is-for.md",
        "atlas/rfc/0001-how-made-is-built.md",
        "atlas/adr/0001-keep-project-documents-in-atlas.md",
        "atlas/studies/0001-the-first-measurement-for-made.md",
        "atlas/canvas/start.canvas.json",
        "atlas/models/README.md",
    ] {
        assert!(dir.join(rel).is_file(), "{rel} is missing");
    }
    assert!(read(&dir, "atlas/prd/0001-what-made-is-for.md").contains("Stub"));
    assert!(read(&dir, "atlas/canvas/start.canvas.json").contains("\"iohr.canvas\""));

    // The person edits one; `atlas init` fills only what is missing.
    let prd = dir.join("atlas/prd/0001-what-made-is-for.md");
    std::fs::write(&prd, "mine").unwrap();
    std::fs::remove_file(dir.join("atlas/models/README.md")).unwrap();
    let o = iohr(h.path(), &dir, None, &["atlas", "init"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(std::fs::read_to_string(&prd).unwrap(), "mine");
    assert!(dir.join("atlas/models/README.md").is_file());
    assert_eq!(
        std::fs::read_dir(dir.join("atlas/prd")).unwrap().count(),
        1,
        "no second starter"
    );
}

#[test]
fn new_takes_the_next_free_number_and_sync_dry_run_sends_nothing() {
    let (h, dir) = project();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &["atlas", "new", "rfc", "Second thing"],
    );
    assert!(o.status.success(), "{}", text(&o));
    let made = dir.join("atlas/rfc/0002-second-thing.md");
    assert!(made.is_file(), "{}", text(&o));
    assert!(read(&dir, "atlas/rfc/0002-second-thing.md").contains("number: 2"));

    // No session and no server: a dry run with nothing recorded needs neither.
    let o = iohr(
        h.path(),
        &dir,
        None,
        &["sync", "--dry-run", "--space", SPACE],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("would create"), "{t}");
    assert!(
        t.contains("atlas/canvas/start.canvas.json"),
        "canvas stays local: {t}"
    );
    assert!(!read(&dir, "atlas/rfc/0002-second-thing.md").contains("remote_id"));

    // No space linked and none given: a usage error that says what to do.
    let o = iohr(h.path(), &dir, None, &["sync", "--dry-run"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("--space"), "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_creates_saves_and_writes_the_ids_back_then_skips() {
    let (h, dir) = project();
    let server = MockServer::start().await;
    mount(&server, 2, 200).await;
    let base = server.uri();

    let o = iohr(h.path(), &dir, Some(&base), &["sync", "--space", SPACE]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert_eq!(t.matches("created").count(), 4, "{t}");
    assert!(!t.contains("TOKENSIGNATURE"), "{t}");

    let prd = read(&dir, "atlas/prd/0001-what-made-is-for.md");
    assert!(prd.contains("remote_id: ldoc_prd"), "{prd}");
    assert!(prd.contains("remote_version: 2"), "{prd}");
    assert!(prd.contains("remote_number: 7"), "{prd}");
    assert!(prd.contains("remote_sha: "), "{prd}");

    // What was sent carries the text without the local keys, on the created version.
    let sent = server.received_requests().await.unwrap();
    let save = sent
        .iter()
        .find(|r| r.url.path() == format!("{DOCS}/ldoc_prd/versions"))
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&save.body).unwrap();
    assert_eq!(body["base_version"], 1);
    let sent_text = body["text"].as_str().unwrap();
    assert!(sent_text.contains("title:"));
    assert!(!sent_text.contains("remote_id"), "{sent_text}");

    // A second run: the remote is at the recorded version and nothing changed.
    let before = sent.len();
    let o = iohr(h.path(), &dir, Some(&base), &["sync", "--space", SPACE]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert_eq!(t.matches("unchanged").count(), 4, "{t}");
    let after = server.received_requests().await.unwrap();
    assert!(
        after[before..].iter().all(|r| r.method.as_str() == "GET"),
        "a second run only reads"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_is_saved_and_a_newer_remote_is_never_overwritten() {
    let (h, dir) = project();
    let server = MockServer::start().await;
    mount(&server, 2, 200).await;
    let base = server.uri();
    let o = iohr(h.path(), &dir, Some(&base), &["sync", "--space", SPACE]);
    assert!(o.status.success(), "{}", text(&o));

    // An edit is saved on the version the read returned.
    let rel = "atlas/prd/0001-what-made-is-for.md";
    let edited = format!("{}\nMore words.\n", read(&dir, rel));
    std::fs::write(dir.join(rel), &edited).unwrap();
    let o = iohr(h.path(), &dir, Some(&base), &["sync", "--space", SPACE]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("saved"), "{t}");

    // Someone saved in Decisions too (the remote is at 3), and the file changed here again:
    // a conflict, nothing sent.
    server.reset().await;
    mount(&server, 3, 200).await;
    std::fs::write(dir.join(rel), format!("{edited}\nAnd more.\n")).unwrap();
    let o = iohr(h.path(), &dir, Some(&base), &["sync", "--space", SPACE]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("conflict"), "{t}");
    let got = server.received_requests().await.unwrap();
    assert!(
        got.iter().all(|r| r.method.as_str() == "GET"),
        "a conflict sends nothing"
    );
    assert!(
        read(&dir, rel).contains("And more."),
        "the file is untouched"
    );

    // A 409 on the save itself (the remote moved after the read): also a conflict.
    server.reset().await;
    mount(&server, 2, 409).await;
    let o = iohr(h.path(), &dir, Some(&base), &["sync", "--space", SPACE]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("conflict"), "{t}");
    assert!(read(&dir, rel).contains("And more."));
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_never_shows_the_token_or_a_documents_words() {
    let (h, dir) = project();
    let server = MockServer::start().await;
    mount(&server, 2, 200).await;
    let o = iohr(
        h.path(),
        &dir,
        Some(&server.uri()),
        &["sync", "--space", SPACE, "--verbose"],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(!t.contains("TOKENSIGNATURE"), "{t}");
    assert!(t.contains("request id"), "{t}");
    assert!(!t.contains("Stub:"), "no document text in the output: {t}");
}

// ---------------------------------------------------------------- personalities and memory

const PERSONALITIES: &str = "/v1/coordinator/personalities";
const IMPORT: &str = "/v1/coordinator/personalities/import";

async fn mount_personalities(server: &MockServer, listed: serde_json::Value, import_status: u16) {
    Mock::given(method("GET"))
        .and(path(PERSONALITIES))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed))
        .mount(server)
        .await;
    let names = ["base-engineer", "security", "rust-engineer"];
    let written: Vec<_> = names
        .iter()
        .map(|n| serde_json::json!({"name": n, "version": 1}))
        .collect();
    Mock::given(method("POST"))
        .and(path(IMPORT))
        .respond_with(
            ResponseTemplate::new(import_status)
                .set_body_json(serde_json::json!({ "written": written })),
        )
        .mount(server)
        .await;
}

fn imports(sent: &[wiremock::Request]) -> Vec<serde_json::Value> {
    sent.iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == IMPORT)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[test]
#[allow(clippy::many_single_char_names, reason = "short names in a long test")]
fn init_writes_the_personalities_the_record_the_memory_readme_and_the_agent_section() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"made\"\n").unwrap();
    std::fs::write(dir.join("inorbit.toml"), "name = \"made\"\n").unwrap();
    std::fs::write(dir.join("AGENTS.md"), "# Agents\n\nMine.\n").unwrap();
    let o = iohr(h.path(), &dir, None, &["atlas", "init"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    // Signed out: said once, nothing fails.
    assert_eq!(t.matches("Not signed in").count(), 1, "{t}");

    // The language came from the build file: a Rust project gets the Rust engineer.
    for name in ["security", "rust-engineer"] {
        let p = read(&dir, &format!("atlas/personalities/{name}.md"));
        assert!(p.contains(&format!("name: {name}")), "{p}");
        assert!(p.contains("parent:"), "{p}");
        assert!(p.contains("https://"), "every line carries its source: {p}");
    }
    assert!(!dir.join("atlas/personalities/python-engineer.md").exists());
    assert!(read(&dir, "inorbit.toml").contains("rust-engineer"));
    // Memory is local: a README and the seed memories under `.inorbit/memory/`, no network.
    assert!(read(&dir, ".inorbit/memory/README.md").contains("iohr memory recall"));
    assert!(!dir.join("atlas/memory").exists());

    // The agent file keeps the person's text and gains the memory section once.
    let a = read(&dir, "AGENTS.md");
    assert!(a.contains("Mine."), "{a}");
    assert_eq!(a.matches("iohr memory recall").count(), 1, "{a}");
    let o = iohr(h.path(), &dir, None, &["atlas", "init"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(
        read(&dir, "AGENTS.md")
            .matches("iohr memory recall")
            .count(),
        1
    );

    // JSON reports them; the dry run prints the body and sends nothing.
    let o = iohr(h.path(), &dir, None, &["atlas", "init", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["personalities"]["signed_in"], false, "{v}");
    assert!(v["personalities"]["outcomes"].as_array().unwrap().len() >= 2);
    let o = iohr(
        h.path(),
        &dir,
        None,
        &["atlas", "personalities", "import", "--dry-run"],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("\"doctrine\""), "{t}");
    assert!(t.contains("\"parent\""), "{t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_in_init_imports_under_the_base_then_leaves_alone_and_skips() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"made\"\n").unwrap();
    let server = MockServer::start().await;
    // An empty account: no root, so the base engineer comes with them.
    mount_personalities(&server, serde_json::json!({ "personalities": [] }), 200).await;
    let base = server.uri();

    let o = iohr(h.path(), &dir, Some(&base), &["atlas", "init"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("security: written v1"), "{t}");
    assert!(t.contains("rust-engineer: written v1"), "{t}");
    assert!(!t.contains("TOKENSIGNATURE"), "{t}");
    let sent = imports(&server.received_requests().await.unwrap());
    assert_eq!(sent.len(), 1, "one import for all");
    let names: Vec<&str> = sent[0]["personalities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"base-engineer"), "{names:?}");
    let sec = sent[0]["personalities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "security")
        .unwrap();
    assert_eq!(sec["parent"], "base-engineer");
    let line = &sec["sections"][0]["lines"][0];
    assert_eq!(line["origin"], "doctrine");
    assert!(line["cites"][0].as_str().unwrap().starts_with("https://"));

    // The import is recorded in the pack, so the next run only compares.
    let p = read(&dir, "atlas/personalities/security.md");
    assert!(p.contains("imported_version: 1"), "{p}");

    // The account now has them at the version we wrote: unchanged, no second POST.
    let server2 = MockServer::start().await;
    mount_personalities(
        &server2,
        serde_json::json!({ "personalities": [
            {"name": "base-engineer", "parent": "", "head_version": 1},
            {"name": "security", "parent": "base-engineer", "head_version": 1},
            {"name": "rust-engineer", "parent": "security", "head_version": 1},
        ]}),
        200,
    )
    .await;
    let o = iohr(
        h.path(),
        &dir,
        Some(&server2.uri()),
        &["atlas", "personalities", "import"],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("security: unchanged"), "{t}");
    assert_eq!(
        imports(&server2.received_requests().await.unwrap()).len(),
        0
    );

    // Edited in InOrbit since our import: left alone, never overwritten.
    let server3 = MockServer::start().await;
    mount_personalities(
        &server3,
        serde_json::json!({ "personalities": [
            {"name": "base-engineer", "parent": "", "head_version": 1},
            {"name": "security", "parent": "base-engineer", "head_version": 4},
            {"name": "rust-engineer", "parent": "security", "head_version": 1},
        ]}),
        200,
    )
    .await;
    let o = iohr(
        h.path(),
        &dir,
        Some(&server3.uri()),
        &["atlas", "personalities", "import"],
    );
    let t = text(&o);
    assert!(t.contains("security: left alone"), "{t}");
    assert_eq!(
        imports(&server3.received_requests().await.unwrap()).len(),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_import_never_fails_init_and_says_it_once() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    let server = MockServer::start().await;
    mount_personalities(&server, serde_json::json!({ "personalities": [] }), 403).await;
    let o = iohr(
        h.path(),
        h.path(),
        Some(&server.uri()),
        &[
            "project",
            "init",
            "made",
            "--dir",
            dir.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert_eq!(t.matches("stay local").count(), 1, "{t}");
    assert!(dir.join("atlas/personalities/security.md").is_file());
}
