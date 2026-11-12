use super::*;
use crate::runner::host_action::testing::{Fake, fake};
use crate::runner::host_action::{HostRunner, KubeTarget, Output};
use serde_json::json;

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn restart() -> HostAction {
    serde_json::from_value(json!({
        "id": "example.restart-worker",
        "params": {
            "worker": {"type": "dns-label"},
            "grace_secs": {"type": "integer", "min": 0, "max": 600},
        },
        "os": ["linux", "macos"],
        "effect": "Restarts worker {worker} after {grace_secs} seconds.",
        "timeout_secs": 120,
    }))
    .unwrap()
}

fn installed() -> Installed {
    Installed {
        version: "1.2.0".to_owned(),
        digests: vec!["sha256:index".to_owned(), DIGEST.to_owned()],
        actions: vec![restart()],
        program: PathBuf::from("/ext/example/bin"),
    }
}

/// A host with `example` 1.2.0 installed, declaring `example.restart-worker`.
fn host() -> Fake {
    let mut k = fake("", false);
    k.extensions = vec![("example".to_owned(), installed())];
    k
}

fn doc() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/host-action/v1/examples/extension.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn with(mut d: Value, pointer: &str, v: Value) -> Value {
    *d.pointer_mut(pointer).unwrap() = v;
    d
}

fn not_here(r: Result<Routed, NotRouted>) -> String {
    match r {
        Err(NotRouted::NotHere(why)) => why,
        other => panic!("not refused here: {other:?}"),
    }
}

fn schema(r: Result<Routed, NotRouted>) -> String {
    match r {
        Err(NotRouted::Schema(why)) => why,
        other => panic!("not a schema refusal: {other:?}"),
    }
}

#[test]
fn a_declared_action_routes_to_its_program_with_the_parameters_as_json() {
    let Ok(Routed::Extension(call)) = route_on(&doc(), &host(), "linux") else {
        panic!("not routed to the extension");
    };
    assert_eq!(call.id, "example.restart-worker");
    assert_eq!(call.program, PathBuf::from("/ext/example/bin"));
    assert_eq!(call.limit, Duration::from_secs(120));
    let input: Value = serde_json::from_slice(&call.input).unwrap();
    assert_eq!(input, json!({"worker": "build-1", "grace_secs": 30}));
}

#[test]
fn a_builtin_or_v0_document_routes_to_the_catalog() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/host-action/schema/examples/cancel-run.json");
    let v0: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let r = route_on(&v0, &host(), "linux").unwrap();
    assert!(matches!(r, Routed::Builtin(_)), "{r:?}");
    assert_eq!(r.id(), "gh.cancel-run");
    let mut k = host();
    k.not_here = Some("no gh here".to_owned());
    assert_eq!(not_here(route_on(&v0, &k, "linux")), "no gh here");
}

#[test]
fn an_extension_action_is_refused_unless_the_installed_copy_is_the_approved_one() {
    let k = host();
    let why = not_here(route_on(
        &with(doc(), "/provider/extension", json!("other")),
        &k,
        "linux",
    ));
    assert_eq!(why, "other is not installed");
    let why = not_here(route_on(
        &with(doc(), "/provider/version", json!("1.3.0")),
        &k,
        "linux",
    ));
    assert_eq!(why, "example 1.3.0 was approved, but 1.2.0 is installed");
    let other = format!("sha256:{}", "f".repeat(64));
    let why = not_here(route_on(
        &with(doc(), "/provider/manifest_digest", json!(other)),
        &k,
        "linux",
    ));
    assert!(why.contains("not the one approved"), "{why}");
    let why = not_here(route_on(
        &with(doc(), "/action", json!("example.stop-worker")),
        &k,
        "linux",
    ));
    assert_eq!(why, "example 1.2.0 does not declare example.stop-worker");
    let why = not_here(route_on(&doc(), &k, "windows"));
    assert_eq!(why, "example.restart-worker does not run on windows");
    // The runner that carries out no extension's actions says so.
    let why = not_here(route_on(&doc(), &fake("", false), "linux"));
    assert!(why.contains("not installed"), "{why}");
    assert!(k.ext_calls.lock().unwrap().is_empty());
}

