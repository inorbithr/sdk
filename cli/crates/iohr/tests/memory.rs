//! `iohr memory` (RFC 0205, ADR 0216): memory is local first, kept as one Markdown file per
//! memory. The binary runs with a private HOME against a temp repository; the service is a mock
//! and is only reached by `sync`, `drift --remote` and `--scope account`.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![allow(
    clippy::many_single_char_names,
    reason = "short names for the home, the repo and the output in long scenarios"
)]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use support::kept_env;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn token() -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": ["memory:read", "memory:write"], "org": "acc_test1",
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

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(o)))
}

fn git(dir: &Path, args: &[&str], date: Option<&str>) {
    let mut c = Command::new("git");
    c.current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    if let Some(d) = date {
        c.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
    }
    let o = c.output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", text(&o));
}

/// A private HOME and a git repository with `src/a.rs` in it.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/shop");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
    git(&dir, &["init", "-q", "."], None);
    git(&dir, &["add", "src/a.rs"], None);
    git(
        &dir,
        &["commit", "-q", "-m", "a"],
        Some("2020-01-01T00:00:00Z"),
    );
    (h, dir)
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            e.get("entry").unwrap_or(e)["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn memory_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir.join(".inorbit/memory"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

// ---------------------------------------------------------------- offline

#[test]
fn remember_recall_list_and_forget_work_offline_in_the_project() {
    let (h, dir) = repo();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "We use sqlx for queries",
            "--why",
            "typed SQL",
            "--ref",
            "src/a.rs",
            "--json",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let v = json(&o);
    let id = v["entry"]["id"].as_str().unwrap().to_owned();
    assert_eq!(v["scope"], "project");
    assert_eq!(v["file"], format!(".inorbit/memory/{id}.md"));
    // One Markdown file with front matter, the text as the body; no database anywhere.
    let file =
        std::fs::read_to_string(dir.join(".inorbit/memory").join(format!("{id}.md"))).unwrap();
    assert!(file.starts_with("---\n"), "{file}");
    assert!(file.contains("why: typed SQL"), "{file}");
    assert!(file.contains("refs: [src/a.rs]"), "{file}");
    assert!(
        file.trim_end().ends_with("We use sqlx for queries"),
        "{file}"
    );

    // A preference too, then recall and list read the files.
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "Keep answers short",
            "--preference",
            "--why",
            "owner",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let o = iohr(
        h.path(),
        &dir,
        None,
        &["memory", "recall", "sqlx", "--json"],
    );
    assert_eq!(ids(&json(&o)), std::slice::from_ref(&id), "{}", text(&o));
    let o = iohr(h.path(), &dir, None, &["memory", "list", "--json"]);
    assert_eq!(ids(&json(&o)).len(), 2);
    let o = iohr(h.path(), &dir, None, &["memory", "list"]);
    let t = text(&o);
    assert!(
        o.status.success() && t.contains(&id) && t.contains("preference"),
        "{t}"
    );

    // Forget removes the file; forgetting it again is a usage error (exit 2).
    let o = iohr(h.path(), &dir, None, &["memory", "forget", &id]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(memory_files(&dir).len(), 1);
    let o = iohr(h.path(), &dir, None, &["memory", "forget", &id]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    let o = iohr(
        h.path(),
        &dir,
        None,
        &["memory", "recall", "sqlx", "--json"],
    );
    assert_eq!(ids(&json(&o)), Vec::<String>::new());
}

#[test]
fn a_secret_is_refused_and_the_user_scope_lives_in_the_home() {
    let (h, dir) = repo();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "token is ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        ],
    );
    assert!(!o.status.success(), "{}", text(&o));
    assert_eq!(memory_files(&dir), Vec::<PathBuf>::new());

    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "I like tabs",
            "--scope",
            "user",
            "--why",
            "habit",
            "--json",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(json(&o)["scope"], "user");
    assert!(
        memory_files(&dir).is_empty(),
        "user memory is not in the repo"
    );
    assert_eq!(
        std::fs::read_dir(h.path().join(".inorbit/memory/notes"))
            .unwrap()
            .count(),
        1
    );
    // Outside any project the default falls back to the user scope, said on stderr.
    let o = iohr(h.path(), h.path(), None, &["memory", "list", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    let diag: Vec<String> = h.path().ancestors().map(|d| format!("{} git={} inorbit={} atlas={}", d.display(), d.join(".git").exists(), d.join(".inorbit").is_dir(), d.join("atlas").is_dir())).collect();
    assert_eq!(json(&o)["scope"], "user", "DIAG home={:?} canon={:?} {:#?}", h.path(), h.path().canonicalize(), diag);
    assert!(String::from_utf8_lossy(&o.stderr).contains("not inside a project"));
    // Asked for by name, a project is required.
    let o = iohr(
        h.path(),
        h.path(),
        None,
        &["memory", "list", "--scope", "project"],
    );
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
}

#[test]
fn the_account_scope_needs_sync_on_and_never_reaches_the_network_without_it() {
    let (h, dir) = repo();
    // No server is configured at all: the refusal comes before any call.
    let o = iohr(
        h.path(),
        &dir,
        None,
        &["memory", "list", "--scope", "account"],
    );
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("[memory] sync = true"), "{}", text(&o));
}

// ---------------------------------------------------------------- check

#[test]
fn check_finds_a_dangling_ref_and_a_missing_why_and_strict_fails() {
    let (h, dir) = repo();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "The queue lives in src/gone.rs",
            "--ref",
            "src/gone.rs",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "Escapes the repo",
            "--why",
            "x",
            "--ref",
            "../outside",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));

    let o = iohr(h.path(), &dir, None, &["memory", "check", "--json"]);
    assert!(
        o.status.success(),
        "without --strict findings do not fail: {}",
        text(&o)
    );
    let v = json(&o);
    let rules: Vec<&str> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule"].as_str().unwrap())
        .collect();
    for want in ["dangling-ref", "missing-why", "bad-ref", "gap"] {
        assert!(rules.contains(&want), "{want} in {rules:?}");
    }
    // The memory text never goes into the findings.
    assert!(!v.to_string().contains("The queue lives"), "{v}");

    let o = iohr(h.path(), &dir, None, &["memory", "check", "--strict"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("dangling-ref"), "{}", text(&o));
}

#[test]
fn check_flags_a_memory_with_no_checked_date_or_an_old_one_and_remember_dates_it_today() {
    let (h, dir) = repo();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "Edition nine is current.",
            "--why",
            "doc",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let files = memory_files(&dir);
    assert_eq!(files.len(), 1);
    let kept = std::fs::read_to_string(&files[0]).unwrap();
    let today = kept
        .lines()
        .find_map(|l| l.strip_prefix("checked: "))
        .unwrap_or_else(|| panic!("no checked line: {kept}"))
        .to_owned();
    assert_eq!(today.len(), 10, "{today}");
    assert!(
        today.starts_with("20") && today.as_str() >= "2026-11-05",
        "{today}"
    );
    let findings = |h: &Path, extra: &[&str]| {
        let mut a = vec!["memory", "check", "--json"];
        a.extend_from_slice(extra);
        json(&iohr(h, &dir, None, &a))["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["rule"] == "unchecked")
            .map(|f| f["message"].as_str().unwrap().to_owned())
            .collect::<Vec<String>>()
    };
    assert!(findings(h.path(), &[]).is_empty(), "fresh");
    // An old date, then none: each is one plain line naming the limit or the missing key.
    std::fs::write(
        &files[0],
        kept.replace(&format!("checked: {today}"), "checked: 2025-01-01"),
    )
    .unwrap();
    let old = findings(h.path(), &[]);
    assert_eq!(old.len(), 1, "{old:?}");
    assert!(
        old[0].starts_with("checked 2025-01-01,") && old[0].contains("limit 180"),
        "{old:?}"
    );
    assert!(
        findings(h.path(), &["--checked-days", "100000"]).is_empty(),
        "limit"
    );
    std::fs::write(&files[0], kept.replace(&format!("checked: {today}\n"), "")).unwrap();
    let none = findings(h.path(), &[]);
    assert_eq!(none.len(), 1, "{none:?}");
    assert!(none[0].contains("no `checked:` date"), "{none:?}");
    // The limit also comes from `[memory] check_days`.
    let cfg = h.path().join("config/config.toml");
    std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    std::fs::write(&cfg, "[memory]\ncheck_days = 100000\n").unwrap();
    std::fs::write(
        &files[0],
        kept.replace(&format!("checked: {today}"), "checked: 2025-01-01"),
    )
    .unwrap();
    assert!(findings(h.path(), &[]).is_empty(), "config limit is read");
    // Plain text under NO_COLOR and a pipe: the rule and the line, no escape codes.
    std::fs::remove_file(&cfg).unwrap();
    let o = iohr(h.path(), &dir, None, &["memory", "check"]);
    let t = text(&o);
    assert!(t.contains("unchecked") && t.contains("limit 180"), "{t}");
    assert!(!t.contains('\u{1b}'), "{t:?}");
}

#[test]
fn check_finds_duplicates_and_a_clean_store_passes_strict_once_the_gaps_are_covered() {
    let (h, dir) = repo();
    for t in [
        "Run cargo nextest from the cli folder before a push",
        "run cargo nextest from the cli folder before a push.",
    ] {
        let o = iohr(
            h.path(),
            &dir,
            None,
            &["memory", "remember", t, "--why", "ci"],
        );
        assert!(o.status.success(), "{}", text(&o));
    }
    let o = iohr(h.path(), &dir, None, &["memory", "check", "--json"]);
    let v = json(&o);
    let rules: Vec<&str> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule"].as_str().unwrap())
        .collect();
    // The second remember either said "similar" and kept one file, or kept both and check flags it.
    assert!(
        memory_files(&dir).len() == 1 || rules.iter().any(|r| r.contains("duplicate")),
        "{rules:?} {:?}",
        memory_files(&dir)
    );
}

// ---------------------------------------------------------------- drift

#[test]
fn drift_sees_a_ref_changed_in_git_after_the_memory_and_only_reads() {
    let (h, dir) = repo();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "a.rs holds the entry point",
            "--why",
            "read it",
            "--ref",
            "src/a.rs",
            "--json",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let id = json(&o)["entry"]["id"].as_str().unwrap().to_owned();
    // Backdate the memory so the ref's later commit is "after" it.
    let f = dir.join(".inorbit/memory").join(format!("{id}.md"));
    let body = std::fs::read_to_string(&f).unwrap();
    let body: String = body
        .lines()
        .map(|l| {
            if l.starts_with("created:") || l.starts_with("updated:") {
                format!("{}: 2021-01-01T00:00:00Z", l.split(':').next().unwrap())
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&f, &body).unwrap();

    // Not yet changed since: no drift.
    let o = iohr(h.path(), &dir, None, &["memory", "drift", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        json(&o)["code"]["drifts"].as_array().unwrap().is_empty(),
        "{}",
        text(&o)
    );

    std::fs::write(dir.join("src/a.rs"), "fn a() { /* moved */ }\n").unwrap();
    git(
        &dir,
        &["commit", "-q", "-am", "change a"],
        Some("2022-01-01T00:00:00Z"),
    );
    let o = iohr(h.path(), &dir, None, &["memory", "drift", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    let v = json(&o);
    let d = &v["code"]["drifts"][0];
    assert_eq!(d["rule"], "ref-changed", "{v}");
    assert_eq!(d["id"], id.as_str());
    assert_eq!(
        v["service"],
        serde_json::Value::Null,
        "the service is not asked unless sync is on: {v}"
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        body,
        "drift only reads"
    );

    // An uncommitted edit is its own finding; --strict turns drift into exit 1.
    std::fs::write(dir.join("src/a.rs"), "fn a() { /* again */ }\n").unwrap();
    let o = iohr(h.path(), &dir, None, &["memory", "drift", "--json"]);
    let rules: Vec<String> = json(&o)["code"]["drifts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["rule"].as_str().unwrap().to_owned())
        .collect();
    assert!(rules.contains(&"ref-uncommitted".to_owned()), "{rules:?}");
    let o = iohr(h.path(), &dir, None, &["memory", "drift", "--strict"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
}

#[test]
fn drift_outside_git_says_it_skipped_instead_of_guessing() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("plain");
    std::fs::create_dir_all(dir.join(".inorbit")).unwrap();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "x holds y",
            "--why",
            "z",
            "--ref",
            "x.rs",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let o = iohr(h.path(), &dir, None, &["memory", "drift", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(json(&o)["code"]["skipped"].is_string(), "{}", text(&o));
}

// ---------------------------------------------------------------- sync

fn remote_entry(id: &str, text: &str, source: &str) -> serde_json::Value {
    serde_json::json!({"id": id, "kind": "KIND_FACT", "text": text, "source": source,
        "created_at": "2026-11-05T00:00:00Z"})
}

async fn mount_list(server: &MockServer, entries: Vec<serde_json::Value>) {
    let n = entries.len();
    Mock::given(method("GET"))
        .and(path("/v1/memory/entries"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"entries": entries, "total": n, "max_entries": 100}),
            ),
        )
        .mount(server)
        .await;
}

fn posts(sent: &[wiremock::Request]) -> Vec<&wiremock::Request> {
    sent.iter()
        .filter(|r| r.method.as_str() == "POST" || r.method.as_str() == "DELETE")
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_dry_run_plans_a_push_sends_nothing_and_a_real_run_links_the_ids_back() {
    let (h, dir) = repo();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "We use sqlx",
            "--why",
            "typed",
            "--json",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let id = json(&o)["entry"]["id"].as_str().unwrap().to_owned();
    let server = MockServer::start().await;
    mount_list(&server, vec![]).await;
    Mock::given(method("POST"))
        .and(path("/v1/memory/entries"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"entry": remote_entry("rem1", "We use sqlx", "project:shop")}),
        ))
        .mount(&server)
        .await;
    let base = server.uri();

    let o = iohr(
        h.path(),
        &dir,
        Some(&base),
        &["memory", "sync", "--dry-run", "--json"],
    );
    assert!(o.status.success(), "{}", text(&o));
    let v = json(&o);
    assert_eq!(v["dry_run"], true, "{v}");
    assert_eq!(v["rows"][0]["action"], "would push", "{v}");
    let sent = server.received_requests().await.unwrap();
    assert!(posts(&sent).is_empty(), "a dry run sends nothing");
    assert!(
        !std::fs::read_to_string(dir.join(".inorbit/memory").join(format!("{id}.md")))
            .unwrap()
            .contains("remote_id")
    );

    let o = iohr(h.path(), &dir, Some(&base), &["memory", "sync", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    let sent = server.received_requests().await.unwrap();
    let p = posts(&sent);
    assert_eq!(p.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&p[0].body).unwrap();
    assert_eq!(body["source"], "project:shop");
    assert_eq!(body["text"], "We use sqlx");
    assert!(
        sent.iter().all(|r| !r.url.to_string().contains("sqlx")),
        "memory text is never in a URL"
    );
    let file =
        std::fs::read_to_string(dir.join(".inorbit/memory").join(format!("{id}.md"))).unwrap();
    assert!(file.contains("remote_id: rem1"), "{file}");
    assert!(file.contains("remote_sha:"), "{file}");
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_stops_on_a_conflict_overwrites_nothing_and_exits_one() {
    let (h, dir) = repo();
    let o = iohr(
        h.path(),
        &dir,
        None,
        &[
            "memory",
            "remember",
            "We use sqlx",
            "--why",
            "typed",
            "--json",
        ],
    );
    let id = json(&o)["entry"]["id"].as_str().unwrap().to_owned();
    let server = MockServer::start().await;
    mount_list(&server, vec![]).await;
    Mock::given(method("POST"))
        .and(path("/v1/memory/entries"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"entry": remote_entry("rem1", "We use sqlx", "project:shop")}),
        ))
        .mount(&server)
        .await;
    let base = server.uri();
    let o = iohr(h.path(), &dir, Some(&base), &["memory", "sync"]);
    assert!(o.status.success(), "{}", text(&o));

    // Both sides change: the file here, and the entry on the service (a new id and text).
    let f = dir.join(".inorbit/memory").join(format!("{id}.md"));
    let edited = std::fs::read_to_string(&f)
        .unwrap()
        .replace("We use sqlx", "We use sqlx and sea-query");
    std::fs::write(&f, &edited).unwrap();
    server.reset().await;
    mount_list(
        &server,
        vec![remote_entry("rem1", "We use diesel", "project:shop")],
    )
    .await;
    let o = iohr(h.path(), &dir, Some(&base), &["memory", "sync", "--json"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let v = json(&o);
    assert_eq!(v["conflicts"], 1, "{v}");
    assert_eq!(v["rows"][0]["action"], "conflict", "{v}");
    let sent = server.received_requests().await.unwrap();
    assert!(
        posts(&sent).is_empty(),
        "a conflict sends nothing: {sent:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        edited,
        "the file is untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_does_not_pull_a_remote_only_entry_unless_asked_and_a_secret_is_never_pushed() {
    let (h, dir) = repo();
    let server = MockServer::start().await;
    mount_list(
        &server,
        vec![remote_entry(
            "rem9",
            "Deploys roll on fridays",
            "project:shop",
        )],
    )
    .await;
    let base = server.uri();
    let o = iohr(h.path(), &dir, Some(&base), &["memory", "sync", "--json"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(memory_files(&dir), Vec::<PathBuf>::new());
    let o = iohr(h.path(), &dir, Some(&base), &["memory", "sync", "--pull"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(memory_files(&dir).len(), 1);
    // Signed out, sync is a plain sign-in error, never a silent no-op; local stays untouched.
    let o = iohr(h.path(), &dir, None, &["memory", "sync"]);
    assert!(!o.status.success(), "{}", text(&o));
    assert_eq!(memory_files(&dir).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn account_scope_with_sync_on_uses_the_service_and_keeps_text_out_of_the_url() {
    let (h, dir) = repo();
    std::fs::create_dir_all(h.path().join("config")).unwrap();
    std::fs::write(
        h.path().join("config/config.toml"),
        "[memory]\nsync = true\n",
    )
    .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/memory/entries"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"entry": remote_entry("m1", "we use sqlx", "cli")}),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/memory/recall"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"entries": [remote_entry("m1", "we use sqlx", "cli")]}),
        ))
        .mount(&server)
        .await;
    let base = server.uri();
    let o = iohr(
        h.path(),
        &dir,
        Some(&base),
        &["memory", "remember", "we use sqlx", "--scope", "account"],
    );
    assert!(o.status.success(), "{}", text(&o));
    let o = iohr(
        h.path(),
        &dir,
        Some(&base),
        &["memory", "recall", "db", "--scope", "account", "--json"],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(json(&o)["entries"][0]["id"], "m1");
    let sent = server.received_requests().await.unwrap();
    assert!(sent.iter().all(|r| !r.url.to_string().contains("sqlx")));
    assert!(
        memory_files(&dir).is_empty(),
        "account memory is not written to the repo"
    );
}

// ---------------------------------------------------------------- init

#[test]
fn project_init_and_atlas_init_create_the_local_memory_with_no_network() {
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
    let readme = std::fs::read_to_string(dir.join(".inorbit/memory/README.md")).unwrap();
    assert!(readme.contains("iohr memory recall"), "{readme}");
    let seeds = memory_files(&dir).len() - 1;
    assert!(seeds > 0, "the roles' seed memories are written locally");
    assert!(!dir.join("atlas/memory").exists());
    // The seeds read back through the same commands, and a clean `check` has no dangling ref.
    let o = iohr(h.path(), &dir, None, &["memory", "list", "--json"]);
    assert_eq!(ids(&json(&o)).len(), seeds, "{}", text(&o));
    let o = iohr(h.path(), &dir, None, &["memory", "check", "--json"]);
    let v = json(&o);
    assert!(
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["rule"] != "dangling-ref"),
        "{v}"
    );
    // A person who forgets a seed keeps it forgotten: init again does not bring it back.
    let first = ids(&json(&iohr(
        h.path(),
        &dir,
        None,
        &["memory", "list", "--json"],
    )))[0]
        .clone();
    let o = iohr(h.path(), &dir, None, &["memory", "forget", &first]);
    assert!(o.status.success(), "{}", text(&o));
    let o = iohr(h.path(), &dir, None, &["atlas", "init"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(memory_files(&dir).len() - 1, seeds - 1);
}
