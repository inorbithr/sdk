//! `iohr init` and `iohr project sync` make the project's space in InOrbit (RFC 0127.1): the
//! binary with a private HOME and a local Decisions API (port 0) that records what it was asked.
//! Nothing of the machine is read; a run with no server at all queues the sync and still succeeds.

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
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SELF: &str = "acc_test1";
const TEAM: &str = "acc_team1";
const SPACES: &str = "/v1/decisions/spaces";

fn token() -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": ["decisions:read", "decisions:write", "account:read"], "org": SELF,
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

fn space(id: &str, account: &str, name: &str, kind: &str) -> serde_json::Value {
    serde_json::json!({
        "space_id": id, "account_id": account, "slug": name, "name": name,
        "description": "", "created_at": "2026-11-07T00:00:00Z",
        "updated_at": "2026-11-07T00:00:00Z", "documents": 0, "open": 0, "awaiting_review": 0,
        "parent_id": "", "kind": kind, "department": "", "depth": 0, "children": 0,
        "access_floor": "", "effective_access_floor": "", "templates": []
    })
}

async fn me(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "subject": "ak_tok1", "kind": "client", "org": SELF, "scopes": ["decisions:write"]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "account": {"id": SELF, "slug": "nevio", "name": "Nevio", "plan": "free"},
            "teams": [{"id": TEAM, "slug": "acme", "name": "Acme", "plan": "team"}]
        })))
        .mount(server)
        .await;
}

async fn listing(server: &MockServer, spaces: &[serde_json::Value]) {
    Mock::given(method("GET"))
        .and(path(SPACES))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "spaces": spaces, "next_page_token": "" })),
        )
        .mount(server)
        .await;
}

async fn posts(server: &MockServer) -> Vec<serde_json::Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == SPACES)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn init_args<'a>(dir: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec![
        "init",
        "shop",
        "--template",
        "blank",
        "--lang",
        "python",
        "--no-smoke",
        "--no-plan",
        "--yes",
        "--dir",
        dir,
    ];
    a.extend_from_slice(extra);
    a
}