#[test]
fn parameters_and_effect_must_be_what_the_declaration_says() {
    let k = host();
    let why = schema(route_on(
        &with(doc(), "/params/worker", json!("Build_1")),
        &k,
        "linux",
    ));
    assert_eq!(why, "/params/worker: not a DNS label");
    assert!(!why.contains("Build_1"));
    let why = schema(route_on(
        &with(doc(), "/params", json!({"worker": "build-1"})),
        &k,
        "linux",
    ));
    assert_eq!(why, "/params/grace_secs: required");
    let why = schema(route_on(
        &with(doc(), "/params/grace_secs", json!(601)),
        &k,
        "linux",
    ));
    assert!(why.starts_with("/params/grace_secs:"), "{why}");
    let why = schema(route_on(
        &with(doc(), "/effect", json!("Restarts worker build-1 now.")),
        &k,
        "linux",
    ));
    assert!(why.starts_with("/effect:"), "{why}");
}

#[tokio::test]
async fn carrying_out_a_call_reports_what_the_program_said() {
    let Ok(Routed::Extension(call)) = route_on(&doc(), &host(), "linux") else {
        panic!("not routed");
    };
    let mut k = host();
    k.answer = Some(Output::new(true, "restarted\n", ""));
    assert_eq!(
        carry_out(&call, &k).await,
        Ok("example.restart-worker: done\nrestarted".to_owned())
    );
    let calls = k.ext_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "example.restart-worker");
    assert_eq!(calls[0].2, call.input);
    let k = fake("", true);
    let why = carry_out(&call, &k).await.unwrap_err();
    assert!(why.starts_with("example.restart-worker: failed"), "{why}");
}

/// A host whose environment holds `IOHR_TOKEN`, to show the extension never gets it.
#[cfg(unix)]
fn token_host() -> HostRunner {
    HostRunner {
        path: None,
        env: vec![
            ("IOHR_TOKEN".into(), "t0ken".into()),
            ("IOHR_TOKEN_CI".into(), "t0ken".into()),
            ("KUBECONFIG".into(), "/k".into()),
            ("LANG".into(), "C".into()),
        ],
        kube: KubeTarget::new(None, None),
        iohr: None,
        install_env: Vec::new(),
        user_units: Vec::new(),
        core_checkout: None,
        ssh_env: Vec::new(),
        account: None,
        extensions: None,
    }
}

#[cfg(unix)]
fn program(dir: &std::path::Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let p = dir.join("ext");
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[cfg(unix)]
#[tokio::test]
async fn the_program_gets_its_action_and_parameters_and_no_token() {
    let dir = tempfile::tempdir().unwrap();
    let p = program(
        dir.path(),
        "echo \"args: $*\"; /bin/cat; echo; /usr/bin/env | /usr/bin/sort",
    );
    let out = token_host()
        .run_extension(
            &p,
            "example.restart-worker",
            br#"{"worker":"build-1"}"#,
            Duration::from_secs(20),
        )
        .await
        .unwrap();
    assert!(out.success, "{}", out.stderr);
    assert!(
        out.stdout
            .contains("args: host-action example.restart-worker"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(r#"{"worker":"build-1"}"#),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("LANG=C"), "{}", out.stdout);
    assert!(!out.stdout.contains("IOHR_TOKEN"), "{}", out.stdout);
    assert!(!out.stdout.contains("KUBECONFIG"), "{}", out.stdout);
}

#[cfg(unix)]
#[tokio::test]
async fn a_program_that_outlives_its_limit_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let p = program(dir.path(), "/bin/sleep 30");
    let why = token_host()
        .run_extension(&p, "example.slow", b"{}", Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(why, "example.slow took over 1s");
}
