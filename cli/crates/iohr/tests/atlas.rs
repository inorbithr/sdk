//! `iohr project init`'s atlas, `iohr atlas new` and `iohr sync` (RFC 0205): the binary with a
//! private HOME, and a mock Decisions API for the sync. Nothing of the machine is read.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use support::kept_env;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
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
        .envs(kept_env())
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
            "--template",
            "blank",
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

// ---------------------------------------------------------------- roles and memory

const ROLES: &str = "/v1/coordinator/personalities";
const IMPORT: &str = "/v1/coordinator/personalities/import";

async fn mount_roles(server: &MockServer, listed: serde_json::Value, import_status: u16) {
    Mock::given(method("GET"))
        .and(path(ROLES))
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
fn init_writes_the_roles_the_record_the_memory_readme_and_the_agent_section() {
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
        let p = read(&dir, &format!("atlas/roles/{name}.md"));
        assert!(p.contains(&format!("name: {name}")), "{p}");
        assert!(p.contains("parent:"), "{p}");
        assert!(p.contains("https://"), "every line carries its source: {p}");
    }
    assert!(!dir.join("atlas/roles/python-engineer.md").exists());
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
    assert_eq!(v["roles"]["signed_in"], false, "{v}");
    assert!(v["roles"]["outcomes"].as_array().unwrap().len() >= 2);
    let o = iohr(
        h.path(),
        &dir,
        None,
        &["atlas", "roles", "import", "--dry-run"],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("\"doctrine\""), "{t}");
    assert!(t.contains("\"parent\""), "{t}");
}

#[test]
fn the_built_in_facts_carry_the_day_they_were_checked() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"made\"\n").unwrap();
    let o = iohr(h.path(), &dir, None, &["atlas", "init"]);
    assert!(o.status.success(), "{}", text(&o));
    let pack = read(&dir, "atlas/roles/security.md");
    assert!(pack.contains("\nchecked: 2026-11-06\n"), "{pack}");
    let mut seeded = 0;
    for e in std::fs::read_dir(dir.join(".inorbit/memory"))
        .unwrap()
        .flatten()
    {
        if e.file_name() == "README.md" {
            continue;
        }
        let t = std::fs::read_to_string(e.path()).unwrap();
        assert!(t.contains("\nchecked: 2026-11-06\n"), "{t}");
        seeded += 1;
    }
    assert!(seeded > 0);
}

#[test]
fn an_older_personalities_folder_moves_to_roles_once_and_never_overwrites() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    std::fs::create_dir_all(dir.join("atlas/personalities")).unwrap();
    std::fs::create_dir_all(dir.join("atlas/roles")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"made\"\n").unwrap();
    std::fs::write(
        dir.join("atlas/personalities/security.md"),
        "old security\n",
    )
    .unwrap();
    std::fs::write(dir.join("atlas/personalities/mine.md"), "my own\n").unwrap();
    std::fs::write(dir.join("atlas/roles/mine.md"), "already in roles\n").unwrap();
    let o = iohr(h.path(), &dir, None, &["atlas", "init"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert_eq!(
        t.matches("moved atlas/personalities/ to atlas/roles/")
            .count(),
        1,
        "{t}"
    );
    // Moved, not overwritten, and the one that clashed stays where it was.
    assert_eq!(read(&dir, "atlas/roles/security.md"), "old security\n");
    assert_eq!(read(&dir, "atlas/roles/mine.md"), "already in roles\n");
    assert_eq!(read(&dir, "atlas/personalities/mine.md"), "my own\n");
    assert!(!dir.join("atlas/personalities/security.md").exists());
    // The second run has nothing to move and says nothing about it.
    let o = iohr(h.path(), &dir, None, &["atlas", "init"]);
    assert!(
        !text(&o).contains("moved atlas/personalities/"),
        "{}",
        text(&o)
    );
    // The old subcommand name still works, hidden, and so does the new one.
    for word in ["personalities", "roles"] {
        let o = iohr(h.path(), &dir, None, &["atlas", word, "list", "--json"]);
        assert!(o.status.success(), "{word}: {}", text(&o));
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert!(v["roles"].is_array(), "{v}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_in_init_imports_under_the_base_then_leaves_alone_and_skips() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"made\"\n").unwrap();
    let server = MockServer::start().await;
    // An empty account: no root, so the base engineer comes with them.
    mount_roles(&server, serde_json::json!({ "personalities": [] }), 200).await;
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
    let p = read(&dir, "atlas/roles/security.md");
    assert!(p.contains("imported_version: 1"), "{p}");

    // The account now has them at the version we wrote: unchanged, no second POST.
    let server2 = MockServer::start().await;
    mount_roles(
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
        &["atlas", "roles", "import"],
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
    mount_roles(
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
        &["atlas", "roles", "import"],
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
    // Signed in: the roles go to the account with the account step, so init has an account.
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "subject": "ak_tok1", "kind": "client", "scopes": []
        })))
        .mount(&server)
        .await;
    mount_roles(&server, serde_json::json!({ "personalities": [] }), 403).await;
    let o = iohr(
        h.path(),
        h.path(),
        Some(&server.uri()),
        &[
            "project",
            "init",
            "--template",
            "blank",
            "made",
            "--dir",
            dir.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert_eq!(t.matches("stay local").count(), 1, "{t}");
    assert!(dir.join("atlas/roles/security.md").is_file());
}

/// InOrbit lints an import as a whole: one refused role must not keep the others local.
#[tokio::test(flavor = "multi_thread")]
async fn one_refused_role_keeps_only_itself_local_and_the_others_are_imported() {
    let h = tempfile::tempdir().unwrap();
    let dir = fresh_rust_project(h.path());
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ROLES))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "personalities": [] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(IMPORT))
        .and(body_string_contains(r#""name":"security""#))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {"code": "failed_precondition", "message": "a line is refused by the privacy filter"}
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(IMPORT))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "written": [{"name": "base-engineer", "version": 1}, {"name": "rust-engineer", "version": 1}]
        })))
        .with_priority(2)
        .mount(&server)
        .await;
    let o = iohr(h.path(), &dir, Some(&server.uri()), &["atlas", "init"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("InOrbit refused security"), "{t}");
    assert!(!t.contains("stay local ("), "{t}");
    let sent = imports(&server.received_requests().await.unwrap());
    let alone = sent
        .iter()
        .filter(|b| {
            b["personalities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "rust-engineer")
        })
        .count();
    assert_eq!(alone, 2, "the batch, then rust-engineer alone: {sent:?}");
    let rust = read(&dir, "atlas/roles/rust-engineer.md");
    assert!(rust.contains("imported_version"), "{rust}");
    let security = read(&dir, "atlas/roles/security.md");
    assert!(!security.contains("imported_version"), "{security}");
}

/// An account that already holds `security` (and its root) at version 1, from an earlier
/// project: its head's lines and the cause of each version it has, for the upgrade tests.
async fn account_with_security(
    lines: serde_json::Value,
    causes: &[&str],
    posted_version: i64,
) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ROLES))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "personalities": [
                {"name": "base-engineer", "parent": "", "head_version": 1},
                {"name": "security", "parent": "base-engineer", "head_version": causes.len()},
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{ROLES}/security")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "security", "head_version": causes.len(), "lines": lines
        })))
        .mount(&server)
        .await;
    let versions: Vec<_> = causes
        .iter()
        .enumerate()
        .map(|(i, c)| serde_json::json!({"version": i + 1, "cause": c}))
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("{ROLES}/security/versions")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "versions": versions })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(IMPORT))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "written": [{"name": "security", "version": posted_version}]
        })))
        .mount(&server)
        .await;
    server
}

