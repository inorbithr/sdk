//! Targeting an environment (RFC 0148, Keel K07): `--env` and `IOHR_ENV` choose it,
//! production is the default, an unknown name is refused, and `local` is on loopback.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::process::{Command, Output, Stdio};

use clap::Parser as _;
use inorbithr::DEFAULT_BASE_URL;
use iohr::cli::Cli;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A run with only what a process needs to start, plus `extra`.
fn run(args: &[&str], extra: &[(&str, &str)]) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
    c.args(args).env_clear().env("IOHR_CONFIG_DIR", dir.path());
    for k in ["PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    c.envs(extra.iter().copied());
    c.stdin(Stdio::null()).output().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn parse(args: &[&str]) -> Cli {
    let mut v = vec!["iohr"];
    v.extend_from_slice(args);
    v.push("whoami");
    Cli::try_parse_from(v).unwrap()
}

#[test]
fn production_is_the_default_and_uses_the_built_in_address() {
    // Ambient IOHR_ENV / IOHR_BASE_URL would change the answer; the tests run in CI
    // without them.
    if std::env::var_os("IOHR_ENV").is_some() || std::env::var_os("IOHR_BASE_URL").is_some() {
        return;
    }
    let g = parse(&[]).global;
    assert_eq!(g.base_url(), DEFAULT_BASE_URL);
    assert_eq!(g.issuer(), iohr_auth::DEFAULT_ISSUER);
    assert!(g.check_environment().is_ok());
    let g = parse(&["--env", "production"]).global;
    assert_eq!(g.base_url(), DEFAULT_BASE_URL);
}

#[test]
fn the_env_flag_selects_local_on_loopback_and_never_a_production_host() {
    let g = parse(&["--env", "local"]).global;
    assert_eq!(g.base_url(), "http://localhost:28080");
    assert_eq!(g.issuer(), "http://auth.localhost:28080");
    assert!(g.check_environment().is_ok());
}

#[test]
fn an_explicit_base_url_wins_over_the_environment() {
    let g = parse(&["--env", "local", "--base-url", "http://127.0.0.1:9"]).global;
    assert_eq!(g.base_url(), "http://127.0.0.1:9");
    assert!(g.check_environment().is_ok());
}

#[test]
fn an_unknown_environment_is_refused_from_the_flag_and_from_the_variable() {
    let o = run(&["--env", "staging", "whoami"], &[]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("unknown environment 'staging'"),
        "{}",
        stderr(&o)
    );
    let o = run(&["whoami"], &[("IOHR_ENV", "staging")]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("unknown environment 'staging'"),
        "{}",
        stderr(&o)
    );
}

#[tokio::test]
async fn local_calls_the_address_it_is_given() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let uri = server.uri();
    let args = ["api", "GET", "/v1/ping"];
    let o = tokio::task::spawn_blocking(move || {
        run(
            &args,
            &[
                ("IOHR_ENV", "local"),
                ("IOHR_BASE_URL", &uri),
                ("IOHR_TOKEN", &token()),
            ],
        )
    })
    .await
    .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    server.verify().await;
}

fn token() -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.SIG",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(
            serde_json::json!({"sub": "ak_t", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": ["identity:read"], "org": "acc_t", "plan": "free", "key": "k"})
        )
    )
}
