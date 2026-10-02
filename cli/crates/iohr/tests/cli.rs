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
            "code": "permission_denied", "error": "this token may not read the account"
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
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "subject": "ak_tok1", "account": {"id": ACCOUNT}, "teams": [], "keys": [], "paths": {},
            "token": {"id": "k"}, "access_token": "NEWTOKENVALUE", "expires_at": ""
        })))
        .mount(&server)
        .await;
    let out_file = dir.path().join("doc.json");
    let out_file = out_file.to_str().unwrap();
    let commands: Vec<Vec<&str>> = vec![
        vec!["whoami"],
        vec!["accounts", "list"],
        vec!["token", "list"],
        vec!["token", "revoke", "key_1"],
        vec!["token", "create", "--name", "n", "--scope", "radar:read"],
        vec!["api", "GET", "/v1/me", "-f", "a=b"],
        vec!["openapi", "pull", "-o", out_file],
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