#[tokio::test(flavor = "multi_thread")]
async fn init_makes_the_space_under_the_team_links_the_folder_and_fills_the_manifest() {
    let server = MockServer::start().await;
    me(&server).await;
    listing(&server, &[]).await;
    Mock::given(method("POST"))
        .and(path(SPACES))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "space": space("lab_shop1", TEAM, "shop", "project") }),
        ))
        .mount(&server)
        .await;
    let h = tempfile::tempdir().unwrap();
    let dir: PathBuf = h.path().join("work/shop");
    let d = dir.to_str().unwrap();
    let o = iohr(
        h.path(),
        h.path(),
        Some(&server.uri()),
        &init_args(d, &["--team", "acme", "--visibility", "managers"]),
    );
    assert!(o.status.success(), "{}", text(&o));
    let said = text(&o);
    assert!(
        said.contains("done: the project's space in inorbit"),
        "{said}"
    );
    assert!(said.contains("lab_shop1"), "{said}");
    let sent = posts(&server).await;
    assert_eq!(sent.len(), 1, "one space made: {sent:?}");
    assert_eq!(sent[0]["account_id"], TEAM);
    assert_eq!(sent[0]["kind"], "project");
    assert_eq!(sent[0]["slug"], "shop");
    assert_eq!(sent[0]["access_floor"], "internal", "managers read it");
    let link = std::fs::read_to_string(dir.join(".inorbit/space")).unwrap();
    assert!(link.contains("lab_shop1"), "{link}");
    let manifest = std::fs::read_to_string(dir.join("inorbit.toml")).unwrap();
    assert!(manifest.contains("space_id = \"lab_shop1\""), "{manifest}");
    assert!(
        !said.contains("TOKENSIGNATUREMARKER"),
        "the token stays out of the output"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_sync_finds_the_space_and_makes_no_second_one() {
    let server = MockServer::start().await;
    me(&server).await;
    listing(&server, &[space("lab_shop1", SELF, "shop", "project")]).await;
    let h = tempfile::tempdir().unwrap();
    let dir: PathBuf = h.path().join("work/shop");
    let d = dir.to_str().unwrap();
    let o = iohr(h.path(), h.path(), Some(&server.uri()), &init_args(d, &[]));
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("found"), "{}", text(&o));
    assert_eq!(posts(&server).await.len(), 0, "nothing was made twice");
    let one = format!("{SPACES}/lab_shop1");
    Mock::given(method("GET"))
        .and(path(&one))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "space": space("lab_shop1", SELF, "shop", "project") }),
        ))
        .mount(&server)
        .await;
    let o = iohr(h.path(), &dir, Some(&server.uri()), &["project", "sync"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(posts(&server).await.len(), 0);
    // A teammate whose sign-in cannot open the space is told who to ask, and nothing is made.
    server.reset().await;
    me(&server).await;
    Mock::given(method("GET"))
        .and(path(&one))
        .respond_with(ResponseTemplate::new(403).set_body_string("denied"))
        .mount(&server)
        .await;
    let o = iohr(h.path(), &dir, Some(&server.uri()), &["project", "sync"]);
    assert!(!o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("ask a team owner or admin"),
        "{}",
        text(&o)
    );
    assert_eq!(posts(&server).await.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_team_the_person_is_not_in_is_refused_before_anything_is_made() {
    let server = MockServer::start().await;
    me(&server).await;
    listing(&server, &[]).await;
    let h = tempfile::tempdir().unwrap();
    let dir: PathBuf = h.path().join("work/shop");
    let d = dir.to_str().unwrap();
    let o = iohr(
        h.path(),
        h.path(),
        Some(&server.uri()),
        &init_args(d, &["--team", "strangers"]),
    );
    assert!(!o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("not an account you belong to"),
        "{}",
        text(&o)
    );
    assert_eq!(posts(&server).await.len(), 0);
    assert!(!dir.exists(), "no folder was made");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_the_server_gives_queues_the_sync_and_init_still_succeeds() {
    let server = MockServer::start().await;
    me(&server).await;
    listing(&server, &[]).await;
    Mock::given(method("POST"))
        .and(path(SPACES))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&server)
        .await;
    let h = tempfile::tempdir().unwrap();
    let dir: PathBuf = h.path().join("work/shop");
    let d = dir.to_str().unwrap();
    let o = iohr(h.path(), h.path(), Some(&server.uri()), &init_args(d, &[]));
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("queued: the project's space"),
        "{}",
        text(&o)
    );
    let q = h
        .path()
        .join(".inorbit/projects/shop/space-sync.production.toml");
    assert!(q.is_file(), "the queue is kept at {}", q.display());
    // A later sync, once the server answers, completes it and clears the queue.
    server.reset().await;
    me(&server).await;
    listing(&server, &[]).await;
    Mock::given(method("POST"))
        .and(path(SPACES))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "space": space("lab_shop2", SELF, "shop", "project") }),
        ))
        .mount(&server)
        .await;
    let o = iohr(h.path(), &dir, Some(&server.uri()), &["project", "sync"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(!q.exists(), "the queue is cleared");
    assert!(
        std::fs::read_to_string(dir.join(".inorbit/space"))
            .unwrap()
            .contains("lab_shop2")
    );
}

#[test]
fn signed_out_init_queues_says_so_and_never_fails() {
    let h = tempfile::tempdir().unwrap();
    let dir: PathBuf = h.path().join("work/shop");
    let d = dir.to_str().unwrap();
    let o = iohr(h.path(), h.path(), None, &init_args(d, &[]));
    assert!(o.status.success(), "{}", text(&o));
    let said = text(&o);
    assert!(said.contains("queued: the project's space"), "{said}");
    assert!(said.contains("iohr project sync"), "{said}");
    assert!(
        h.path()
            .join(".inorbit/projects/shop/space-sync.production.toml")
            .is_file()
    );
}

#[test]
fn no_sync_skips_the_space_and_a_dry_run_changes_nothing() {
    let h = tempfile::tempdir().unwrap();
    let dir: PathBuf = h.path().join("work/shop");
    let d = dir.to_str().unwrap();
    let o = iohr(h.path(), h.path(), None, &init_args(d, &["--no-sync"]));
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("skipped: the project's space"),
        "{}",
        text(&o)
    );
    let other = h.path().join("work/dry");
    let o = iohr(
        h.path(),
        h.path(),
        None,
        &[
            "init",
            "dry",
            "--template",
            "blank",
            "--dry-run",
            "--yes",
            "--dir",
            other.to_str().unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("would: make or find the project's space"),
        "{}",
        text(&o)
    );
    assert!(!other.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_roles_go_to_the_account_only_with_the_account_step() {
    let roles = |reqs: &[wiremock::Request]| {
        reqs.iter()
            .filter(|r| r.url.path().starts_with("/v1/coordinator/personalities"))
            .count()
    };
    // Signed in but local only: not one call to the roles API, and a line that says so.
    let server = MockServer::start().await;
    me(&server).await;
    let h = tempfile::tempdir().unwrap();
    let dir: PathBuf = h.path().join("work/shop");
    let d = dir.to_str().unwrap();
    let o = iohr(
        h.path(),
        h.path(),
        Some(&server.uri()),
        &init_args(d, &["--no-sync"]),
    );
    assert!(o.status.success(), "{}", text(&o));
    let said = text(&o);
    assert!(
        said.contains("built-in roles stay in atlas/roles/"),
        "{said}"
    );
    assert!(said.contains("--no-sync"), "{said}");
    let got = server.received_requests().await.unwrap();
    assert_eq!(roles(&got), 0, "no roles call: {got:?}");
    assert!(got.iter().all(|r| r.method.as_str() == "GET"), "{got:?}");
    // Signed out: the same, with its own reason.
    let o = iohr(
        h.path(),
        h.path(),
        None,
        &init_args(h.path().join("work/two").to_str().unwrap(), &[]),
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("you are not signed in"), "{}", text(&o));
    // With the account step the roles are imported (the list is read first).
    let server = MockServer::start().await;
    me(&server).await;
    listing(&server, &[space("lab_shop1", SELF, "shop", "project")]).await;
    let three = h.path().join("work/three");
    let o = iohr(
        h.path(),
        h.path(),
        Some(&server.uri()),
        &init_args(three.to_str().unwrap(), &[]),
    );
    assert!(o.status.success(), "{}", text(&o));
    let got = server.received_requests().await.unwrap();
    assert!(
        roles(&got) > 0,
        "the account step imports the roles: {got:?}"
    );
}
