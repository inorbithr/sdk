//! The binary against a mock API: exit codes, the stores, retries, and that no token
//! ever reaches the output (SR-24).

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use wiremock::matchers::{body_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ACCOUNT: &str = "acc_test1";

/// An unsigned token with InOrbit claims. Its signature part is a fixed marker so a
/// leak is easy to find.
fn token(scopes: &[&str]) -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": scopes, "org": ACCOUNT, "plan": "free", "key": "key_1"
        }))
    )
}

/// What a child process needs to run at all; `SYSTEMROOT` is required for sockets on
/// Windows. Nothing that could hold a credential is passed on.
fn kept_env() -> Vec<(String, std::ffi::OsString)> {
    ["PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"]
        .into_iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (k.to_owned(), v)))
        .collect()
}

struct Run<'a> {
    server: &'a MockServer,
    config: &'a Path,
}

impl Run<'_> {
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
        c.args(args)
            .env_clear()
            .env("IOHR_BASE_URL", self.server.uri())
            .env("IOHR_CONFIG_DIR", self.config)
            .envs(kept_env());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).stdin(Stdio::null()).output().unwrap()
    }

    fn with_token(&self, token: &str, args: &[&str]) -> Output {
        self.cmd(args)
            .env("IOHR_TOKEN", token)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn with_token_and_stdin(&self, token: &str, input: &str, args: &[&str]) -> Output {
        spawn_with_stdin(self.cmd(args).env("IOHR_TOKEN", token), input)
    }

    fn with_stdin(&self, input: &str, args: &[&str]) -> Output {
        spawn_with_stdin(&mut self.cmd(args), input)
    }
}

fn spawn_with_stdin(cmd: &mut Command, input: &str) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