fn fresh_rust_project(h: &Path) -> PathBuf {
    let dir = h.join("work/made");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"made\"\n").unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unmodified_older_built_in_in_the_account_is_upgraded_and_says_v1_to_v2() {
    let h = tempfile::tempdir().unwrap();
    let dir = fresh_rust_project(h.path());
    // Every version in its history is an import, and its lines are an older catalogue's.
    let server = account_with_security(
        serde_json::json!([{"place": "base", "section": "Old", "text": "an older built-in line", "origin": "doctrine"}]),
        &["import"],
        2,
    )
    .await;
    let o = iohr(h.path(), &dir, Some(&server.uri()), &["atlas", "init"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("security: upgraded v1 \u{2192} v2"), "{t}");
    assert!(!t.contains("left alone v1"), "{t}");
    let sent = imports(&server.received_requests().await.unwrap());
    assert_eq!(sent.len(), 1, "one import, with the newer security in it");
    assert!(
        sent[0]["personalities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "security")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_the_person_edited_is_left_alone_with_the_command_that_takes_the_newer_one() {
    let h = tempfile::tempdir().unwrap();
    let dir = fresh_rust_project(h.path());
    // A version that is not an import: the person (or a learned line) changed it.
    let server = account_with_security(
        serde_json::json!([{"place": "base", "section": "Mine", "text": "my own rule", "origin": "doctrine"}]),
        &["import", "applied"],
        3,
    )
    .await;
    let o = iohr(h.path(), &dir, Some(&server.uri()), &["atlas", "init"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("security: left alone"), "{t}");
    assert!(t.contains("catalogue v"), "{t}");
    assert!(t.contains("iohr atlas roles import --upgrade"), "{t}");
    // The language's engineer is new to this account and goes; the edited security does not.
    for sent in imports(&server.received_requests().await.unwrap()) {
        assert!(
            sent["personalities"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["name"] != "security"),
            "nothing is sent over an edited copy"
        );
    }

    // `--upgrade` is the person's go: the catalogue's version replaces it.
    let o = iohr(
        h.path(),
        &dir,
        Some(&server.uri()),
        &["atlas", "roles", "import", "--upgrade"],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("security: upgraded v2 \u{2192} v3"), "{t}");
    assert!(
        imports(&server.received_requests().await.unwrap())
            .iter()
            .any(|s| s["personalities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "security")),
        "the catalogue's security was sent on --upgrade"
    );
}
