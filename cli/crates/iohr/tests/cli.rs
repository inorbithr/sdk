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
use wiremock::matchers::{body_json, header, method, path, query_param};
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

    fn with_stdin(&self, input: &str, args: &[&str]) -> Output {
        let mut child = self
            .cmd(args)
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
            "token": {"id": "k"}, "access_token": "NEWTOKENVALUE", "expires_at": ""
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
    let commands: Vec<Vec<&str>> = vec![
        vec!["whoami"],
        vec!["accounts", "list"],
        vec!["token", "list"],
        vec!["token", "revoke", "key_1"],
        vec!["token", "create", "--name", "n", "--scope", "radar:read"],
        vec!["api", "GET", "/v1/me", "-f", "a=b"],
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
    ];
    for args in commands {
        let mut args = args.clone();
        args.push("--verbose");
        let o = r.with_token(&t, &args);
        assert_eq!(code(&o), 0, "{args:?}: {}", text(&o));
        let all = text(&o);
        assert!(
            !all.contains("TOKENSIGNATUREMARKER"),
            "{args:?} leaked the token"
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