async fn me(server: &MockServer, bearer: &str) {
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", format!("Bearer {bearer}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "subject": "ak_tok1", "kind": "client", "org": ACCOUNT, "scopes": ["identity:read"]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/me"))
        .respond_with(ResponseTemplate::new(403).set_body_string("RBAC: access denied"))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn login_with_a_token_from_stdin_keeps_it_in_an_owner_only_file() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read"]);
    me(&server, &t).await;

    let o = r.with_stdin(
        &format!("{t}\n"),
        &[
            "login",
            "--with-token",
            "--insecure-storage",
            "--profile",
            "ci",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(!text(&o).contains("TOKENSIGNATUREMARKER"));

    let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(config.contains("account = \"acc_test1\"") && !config.contains("TOKENSIGNATUREMARKER"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.path().join("secrets/ci.acc_test1"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let o = r.run(&["whoami"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("ak_tok1"));

    let o = r.run(&["logout"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(!dir.path().join("secrets/ci.acc_test1").exists());
    assert_eq!(code(&r.run(&["whoami"])), 3, "signed out");
}

#[tokio::test(flavor = "multi_thread")]
async fn iohr_token_works_with_nothing_on_disk() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read"]);
    me(&server, &t).await;

    let o = r.with_token(&t, &["whoami", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["account"], ACCOUNT);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "nothing written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_codes_say_what_went_wrong() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["radar:read"]);
    Mock::given(path("/v1/accounts/me"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "code": "forbidden", "error": "this token may not read the account"
        })))
        .mount(&server)
        .await;
    Mock::given(path("/v1/radar/digests"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Jwt is expired"))
        .mount(&server)
        .await;

    assert_eq!(code(&r.run(&["whoami"])), 3, "no profile");
    let o = r.with_token(&t, &["accounts", "list"]);
    assert_eq!(code(&o), 4, "{}", text(&o));
    assert!(text(&o).contains("account:read"));
    assert_eq!(
        code(&r.with_token(&t, &["api", "GET", "/v1/radar/digests"])),
        3,
        "refused token"
    );
    assert_eq!(
        code(&r.with_token("ak_123", &["whoami"])),
        3,
        "a key id is not a token"
    );
    assert_eq!(code(&r.run(&["api", "GET"])), 2, "usage");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_in_argv_is_refused_and_not_echoed() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read"]);
    let o = r.run(&["login", "--with-token", &t]);
    assert_eq!(code(&o), 2);
    assert!(!text(&o).contains("TOKENSIGNATUREMARKER"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_is_retried_after_a_503_and_post_is_not() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["radar:read"]);
    Mock::given(method("GET"))
        .and(path("/v1/radar/digests"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/radar/digests"))
        .and(query_param("limit", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"digests": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/thing"))
        .and(body_json(serde_json::json!({"name": "x", "n": 3})))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let o = r.with_token(&t, &["api", "GET", "/v1/radar/digests", "-f", "limit=2"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let o = r.with_token(
        &t,
        &["api", "POST", "/v1/thing", "-f", "name=x", "-F", "n=3"],
    );
    assert_eq!(code(&o), 1);
    let posts = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|q| q.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 1, "a POST is sent once");
}

#[tokio::test(flavor = "multi_thread")]
async fn api_all_walks_every_page_into_one_answer() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["finance:read"]);
    // Raw bodies keep the server's key order: the list comes first, and another
    // array (`party_ids`) sorts before it, as finance's transactions answer does.
    let page = |items: &str, next: &str| {
        ResponseTemplate::new(200).set_body_raw(
            format!(r#"{{"things":[{items}],"party_ids":["p1"],"next_page_token":"{next}"}}"#),
            "application/json",
        )
    };
    Mock::given(method("GET"))
        .and(path("/v1/things"))
        .and(query_param("kind", "a"))
        .and(query_param_is_missing("page_token"))
        .respond_with(page("1,2", "t2"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/things"))
        .and(query_param("kind", "a"))
        .and(query_param("page_token", "t2"))
        .respond_with(page("3", "t3"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/things"))
        .and(query_param("kind", "a"))
        .and(query_param("page_token", "t3"))
        .respond_with(page("4", ""))
        .mount(&server)
        .await;

    let o = r.with_token(&t, &["api", "GET", "/v1/things", "-f", "kind=a", "--all"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["things"], serde_json::json!([1, 2, 3, 4]));
    assert_eq!(
        v["party_ids"],
        serde_json::json!(["p1"]),
        "other fields from page one"
    );
    assert_eq!(v["next_page_token"], "");

    // A bound stops the walk and keeps the token to go on with.
    let o = r.with_token(
        &t,
        &[
            "api",
            "GET",
            "/v1/things",
            "-f",
            "kind=a",
            "--paginate",
            "--max-pages",
            "2",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["things"], serde_json::json!([1, 2, 3]));
    assert_eq!(v["next_page_token"], "t3");
    assert!(String::from_utf8_lossy(&o.stderr).contains("stopped after 2 pages"));

    // An answer that does not page is printed as it is.
    me(&server, &t).await;
    let o = r.with_token(&t, &["api", "GET", "/v1/me", "--all"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        serde_json::from_slice::<serde_json::Value>(&o.stdout)
            .unwrap()
            .is_object()
    );

    // Usage: GET only, and the walk owns page_token; --max-pages needs --all.
    for args in [
        vec!["api", "POST", "/v1/things", "--all"],
        vec!["api", "GET", "/v1/things", "-f", "page_token=x", "--all"],
        vec!["api", "GET", "/v1/things", "--max-pages", "3"],
    ] {
        assert_eq!(code(&r.with_token(&t, &args)), 2, "{args:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn token_create_prints_the_new_token_alone_on_stdout() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read"]);
    Mock::given(method("POST"))
        .and(path(format!("/v1/accounts/orgs/{ACCOUNT}/tokens")))
        .and(body_json(
            serde_json::json!({"name": "ci", "scopes": ["radar:read"], "expires_in_days": 7}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": {"id": "key_9", "name": "ci", "scopes": ["radar:read"]},
            "access_token": "NEWTOKENVALUE", "expires_at": "2026-10-09T10:00:00Z"
        })))
        .mount(&server)
        .await;
    let o = r.with_token(
        &t,
        &[
            "token",
            "create",
            "--name",
            "ci",
            "--scope",
            "radar:read",
            "--days",
            "7",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(String::from_utf8_lossy(&o.stdout), "NEWTOKENVALUE\n");
    assert!(String::from_utf8_lossy(&o.stderr).contains("shown once"));
}

#[tokio::test(flavor = "multi_thread")]
async fn domains_add_verify_confirm_list_and_remove() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["domains:read", "domains:write"]);
    let base = format!("/v1/accounts/orgs/{ACCOUNT}/domains");
    let domain = serde_json::json!({
        "domain": "staging.acme.hr", "scope": "subdomain", "status": "pending",
        "record_name": "_inorbit-verify.staging.acme.hr",
        "record_value": "inorbit-verify=ABCDEF234567", "created_at": "2026-10-03T10:00:00Z",
        "expires_at": "2026-10-10T10:00:00Z"
    });
    Mock::given(method("POST"))
        .and(path(&base))
        .and(body_json(
            serde_json::json!({"domain": "staging.acme.hr", "scope": "subdomain"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"domain": domain})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{base}/staging.acme.hr/check")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "seen",
            "resolvers": [{"name": "cloudflare", "seen": true, "value": "inorbit-verify=ABCDEF234567"},
                          {"name": "google", "seen": true}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{base}/staging.acme.hr/confirm")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "domain": {"domain": "staging.acme.hr", "status": "verified"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(&base))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"domains": [domain]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{base}/staging.acme.hr")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let o = r.with_token(&t, &["domains", "add", "Staging.Acme.HR.", "--subdomain"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        stdout.contains("_inorbit-verify.staging.acme.hr")
            && stdout.contains("inorbit-verify=ABCDEF234567"),
        "{stdout}"
    );
    let o = r.with_token(&t, &["domains", "verify", "staging.acme.hr", "--wait"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("iohr domains confirm staging.acme.hr"),
        "{}",
        text(&o)
    );
    let o = r.with_token(&t, &["domains", "confirm", "staging.acme.hr"]);
    assert!(text(&o).contains("verified"), "{}", text(&o));
    let o = r.with_token(&t, &["domains", "list"]);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("staging.acme.hr"),
        "{}",
        text(&o)
    );
    let o = r.with_token(&t, &["domains", "rm", "staging.acme.hr"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let o = r.with_token(&t, &["domains", "add", "not a domain"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_to_another_host_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(["whoami"])
        .env_clear()
        .envs(kept_env())
        .env("IOHR_BASE_URL", "http://api.example.com")
        .env("IOHR_CONFIG_DIR", dir.path())
        .env("IOHR_TOKEN", token(&["identity:read"]))
        .output()
        .unwrap();
    assert_eq!(code(&o), 1);
    assert!(text(&o).contains("https"));
}

/// Every command, with `--verbose`, against a server that answers everything: the
/// caller's token appears nowhere in the output (SR-24, SR-13).
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines, reason = "one list: every command that calls")]
async fn verbose_output_never_shows_the_token() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read", "account:read"]);
    // Mounted first: wiremock answers with the first mock that matches.
    Mock::given(method("GET"))
        .and(path("/v1/openapi.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cut_document(
            ACCOUNT,
            &["identity:read", "account:read"],
            false,
        )))
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "subject": "ak_tok1", "account": {"id": ACCOUNT}, "teams": [], "keys": [], "paths": {},
            "token": {"id": "k"}, "access_token": "NEWTOKENVALUE", "expires_at": "",
            "connection": connection_json(), "connector": incident_io(),
            "result": {"ok": true, "status": "ok"}, "grant": {"id": "gnt_1"}
        })))
        .mount(&server)
        .await;
    let out_file = dir.path().join("doc.json");
    let out_file = out_file.to_str().unwrap();
    // The sdk commands need a profile and a stamped document; `sdk check` reads the
    // lock `sdk generate` leaves beside its output.
    let o = r.with_stdin(
        &format!("{t}\n"),
        &[
            "login",
            "--with-token",
            "--insecure-storage",
            "--profile",
            "ci",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    let gen_dir = dir.path().join("proj/src/iohr");
    let gen_dir = gen_dir.to_str().unwrap();
    let lock_file = dir.path().join("proj/src/iohr.lock");
    let lock_file = lock_file.to_str().unwrap();
    let secret_file = dir.path().join("key.txt");
    std::fs::write(&secret_file, format!("{SECRET}\n")).unwrap();
    let secret_arg = format!("api_key={}", secret_file.to_str().unwrap());
    let commands: Vec<Vec<&str>> = vec![
        vec!["whoami"],
        vec!["accounts", "list"],
        vec!["token", "list"],
        vec!["token", "revoke", "key_1"],
        vec!["token", "create", "--name", "n", "--scope", "radar:read"],
        vec!["api", "GET", "/v1/me", "-f", "a=b"],
        vec!["api", "GET", "/v1/me", "--all"],
        vec!["openapi", "pull", "-o", out_file],
        vec![
            "sdk", "generate", "--lang", "rust", "--for", "ci", "--out", gen_dir,
        ],
        vec!["sdk", "check", "--lock", lock_file],
        vec!["domains", "list"],
        vec!["domains", "add", "acme.hr"],
        vec!["domains", "verify", "acme.hr"],
        vec!["domains", "confirm", "acme.hr"],
        vec!["domains", "rm", "acme.hr"],
        vec!["connectors", "list"],
        vec!["connectors", "show", "incident-io"],
        vec!["connections", "list"],
        vec!["connections", "show", "con_1"],
        vec![
            "connections",
            "add",
            "incident-io",
            "--secret-file",
            &secret_arg,
        ],
        vec![
            "connections",
            "reconnect",
            "con_1",
            "--secret-file",
            &secret_arg,
        ],
        vec!["connections", "test", "con_1"],
        vec!["connections", "history"],
        vec!["connections", "pause", "con_1"],
        vec!["connections", "resume", "con_1"],
        vec!["connections", "rename", "con_1", "pd"],
        vec![
            "connections",
            "grant",
            "con_1",
            "--to",
            "product:reliability",
            "--actions",
            "create_incident",
        ],
        vec!["connections", "grants", "con_1"],
        vec!["connections", "revoke-grant", "con_1", "gnt_1"],
        vec!["connections", "delete", "con_1", "--yes"],
    ];
    for args in commands {
        let mut args = args.clone();
        args.push("--verbose");
        let o = r.with_token(&t, &args);
        assert_eq!(code(&o), 0, "{args:?}: {}", text(&o));
        let all = text(&o);
        assert!(
            !all.contains("TOKENSIGNATUREMARKER") && !all.contains(SECRET),
            "{args:?} leaked the token or a connection's secret"
        );
        assert!(
            all.contains("request id"),
            "{args:?} printed no verbose line"
        );
    }
    // An offline command makes no call, so it prints no request line; it must not
    // print the token either.
    let lab = dir.path().join("lab/rfcs");
    std::fs::create_dir_all(&lab).unwrap();
    std::fs::write(lab.join("0001-a.md"), LAB_DOC).unwrap();
    let lab = lab.to_str().unwrap();
    let args = ["lab", "check", lab, "--verbose"];
    let o = r.with_token(&t, &args);
    assert_eq!(code(&o), 0, "{args:?}: {}", text(&o));
    let all = text(&o);
    assert!(
        !all.contains("TOKENSIGNATUREMARKER"),
        "{args:?} leaked the token"
    );
    assert!(!all.contains("request id"), "{args:?} made a call");
}

const LAB_DOC: &str = "---\ntitle: A decision\nstatus: open\ndate: 2026-10-03\npublic: true\nlab: core\nsummary: One sentence.\n---\n\n## Problem\n\nThe site is www.example.com.\n\n## Status log\n\n- 2026-10-03: opened.\n";

/// `iohr lab check` with no network: 0 for clean documents, 1 with every finding
/// printed, 2 for a path that is not there; a lab's config adds its own words.
#[test]
fn lab_check_says_what_to_fix() {
    let dir = tempfile::tempdir().unwrap();
    let rfcs = dir.path().join("docs/rfcs");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::write(rfcs.join("0001-a.md"), LAB_DOC).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_iohr"))
            .args(args)
            .current_dir(dir.path())
            .env_clear()
            .envs(kept_env())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };

    let o = run(&["lab", "check"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("1 document checked, 0 findings"),
        "{}",
        text(&o)
    );

    std::fs::write(
        rfcs.join("0002-b.md"),
        LAB_DOC.replace("www.example.com", "db.example.com on :5432"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("docs/lab")).unwrap();
    std::fs::write(
        dir.path().join("docs/lab/redaction.json"),
        r#"{"domains": ["example.com"], "words": ["staging-eu"]}"#,
    )
    .unwrap();
    std::fs::write(
        rfcs.join("0003-c.md"),
        LAB_DOC.replace("A decision", "On staging-eu"),
    )
    .unwrap();
    let o = run(&["lab", "check"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    let all = text(&o);
    assert!(all.contains("0002-b.md:12 [domain]"), "{all}");
    assert!(all.contains("0002-b.md:12 [port]"), "{all}");
    assert!(all.contains("0003-c.md:2 [denylist]"), "{all}");
    assert!(all.contains("3 documents checked, 3 findings"), "{all}");

    let o = run(&["--json", "lab", "check", "docs/rfcs/0002-b.md"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    let findings: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(findings.as_array().map(Vec::len), Some(2), "{findings}");
    assert_eq!(findings[0]["rule"], "domain");

    let o = run(&["lab", "check", "no/such/folder"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
}

/// A provider at the mock server's own address, for a person's sign-in.
async fn mock_provider(server: &MockServer) {
    let uri = server.uri();
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": uri,
            "authorization_endpoint": format!("{uri}/oauth2/auth"),
            "token_endpoint": format!("{uri}/oauth2/token"),
            "device_authorization_endpoint": format!("{uri}/oauth2/device/auth"),
            "revocation_endpoint": format!("{uri}/oauth2/revoke"),
        })))
        .mount(server)
        .await;
}

fn person_tokens(issuer: &str) -> serde_json::Value {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    let jwt = |c: serde_json::Value| {
        format!(
            "{}.{}.TOKENSIGNATUREMARKER",
            enc(serde_json::json!({"alg": "RS256"})),
            enc(c)
        )
    };
    serde_json::json!({
        "access_token": jwt(serde_json::json!({
            "sub": "person-1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": ["openid", "offline_access", "iohr.api"], "org": ACCOUNT, "plan": "free"
        })),
        "token_type": "bearer", "expires_in": 900, "refresh_token": "REFRESHMARKER",
        "id_token": jwt(serde_json::json!({"iss": issuer, "aud": ["iohr-cli"], "sub": "person-1"})),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_signs_in_with_a_device_code_and_logout_revokes() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    mock_provider(&server).await;
    let uri = server.uri();
    Mock::given(method("POST"))
        .and(path("/oauth2/device/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "dc", "user_code": "ABCD2345", "verification_uri": format!("{uri}/oauth2/device/verify"),
            "expires_in": 600, "interval": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(person_tokens(&uri)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"subject": "person-1", "kind": "person"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"account": {"id": ACCOUNT, "plan": "free"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/revoke"))
        .and(wiremock::matchers::body_string_contains(
            "token=REFRESHMARKER",
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let login = r
        .cmd(&["login", "--device", "--insecure-storage", "--profile", "me"])
        .env("IOHR_ISSUER", &uri)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&login), 0, "{}", text(&login));
    let shown = text(&login);
    assert!(
        shown.contains("ABCD2345") && shown.contains("/oauth2/device/verify"),
        "{shown}"
    );
    assert!(!shown.contains("TOKENSIGNATUREMARKER") && !shown.contains("REFRESHMARKER"));
    let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(
        config.contains("kind = \"person\"") && config.contains(&uri),
        "{config}"
    );
    assert!(!config.contains("REFRESHMARKER"));

    let who = r
        .cmd(&["whoami", "--verbose"])
        .env("IOHR_ISSUER", &uri)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&who), 0, "{}", text(&who));
    assert!(text(&who).contains("person"));
    assert!(!text(&who).contains("TOKENSIGNATUREMARKER") && !text(&who).contains("REFRESHMARKER"));

    let out = r.run(&["logout"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!dir.path().join("secrets/me.acc_test1").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn logout_keeps_the_profile_when_revoking_fails() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    mock_provider(&server).await;
    Mock::given(method("POST"))
        .and(path("/oauth2/revoke"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let uri = server.uri();
    std::fs::create_dir_all(dir.path().join("secrets")).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!(
            "default = \"me\"\n[profiles.me]\nkind = \"person\"\naccount = \"{ACCOUNT}\"\nstorage = \"file\"\nissuer = \"{uri}\"\nclient_id = \"iohr-cli\"\n"
        ),
    )
    .unwrap();
    let session = serde_json::json!({"refresh_token": "rt", "access_token": "x", "expires_at": 4_102_444_800_i64});
    std::fs::write(
        dir.path().join(format!("secrets/me.{ACCOUNT}")),
        session.to_string(),
    )
    .unwrap();
    let out = r.run(&["logout"]);
    assert_eq!(code(&out), 1, "{}", text(&out));
    assert!(text(&out).contains("Nothing was removed"));
    assert!(dir.path().join(format!("secrets/me.{ACCOUNT}")).exists());
}

/// A document with a stamp, as the gateway answers `/v1/openapi.json`.
fn cut_document(account: &str, scopes: &[&str], with_radar: bool) -> serde_json::Value {
    let mut paths = serde_json::json!({
        "/v1/me": { "get": { "operationId": "me", "tags": ["identity"], "x-iohr-public": true, "x-iohr-scopes": ["identity:read"],
            "responses": { "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Me" } } } },
                           "default": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Problem" } } } } } } }
    });
    if with_radar {
        paths["/v1/radar/digests"] = serde_json::json!({ "get": { "operationId": "RadarService.ListDigests", "tags": ["radar"], "x-iohr-public": true, "x-iohr-scopes": ["radar:read"],
            "responses": { "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Me" } } } },
                           "default": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Problem" } } } } } } });
    }
    let mut doc = serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "InOrbit API", "version": "0.1.0" },
        "servers": [ { "url": "https://api.inorbit.hr" } ],
        "paths": paths,
        "components": { "schemas": {
            "Me": { "type": "object", "required": ["subject"], "properties": { "subject": { "type": "string" } } },
            "Problem": { "type": "object", "properties": { "code": { "$ref": "#/components/schemas/Code" }, "details": { "type": "array", "items": { "$ref": "#/components/schemas/Detail" } } } },
            "Code": { "type": "string", "enum": ["not_found","timeout"], "x-http-status": { "not_found": 404, "timeout": 504 } },
            "Detail": { "discriminator": { "propertyName": "type" }, "oneOf": [ { "type": "object", "required": ["type"], "properties": { "type": { "const": "retry" } } } ] }
        } }
    });
    let hash = format!(
        "sha256:{}",
        if with_radar {
            "aa".repeat(32)
        } else {
            "bb".repeat(32)
        }
    );
    doc["info"]["x-iohr-cut"] =
        serde_json::json!({ "plan": "free", "account": account, "scopes": scopes, "hash": hash });
    doc
}

#[tokio::test(flavor = "multi_thread")]
#[allow(
    clippy::too_many_lines,
    reason = "one scenario, generate then check then drift"
)]
async fn sdk_generate_writes_a_surface_and_a_lock_and_check_sees_drift() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read", "radar:read"]);
    me(&server, &t).await;
    // The token profile: no ?account= is sent for a token.
    Mock::given(method("GET"))
        .and(path("/v1/openapi.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cut_document(
            ACCOUNT,
            &["identity:read", "radar:read"],
            true,
        )))
        .expect(1)
        .mount(&server)
        .await;
    let o = r.with_stdin(
        &format!("{t}\n"),
        &[
            "login",
            "--with-token",
            "--insecure-storage",
            "--profile",
            "ci",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));

    let out_dir = dir.path().join("proj/src/iohr");
    let o = r.run(&[
        "sdk",
        "generate",
        "--lang",
        "rust",
        "--for",
        "ci",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(!text(&o).contains("TOKENSIGNATUREMARKER"));
    for f in ["mod.rs", "models.rs", "ops.rs", "profiles.rs", "surface.rs"] {
        assert!(out_dir.join(f).is_file(), "{f} missing");
    }
    let lock_path = dir.path().join("proj/src/iohr.lock");
    let lock = std::fs::read_to_string(&lock_path).unwrap();
    assert!(lock.contains("[profiles.ci]"), "{lock}");
    assert!(
        lock.contains(&format!("sha256:{}", "aa".repeat(32))),
        "{lock}"
    );
    assert!(lock.contains("\"GET /v1/radar/digests\""), "{lock}");
    assert!(!lock.contains("TOKENSIGNATUREMARKER"));
    let requests = server.received_requests().await.unwrap();
    let openapi: Vec<_> = requests
        .iter()
        .filter(|q| q.url.path() == "/v1/openapi.json")
        .collect();
    assert_eq!(openapi.len(), 1);
    assert!(
        openapi[0].url.query().is_none(),
        "a token profile sends no ?account="
    );

    // The same cut: check passes, with --files too.
    let o = r.run(&[
        "sdk",
        "check",
        "--lock",
        lock_path.to_str().unwrap(),
        "--files",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));

    // The API no longer grants radar:read: check fails and names the operation.
    server.reset().await;
    me(&server, &t).await;
    Mock::given(method("GET"))
        .and(path("/v1/openapi.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cut_document(
            ACCOUNT,
            &["identity:read"],
            false,
        )))
        .mount(&server)
        .await;
    let o = r.run(&["sdk", "check", "--lock", lock_path.to_str().unwrap()]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("- GET /v1/radar/digests"), "{}", text(&o));
    assert!(text(&o).contains("the cut moved"), "{}", text(&o));

    // Regenerating with --force takes the new cut; a non-empty --out without --force is refused.
    let o = r.run(&[
        "sdk",
        "generate",
        "--lang",
        "rust",
        "--for",
        "ci",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    let o = r.run(&[
        "sdk",
        "generate",
        "--lang",
        "rust",
        "--for",
        "ci",
        "--out",
        out_dir.to_str().unwrap(),
        "--force",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let o = r.run(&[
        "sdk",
        "check",
        "--lock",
        lock_path.to_str().unwrap(),
        "--files",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));

    // No lock: a usage error.
    assert_eq!(
        code(&r.run(&[
            "sdk",
            "check",
            "--lock",
            dir.path().join("nope.lock").to_str().unwrap()
        ])),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sdk_generate_from_files_needs_no_credential() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let a = dir.path().join("personal.json");
    let b = dir.path().join("ci.json");
    std::fs::write(
        &a,
        cut_document("acc_p", &["identity:read"], false).to_string(),
    )
    .unwrap();
    std::fs::write(
        &b,
        cut_document("acc_t", &["identity:read", "radar:read"], true).to_string(),
    )
    .unwrap();
    let out_dir = dir.path().join("gen/src/iohr");
    let o = r.run(&[
        "sdk",
        "generate",
        "--lang",
        "rust",
        "--from",
        &format!("personal={}", a.display()),
        "--from",
        &format!("acme-ci={}", b.display()),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let ops = std::fs::read_to_string(out_dir.join("ops.rs")).unwrap();
    assert!(ops.contains("impl ListDigests for super::profiles::AcmeCi {}"));
    assert!(!ops.contains("impl ListDigests for super::profiles::Personal {}"));
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "offline"
    );
    // Two names that would read the same variables are refused.
    let o = r.run(&[
        "sdk",
        "generate",
        "--lang",
        "rust",
        "--from",
        &format!("acme-ci={}", b.display()),
        "--from",
        &format!("acme_ci={}", b.display()),
        "--out",
        dir.path().join("x").to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("INORBIT_ACME_CI"), "{}", text(&o));
}

#[test]
fn a_language_iohr_does_not_know_is_a_usage_error_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("public.json");
    std::fs::write(
        &doc,
        cut_document("acc_p", &["identity:read"], false).to_string(),
    )
    .unwrap();
    let out_dir = dir.path().join("gen");
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_iohr"))
        .env_clear()
        .env("IOHR_CONFIG_DIR", dir.path())
        .args(["sdk", "generate", "--lang", "cobol", "--from"])
        .arg(format!("public={}", doc.display()))
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("cobol"), "{}", text(&o));
    assert!(
        !out_dir.exists(),
        "nothing is written for a language iohr does not know"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_profile_sends_its_account_and_refuses_a_document_for_another() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    mock_provider(&server).await;
    let uri = server.uri();
    // A person profile written as `iohr login` leaves it, with a refresh token on file.
    std::fs::create_dir_all(dir.path().join("secrets")).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!("default = \"me\"\n[profiles.me]\nkind = \"person\"\naccount = \"acc_team1\"\nstorage = \"file\"\nissuer = \"{uri}\"\nclient_id = \"iohr-cli\"\n"),
    )
    .unwrap();
    let tokens = person_tokens(&uri);
    std::fs::write(
        dir.path().join("secrets/me.acc_team1"),
        serde_json::json!({ "access_token": tokens["access_token"], "refresh_token": "REFRESHMARKER", "expires_at": 4_102_444_800_i64 }).to_string(),
    )
    .unwrap();
    // The gateway answers the personal plan whatever is asked: the stamp names acc_test1.
    Mock::given(method("GET"))
        .and(path("/v1/openapi.json"))
        .and(query_param("account", "acc_team1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cut_document(ACCOUNT, &[], true)))
        .mount(&server)
        .await;
    let o = r.run(&[
        "sdk",
        "generate",
        "--lang",
        "rust",
        "--for",
        "me",
        "--out",
        dir.path().join("o").to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("acc_team1") && text(&o).contains("does not cut by account yet"),
        "{}",
        text(&o)
    );
    assert!(!text(&o).contains("TOKENSIGNATUREMARKER"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profile_token_from_the_environment_stands_in_for_a_profile_in_ci() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read"]);
    me(&server, &t).await;
    Mock::given(method("GET"))
        .and(path("/v1/openapi.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cut_document(
            ACCOUNT,
            &["identity:read"],
            false,
        )))
        .mount(&server)
        .await;
    // A lock written elsewhere names profile acme-ci; this machine has no such profile.
    std::fs::write(
        dir.path().join("iohr.lock"),
        format!("generator = \"0\"\nlang = \"rust\"\nout = \"src/iohr\"\n[profiles.acme-ci]\ncut = \"sha256:{}\"\nplan = \"free\"\nscopes = [\"identity:read\"]\noperations = [\"GET /v1/me\"]\n", "bb".repeat(32)),
    )
    .unwrap();
    let lock = dir.path().join("iohr.lock");
    let o = r.run(&["sdk", "check", "--lock", lock.to_str().unwrap()]);
    assert_eq!(code(&o), 3, "no profile and no token: {}", text(&o));
    let o = r
        .cmd(&["sdk", "check", "--lock", lock.to_str().unwrap()])
        .env("IOHR_TOKEN_ACME_CI", &t)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(!text(&o).contains("TOKENSIGNATUREMARKER"));
    assert!(!dir.path().join("secrets").exists(), "nothing written");
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_account_points_a_person_at_one_of_their_teams() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    mock_provider(&server).await;
    let uri = server.uri();
    std::fs::create_dir_all(dir.path().join("secrets")).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!("default = \"me\"\n[profiles.me]\nkind = \"person\"\naccount = \"{ACCOUNT}\"\nstorage = \"file\"\nissuer = \"{uri}\"\nclient_id = \"iohr-cli\"\n"),
    )
    .unwrap();
    let tokens = person_tokens(&uri);
    std::fs::write(
        dir.path().join(format!("secrets/me.{ACCOUNT}")),
        serde_json::json!({ "access_token": tokens["access_token"], "refresh_token": "REFRESHMARKER", "expires_at": 4_102_444_800_i64 }).to_string(),
    )
    .unwrap();
    Mock::given(method("GET"))
        .and(path("/v1/accounts/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "account": { "id": ACCOUNT, "kind": "personal", "name": "Me", "plan": "free", "slug": "" },
            "teams": [ { "id": "acc_team1", "kind": "team", "name": "Acme", "plan": "pro", "slug": "acme" } ]
        })))
        .mount(&server)
        .await;
    let o = r.run(&["profile", "account", "me", "acme"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("acc_team1") && text(&o).contains("pro"),
        "{}",
        text(&o)
    );
    let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(config.contains("account = \"acc_team1\""), "{config}");
    assert!(
        dir.path().join("secrets/me.acc_team1").is_file(),
        "the credential moved with the account"
    );
    assert!(!dir.path().join(format!("secrets/me.{ACCOUNT}")).exists());
    let o = r.run(&["profile", "account", "me", "acc_nobody"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("acme"), "{}", text(&o));
    assert!(!text(&o).contains("TOKENSIGNATUREMARKER"));
}

// --- connectors and connections (RFC 0044) ---------------------------------------------

/// A connection's secret, as a marker easy to find in any output.
const SECRET: &str = "CONNECTIONSECRETMARKER";

fn incident_io() -> serde_json::Value {
    serde_json::json!({
        "id": "incident-io", "name": "incident.io", "category": "incident",
        "status": "available", "hosts": ["api.incident.io"],
        "auth_modes": [{"mode": "api_key", "label": "API key",
            "fields": [{"name": "api_key", "label": "API key", "secret": true}]}],
        "config_fields": [],
        "actions": [{"name": "create_incident", "class": "write_reversible",
            "model": "incident.create", "params": [{"name": "title", "required": true}]}]
    })
}

fn slack() -> serde_json::Value {
    serde_json::json!({
        "id": "slack", "name": "Slack", "category": "chat", "status": "available",
        "hosts": ["slack.com"],
        "auth_modes": [{"mode": "oauth2", "label": "Sign in with Slack", "fields": [],
            "scopes": ["chat:write", "channels:read"]}],
        "actions": []
    })
}

fn connection_json() -> serde_json::Value {
    serde_json::json!({
        "id": "con_1", "name": "incident-io", "kind": "incident-io",
        "connector": "incident-io", "auth_mode": "api_key",
        "label": "https://app.incident.io/acme", "status": "active", "grants": 1,
        "credential": {"kind": "sealed", "set_at": "2026-10-04T09:00:00Z"},
        "last_test_at": "2026-10-04T09:00:00Z", "last_test_ok": true,
        "created_at": "2026-10-04T09:00:00Z", "owner": "Ana"
    })
}

fn connections_base() -> String {
    format!("/v1/accounts/orgs/{ACCOUNT}/connections")
}

async fn catalogue(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/connectors"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "connectors": [incident_io(), slack()], "next_page_token": ""
        })))
        .mount(server)
        .await;
    for c in [incident_io(), slack()] {
        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/connectors/{}",
                c["id"].as_str().unwrap()
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "connector": c })),
            )
            .mount(server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(connections_base()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "connections": [connection_json()], "next_page_token": ""
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/con_1", connections_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "connection": connection_json()
        })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connectors_and_connections_list_and_show() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["connections:read"]);
    catalogue(&server).await;

    let o = r.with_token(&t, &["connectors", "list"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        stdout.contains("incident-io") && stdout.contains("slack"),
        "{stdout}"
    );
    let o = r.with_token(&t, &["connectors", "list", "--category", "chat", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v.as_array().map(Vec::len), Some(1), "{v}");
    assert_eq!(v[0]["id"], "slack");

    let o = r.with_token(&t, &["connectors", "show", "incident-io"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        stdout.contains("api_key (secret)") && stdout.contains("create_incident"),
        "{stdout}"
    );
    let o = r.with_token(&t, &["connectors", "show", "Not/AnId"]);
    assert_eq!(code(&o), 2, "{}", text(&o));

    let o = r.with_token(&t, &["connections", "list"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("https://app.incident.io/acme"),
        "{}",
        text(&o)
    );
    let o = r.with_token(&t, &["connections", "list", "--status", "paused", "--json"]);
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "[]");
    let o = r.with_token(&t, &["connections", "show", "incident-io", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["id"], "con_1");
    let o = r.with_token(&t, &["connections", "show", "nope"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("iohr connections list"), "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_add_reads_a_key_from_stdin_and_never_shows_it() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["connections:read", "connections:write"]);
    catalogue(&server).await;
    Mock::given(method("POST"))
        .and(path(connections_base()))
        .and(body_json(serde_json::json!({
            "kind": "incident-io", "auth_mode": "api_key", "name": "incidents",
            "config": {}, "credentials": {"api_key": SECRET}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "connection": connection_json()
        })))
        .mount(&server)
        .await;

    let args = [
        "connections",
        "add",
        "incident-io",
        "--name",
        "incidents",
        "--secret-stdin",
        "api_key",
    ];
    let o = r.with_token_and_stdin(&t, &format!("{SECRET}\n"), &args);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("Connected incident-io as https://app.incident.io/acme on incident.io"),
        "{}",
        text(&o)
    );
    assert!(!text(&o).contains(SECRET));
    let mut json_args = args.to_vec();
    json_args.push("--json");
    let o = r.with_token_and_stdin(&t, SECRET, &json_args);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["name"], "incident-io");
    assert!(!text(&o).contains(SECRET));

    // The secret travelled only in the body of the create call: never in a path, a
    // query or a header.
    let requests = server.received_requests().await.unwrap();
    let mut bodies = 0;
    for req in &requests {
        assert!(!req.url.as_str().contains(SECRET), "{}", req.url);
        for (_, v) in &req.headers {
            assert!(!v.to_str().unwrap_or_default().contains(SECRET));
        }
        if String::from_utf8_lossy(&req.body).contains(SECRET) {
            assert_eq!(req.method.as_str(), "POST");
            assert_eq!(req.url.path(), connections_base());
            bodies += 1;
        }
    }
    assert_eq!(bodies, 2);

    // Never as an argument, and an unknown field is named, not echoed.
    let o = r.with_token(
        &t,
        &[
            "connections",
            "add",
            "incident-io",
            "--config",
            &format!("api_key={SECRET}"),
        ],
    );
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(!text(&o).contains(SECRET), "{}", text(&o));
    let o = r.with_token(&t, &["connections", "add", "incident-io"]);
    assert_eq!(code(&o), 2, "no terminal to ask on: {}", text(&o));
    assert!(text(&o).contains("--secret-stdin"), "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_key_is_a_failed_call_and_nothing_is_stored() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["connections:write"]);
    catalogue(&server).await;
    Mock::given(method("POST"))
        .and(path(connections_base()))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "code": "bad_request",
            "error": "the key was refused: auth (the provider answered 401)"
        })))
        .mount(&server)
        .await;
    let o = r.with_token_and_stdin(
        &t,
        SECRET,
        &[
            "connections",
            "add",
            "incident-io",
            "--secret-stdin",
            "api_key",
        ],
    );
    assert_eq!(code(&o), 1, "{}", text(&o));
    let all = text(&o);
    assert!(
        all.contains("the key was refused") && all.contains("Nothing was stored"),
        "{all}"
    );
    assert!(!all.contains(SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_add_signs_in_at_the_provider_and_waits_on_the_session() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["connections:read", "connections:write"]);
    catalogue(&server).await;
    let expires = (time::OffsetDateTime::now_utc() + time::Duration::minutes(10))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    Mock::given(method("POST"))
        .and(path(format!("{}/connect", connections_base())))
        .and(body_json(serde_json::json!({
            "connector": "slack", "auth_mode": "oauth2", "config": {}, "name": "chat"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "authorize_url": "https://slack.com/oauth/v2/authorize?client_id=1&state=STATE1",
            "expires_at": expires, "session_id": "cs_1"
        })))
        .mount(&server)
        .await;
    let session = format!("{}/connect/cs_1", connections_base());
    Mock::given(method("GET"))
        .and(path(&session))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "pending"})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let mut slack_connection = connection_json();
    slack_connection["name"] = "chat".into();
    slack_connection["connector"] = "slack".into();
    slack_connection["label"] = "Acme workspace".into();
    Mock::given(method("GET"))
        .and(path(&session))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "completed", "connection": slack_connection
        })))
        .mount(&server)
        .await;

    let started = std::time::Instant::now();
    let o = r.with_token(&t, &["connections", "add", "slack", "--name", "chat"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let all = text(&o);
    assert!(
        all.contains("https://slack.com/oauth/v2/authorize?client_id=1&state=STATE1"),
        "no browser here, so the link is printed: {all}"
    );
    assert!(
        all.contains("Connected chat as Acme workspace on Slack"),
        "{all}"
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(2),
        "polled twice"
    );
    let polls = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|q| q.url.path() == session)
        .count();
    assert_eq!(polls, 2);

    // A key flag with a mode that signs in is a usage error.
    let o = r.with_token(&t, &["connections", "add", "slack", "--secret-stdin", "x"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_sign_in_says_why() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["connections:write"]);
    catalogue(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("{}/connect", connections_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "authorize_url": "https://slack.com/oauth/v2/authorize?state=S",
            "session_id": "cs_2"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/connect/cs_2", connections_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "failed", "error": "access_denied: the person declined"
        })))
        .mount(&server)
        .await;
    let o = r.with_token(&t, &["--json", "connections", "add", "slack"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("the person declined"), "{}", text(&o));
    assert!(o.stdout.is_empty(), "no result on stdout");
}

#[tokio::test(flavor = "multi_thread")]
#[allow(
    clippy::too_many_lines,
    reason = "one scenario, grant then list then revoke"
)]
async fn connections_grant_list_and_revoke() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["connections:read", "connections:write"]);
    catalogue(&server).await;
    let grants = format!("{}/con_1/grants", connections_base());
    let grant = serde_json::json!({
        "id": "gnt_1", "connection_id": "con_1",
        "consumer": {"kind": "product", "id": "reliability"},
        "actions": ["create_incident"], "expires_at": "2027-01-02T10:00:00Z", "status": "active"
    });
    Mock::given(method("POST"))
        .and(path(&grants))
        .and(body_json(serde_json::json!({
            "consumer": {"kind": "product", "id": "reliability"},
            "actions": ["create_incident"]
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "grant": grant })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(&grants))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({
            "consumer": {"kind": "agent", "id": "triage"},
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "grant": grant })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(&grants))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "grants": [grant], "next_page_token": ""
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{grants}/gnt_1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "grant": {"id": "gnt_1", "revoked_at": "2026-10-04T10:00:00Z", "status": "revoked"}
        })))
        .mount(&server)
        .await;

    let o = r.with_token(
        &t,
        &[
            "connections",
            "grant",
            "incident-io",
            "--to",
            "product:reliability",
            "--actions",
            "create_incident",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(String::from_utf8_lossy(&o.stdout), "gnt_1\n");
    let o = r.with_token(
        &t,
        &[
            "connections",
            "grant",
            "con_1",
            "--to",
            "agent:triage",
            "--actions",
            "create_incident,list_statuses",
            "--expires",
            "30d",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["id"], "gnt_1");
    let sent = server.received_requests().await.unwrap();
    let body: serde_json::Value = sent
        .iter()
        .rev()
        .find(|q| q.method.as_str() == "POST")
        .map(|q| serde_json::from_slice(&q.body).unwrap())
        .unwrap();
    assert_eq!(
        body["actions"],
        serde_json::json!(["create_incident", "list_statuses"])
    );
    assert!(
        body["expires_at"].as_str().unwrap().ends_with('Z'),
        "{body}"
    );

    let o = r.with_token(&t, &["connections", "grants", "incident-io"]);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("product:reliability"),
        "{}",
        text(&o)
    );
    let o = r.with_token(&t, &["connections", "grants", "con_1", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v[0]["id"], "gnt_1");
    let o = r.with_token(
        &t,
        &[
            "connections",
            "revoke-grant",
            "incident-io",
            "gnt_1",
            "--json",
        ],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["status"], "revoked");

    for bad in [
        vec![
            "connections",
            "grant",
            "con_1",
            "--to",
            "person:x",
            "--actions",
            "a",
        ],
        vec![
            "connections",
            "grant",
            "con_1",
            "--to",
            "key:k",
            "--actions",
            "a",
            "--expires",
            "soon",
        ],
    ] {
        let o = r.with_token(&t, &bad);
        assert_eq!(code(&o), 2, "{bad:?}: {}", text(&o));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_delete_asks_first_unless_yes() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["connections:read", "connections:write"]);
    catalogue(&server).await;
    Mock::given(method("DELETE"))
        .and(path(format!("{}/con_1", connections_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let deletes = || async {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|q| q.method.as_str() == "DELETE")
            .count()
    };

    let o = r.with_token(&t, &["connections", "delete", "incident-io"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("--yes"), "{}", text(&o));
    assert_eq!(deletes().await, 0, "nothing deleted without a confirmation");

    let o = r.with_token(
        &t,
        &["connections", "delete", "incident-io", "--yes", "--json"],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"id": "con_1", "name": "incident-io", "deleted": true})
    );
    assert_eq!(deletes().await, 1);
}

/// A token for `account` with `scopes` that expires at `exp` (Unix seconds).
fn token_until(scopes: &[&str], exp: i64) -> String {
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": exp,
            "scp": scopes, "org": ACCOUNT, "plan": "free", "key": "key_1"
        }))
    )
}

/// Regression: `IOHR_TOKEN` was ignored once a default profile existed, though the
/// README and ADR 0009 say it is used. A profile named by `--profile` still wins.
#[tokio::test(flavor = "multi_thread")]
async fn iohr_token_wins_over_the_default_profile() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let stored = token(&["identity:read"]);
    let from_env = token(&["identity:read", "radar:read"]);
    me(&server, &stored).await;
    let o = r.with_stdin(
        &format!("{stored}\n"),
        &["login", "--with-token", "--insecure-storage", "--profile", "ci"],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", format!("Bearer {from_env}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "subject": "ak_from_env", "kind": "client", "org": ACCOUNT, "scopes": ["identity:read"]
        })))
        .mount(&server)
        .await;

    let o = r.with_token(&from_env, &["whoami", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["profile"], "IOHR_TOKEN", "{v}");
    assert_eq!(v["me"]["subject"], "ak_from_env", "{v}");

    let o = r.with_token(&from_env, &["whoami", "--json", "--profile", "ci"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["profile"], "ci", "{v}");
    assert_eq!(v["me"]["subject"], "ak_tok1", "{v}");
}

/// `iohr auth token`: the SDKs' `cli` credential source (docs/config.md section 5.4).
#[tokio::test(flavor = "multi_thread")]
async fn auth_token_prints_the_profiles_token_and_its_expiry() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read"]);
    me(&server, &t).await;
    let o = r.with_stdin(
        &format!("{t}\n"),
        &["login", "--with-token", "--insecure-storage", "--profile", "ci"],
    );
    assert_eq!(code(&o), 0, "{}", text(&o));

    let o = r.run(&["auth", "token", "--profile", "ci", "--format", "json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(o.stderr.is_empty(), "{}", text(&o));
    let stdout = String::from_utf8(o.stdout.clone()).unwrap();
    assert_eq!(stdout.lines().count(), 1, "one line: {stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["access_token"], t.as_str());
    assert_eq!(v["expires_at"], "2100-01-01T00:00:00Z");
    assert_eq!(v["profile"], "ci");
    assert_eq!(v["account"], ACCOUNT);
    assert_eq!(v.as_object().unwrap().len(), 4, "{v}");

    // Text: the token alone; the default profile when none is named; --json is json.
    let o = r.run(&["auth", "token"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(String::from_utf8(o.stdout).unwrap(), format!("{t}\n"));
    let o = r.run(&["auth", "token", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["profile"], "ci");

    // IOHR_TOKEN: no profile.
    let o = r.with_token(&t, &["auth", "token", "--format", "json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["profile"], serde_json::Value::Null);

    // Not signed in: exit 3, nothing on stdout, one line on stderr.
    for args in [
        vec!["auth", "token", "--profile", "missing", "--format", "json"],
        vec!["auth", "token", "--profile", "BAD NAME"],
    ] {
        let o = r.run(&args);
        let want = if args.contains(&"BAD NAME") { 2 } else { 3 };
        assert_eq!(code(&o), want, "{args:?}: {}", text(&o));
        assert!(o.stdout.is_empty(), "{args:?}");
    }
    let expired = token_until(&["identity:read"], 1_000_000_000);
    let o = r.with_token(&expired, &["auth", "token", "--format", "json"]);
    assert_eq!(code(&o), 3, "{}", text(&o));
    assert!(o.stdout.is_empty() && !text(&o).contains("TOKENSIGNATUREMARKER"));
}

/// A signed-in person whose access token has run out: `iohr auth token` refreshes,
/// stores the rotated refresh token and prints only the new access token.
#[tokio::test(flavor = "multi_thread")]
async fn auth_token_refreshes_a_person_and_never_prints_the_refresh_token() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    mock_provider(&server).await;
    let uri = server.uri();
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(wiremock::matchers::body_string_contains("grant_type=refresh_token"))
        .and(wiremock::matchers::body_string_contains("refresh_token=OLDREFRESH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(person_tokens(&uri)))
        .expect(1)
        .mount(&server)
        .await;
    std::fs::write(
        dir.path().join("config.toml"),
        format!(
            "default = \"me\"\n\n[sdk]\nlog = \"warn\"\n\n[profiles.me]\nkind = \"person\"\naccount = \"{ACCOUNT}\"\nstorage = \"file\"\nissuer = \"{uri}\"\nclient_id = \"iohr-cli\"\ntimeout = \"10s\"\n"
        ),
    )
    .unwrap();
    let old_access = token_until(&["iohr.api"], 1_000_000_000);
    std::fs::create_dir_all(dir.path().join("secrets")).unwrap();
    std::fs::write(
        dir.path().join(format!("secrets/me.{ACCOUNT}")),
        serde_json::json!({"refresh_token": "OLDREFRESH", "access_token": old_access, "expires_at": 1_000_000_000}).to_string(),
    )
    .unwrap();

    let o = r.run(&["auth", "token", "--profile", "me", "--format", "json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let all = text(&o);
    assert!(!all.contains("REFRESHMARKER") && !all.contains("OLDREFRESH"), "{all}");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let want = person_tokens(&uri)["access_token"].clone();
    assert_eq!(v["access_token"], want);
    assert_eq!(v["profile"], "me");
    let stored = std::fs::read_to_string(dir.path().join(format!("secrets/me.{ACCOUNT}"))).unwrap();
    assert!(stored.contains("REFRESHMARKER"), "the rotated refresh token is stored");
    // The run read the config and did not rewrite it; the SDK keys are there.
    let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(config.contains("[sdk]") && config.contains("timeout = \"10s\""));
}

/// The commands that write `config.toml` keep what the SDKs keep there (docs/config.md
/// section 4.2): the `[sdk]` table, SDK keys in a profile, SDK-only profiles, comments.
#[tokio::test(flavor = "multi_thread")]
async fn commands_that_write_the_config_keep_sdk_keys_and_comments() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let r = Run {
        server: &server,
        config: dir.path(),
    };
    let t = token(&["identity:read"]);
    me(&server, &t).await;
    let file = dir.path().join("config.toml");
    std::fs::write(
        &file,
        "# Shared with the SDKs; keep this.\n\n[sdk]                # every profile\nlog = \"warn\"\nproxy = \"http://proxy.corp.example:3128\"\n\n[profiles.ci]        # SDK only\nkey_id = \"ak_7f3c\"\nkey_secret_file = \"/run/secrets/inorbit-ci\"\nscopes = [\"identity:read\"]\n\n[profiles.work]\ntimeout = \"10s\"      # SDK key before login\n",
    )
    .unwrap();
    let check = |step: &str| {
        let text = std::fs::read_to_string(&file).unwrap();
        for kept in [
            "# Shared with the SDKs; keep this.",
            "# every profile",
            "log = \"warn\"",
            "proxy = \"http://proxy.corp.example:3128\"",
            "# SDK only",
            "key_id = \"ak_7f3c\"",
            "key_secret_file = \"/run/secrets/inorbit-ci\"",
            "timeout = \"10s\"      # SDK key before login",
        ] {
            assert!(text.contains(kept), "{step} dropped {kept}:\n{text}");
        }
        text
    };
    // An SDK-only profile is not one of the command line's.
    let o = r.run(&["profile", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["profiles"].as_object().unwrap().len(), 0, "{v}");

    for (step, args) in [
        ("login", vec!["login", "--with-token", "--insecure-storage", "--profile", "work"]),
        ("login a second", vec!["login", "--with-token", "--insecure-storage", "--profile", "other"]),
    ] {
        let o = r.with_stdin(&format!("{t}\n"), &args);
        assert_eq!(code(&o), 0, "{step}: {}", text(&o));
        check(step);
    }
    let text_now = check("login");
    assert!(text_now.contains("kind = \"token\""), "{text_now}");
    for (step, args) in [
        ("profile use", vec!["profile", "use", "other"]),
        ("config set", vec!["config", "set", "ext.registry", "registry.acme.hr/inorbit/iohr-ext"]),
        ("config unset", vec!["config", "unset", "ext.registry"]),
        ("logout", vec!["logout", "--profile", "work"]),
    ] {
        let o = r.run(&args);
        assert_eq!(code(&o), 0, "{step}: {}", text(&o));
        check(step);
    }
    let text_now = check("end");
    let t: toml::Table = text_now.parse().unwrap();
    assert_eq!(t["default"].as_str(), Some("other"));
    // logout took the command line's keys from `work` and left the SDK's.
    assert_eq!(t["profiles"]["work"].as_table().unwrap().len(), 1, "{text_now}");
}
