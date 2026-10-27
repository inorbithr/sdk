//! Host actions (platform RFC 0172 slice 3, `schemas/runner/host-action/v0.json`): the
//! few things an approved pinned run may make this runner do on its own host, each a
//! native function with typed parameters, never a script or a shell line.
//!
//! [`validate`] is the schema by hand, rule for rule (the crate has no JSON Schema
//! validator and does not need one for a closed catalog of one action); the core
//! verifier (`approval_messages::pinned::check_action`) holds the same document with
//! `jsonschema`, and both read the same examples and invalid files in their tests.
//!
//! The catalog is one action, `k8s.delete-failed-pods`: `kubectl delete pods
//! --field-selector=status.phase=Failed -n <namespace>`, run from an argument vector
//! with the runner's allowlisted environment, in each namespace the action names or, with
//! none named, every namespace but the cluster's own ([`DENIED_NAMESPACES`]).

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Map, Value};

use super::env;
use super::provider;

/// The schema a run's `schema_id` names.
pub(crate) const HOST_ACTION: &str = "runner/host-action/v0";

/// The one action in the catalog.
pub(crate) const DELETE_FAILED_PODS: &str = "k8s.delete-failed-pods";

/// Namespaces no host action touches: the cluster's own.
pub(crate) const DENIED_NAMESPACES: &[&str] = &["kube-system", "kube-public", "kube-node-lease"];

/// How long one `kubectl` call may take.
const KUBECTL_TIMEOUT: Duration = Duration::from_secs(120);

/// The most `kubectl` output kept from one call, in bytes.
const OUTPUT_CAP: usize = 16 * 1024;

/// `^[a-z0-9]+(-[a-z0-9]+)*$`: one word of an action id.
fn is_word(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// `^[a-z0-9]+(-[a-z0-9]+)*(\.[a-z0-9]+(-[a-z0-9]+)*)+$`, at most 64 characters.
pub(crate) fn is_action_id(s: &str) -> bool {
    let words: Vec<&str> = s.split('.').collect();
    s.len() <= 64 && words.len() >= 2 && words.iter().all(|w| is_word(w))
}

/// `^rnr_[0-9A-Z]{26}$`.
pub(crate) fn is_runner_id(s: &str) -> bool {
    s.strip_prefix("rnr_").is_some_and(|rest| {
        rest.len() == 26
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    })
}

/// A Kubernetes namespace name (a DNS label): 1 to 63 characters,
/// `^[a-z0-9]([-a-z0-9]*[a-z0-9])?$`.
pub(crate) fn is_dns_label(s: &str) -> bool {
    let b = s.as_bytes();
    let edge = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    (1..=63).contains(&b.len())
        && edge(b[0])
        && edge(b[b.len() - 1])
        && b.iter().all(|&c| edge(c) || c == b'-')
}

/// The effect sentence: 1 to 200 characters, no control character.
fn is_effect(s: &str) -> bool {
    let n = s.chars().count();
    (1..=200).contains(&n)
        && !s
            .chars()
            .any(|c| matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
}

fn closed(m: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    match m.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(_) => Err(format!("{at}: a field the schema does not allow")),
        None => Ok(()),
    }
}

/// Whether `action` is a `runner/host-action/v0` document: the schema's rules, in its
/// order. The error names where, never a value (a value is the requester's text).
///
/// # Errors
/// The first rule it breaks.
pub(crate) fn validate(action: &Value) -> Result<(), String> {
    let Value::Object(m) = action else {
        return Err("/: not an object".to_owned());
    };
    closed(m, &["action", "runner_id", "params", "effect"], "/")?;
    for k in ["action", "runner_id", "params", "effect"] {
        if !m.contains_key(k) {
            return Err(format!("/{k}: required"));
        }
    }
    let id = m["action"].as_str().ok_or("/action: not a string")?;
    if !is_action_id(id) {
        return Err("/action: not an action id".to_owned());
    }
    let runner = m["runner_id"].as_str().ok_or("/runner_id: not a string")?;
    if !is_runner_id(runner) {
        return Err("/runner_id: not a runner id".to_owned());
    }
    let Value::Object(params) = &m["params"] else {
        return Err("/params: not an object".to_owned());
    };
    let effect = m["effect"].as_str().ok_or("/effect: not a string")?;
    if !is_effect(effect) {
        return Err("/effect: 1 to 200 characters on one line".to_owned());
    }
    // oneOf: exactly one catalog entry matches.
    match id {
        DELETE_FAILED_PODS => {
            closed(params, &["namespace"], "/params")?;
            if let Some(ns) = params.get("namespace") {
                let ns = ns.as_str().ok_or("/params/namespace: not a string")?;
                if !is_dns_label(ns) {
                    return Err("/params/namespace: not a namespace name".to_owned());
                }
            }
            Ok(())
        }
        _ => Err("/action: unknown action".to_owned()),
    }
}

/// An action this runner knows, with its parameters read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// `k8s.delete-failed-pods`: in `namespace`, or in every namespace when `None`.
    DeleteFailedPods { namespace: Option<String> },
}

/// The registry: the action `doc` names, from a document [`validate`] passed. An id
/// that is not in the catalog is `unknown action`.
///
/// # Errors
/// `unknown action`, a parameter of the wrong type, or a namespace refused by
/// [`namespace_allowed`].
pub(crate) fn lookup(doc: &Value) -> Result<Action, String> {
    match doc.get("action").and_then(Value::as_str) {
        Some(DELETE_FAILED_PODS) => {
            let namespace = match doc.pointer("/params/namespace") {
                None => None,
                Some(Value::String(ns)) => {
                    // Refused here, before anything is reported started; carry_out checks
                    // again.
                    namespace_allowed(ns)?;
                    Some(ns.clone())
                }
                Some(_) => return Err("params.namespace: not a string".to_owned()),
            };
            Ok(Action::DeleteFailedPods { namespace })
        }
        _ => Err("unknown action".to_owned()),
    }
}

/// Whether failed pods may be deleted in `ns`: a namespace name, and not the cluster's
/// own.
///
/// # Errors
/// What is wrong with it; the name is repeated only when it is a valid one.
pub(crate) fn namespace_allowed(ns: &str) -> Result<(), String> {
    if !is_dns_label(ns) {
        return Err("namespace: not a namespace name".to_owned());
    }
    if DENIED_NAMESPACES.contains(&ns) {
        return Err(format!("namespace {ns}: refused, it is the cluster's own"));
    }
    Ok(())
}

/// `kubectl`'s arguments to delete the failed pods of one namespace: an argument vector,
/// never a shell line, and only for an allowed namespace.
///
/// # Errors
/// The namespace is not allowed ([`namespace_allowed`]).
pub(crate) fn delete_failed_pods_argv(ns: &str) -> Result<Vec<String>, String> {
    namespace_allowed(ns)?;
    Ok(vec![
        "delete".to_owned(),
        "pods".to_owned(),
        "--field-selector=status.phase=Failed".to_owned(),
        "-n".to_owned(),
        ns.to_owned(),
    ])
}

/// `kubectl`'s arguments to list every namespace's name, space-separated.
pub(crate) fn list_namespaces_argv() -> Vec<String> {
    vec![
        "get".to_owned(),
        "namespaces".to_owned(),
        "-o".to_owned(),
        "jsonpath={.items[*].metadata.name}".to_owned(),
    ]
}

/// What one `kubectl` call gave.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Output {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Runs `kubectl` with an argument vector.
pub(crate) trait Kubectl {
    async fn run(&self, argv: &[String]) -> Result<Output, String>;
}

/// The host's `kubectl`: the first on the runner's `PATH`, started directly (no shell)
/// with the runner's allowlisted environment ([`env::scrub`]) and `KUBECONFIG`, no
/// standard input, killed when it outlives [`KUBECTL_TIMEOUT`].
#[derive(Debug, Clone)]
pub(crate) struct HostKubectl {
    pub(crate) program: PathBuf,
    pub(crate) env: Vec<(OsString, OsString)>,
}

impl HostKubectl {
    /// `kubectl` from this process's `PATH`, or `None` when it is not installed.
    pub(crate) fn find() -> Option<Self> {
        let program = provider::find_on_path("kubectl", std::env::var_os("PATH").as_ref())?;
        let mut env = env::scrub(std::env::vars_os());
        if let Some(k) = std::env::var_os("KUBECONFIG") {
            env.push(("KUBECONFIG".into(), k));
        }
        Some(Self { program, env })
    }
}

fn capped(bytes: &[u8]) -> String {
    let end = bytes.len().min(OUTPUT_CAP);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

impl Kubectl for HostKubectl {
    async fn run(&self, argv: &[String]) -> Result<Output, String> {
        let child = tokio::process::Command::new(&self.program)
            .args(argv)
            .env_clear()
            .envs(self.env.iter().cloned())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("kubectl did not start: {}", e.kind()))?;
        let out = tokio::time::timeout(KUBECTL_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| format!("kubectl took over {}s", KUBECTL_TIMEOUT.as_secs()))?
            .map_err(|e| format!("kubectl failed: {}", e.kind()))?;
        Ok(Output {
            success: out.status.success(),
            stdout: capped(&out.stdout),
            stderr: capped(&out.stderr),
        })
    }
}

/// Carries out `action` with `kubectl`; the answer says what was done, one line per
/// namespace.
///
/// # Errors
/// The first call that failed, with what `kubectl` said, after the namespaces done.
pub(crate) async fn carry_out(action: &Action, kubectl: &impl Kubectl) -> Result<String, String> {
    match action {
        Action::DeleteFailedPods { namespace } => {
            let namespaces: Vec<String> = if let Some(ns) = namespace {
                namespace_allowed(ns)?;
                vec![ns.clone()]
            } else {
                let out = kubectl.run(&list_namespaces_argv()).await?;
                if !out.success {
                    return Err(format!("listing namespaces: {}", out.stderr.trim()));
                }
                out.stdout
                    .split_whitespace()
                    .filter(|ns| namespace_allowed(ns).is_ok())
                    .map(str::to_owned)
                    .collect()
            };
            let mut done = Vec::new();
            for ns in &namespaces {
                let out = kubectl.run(&delete_failed_pods_argv(ns)?).await?;
                if !out.success {
                    done.push(format!("{ns}: {}", out.stderr.trim()));
                    return Err(done.join("\n"));
                }
                let said = out.stdout.trim();
                done.push(format!(
                    "{ns}: {}",
                    if said.is_empty() {
                        "no failed pods"
                    } else {
                        said
                    }
                ));
            }
            if done.is_empty() {
                return Ok("no namespace to clean".to_owned());
            }
            Ok(done.join("\n"))
        }
    }
}

/// A `kubectl` that records each argument vector and answers from a script, for tests.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use super::{Kubectl, Output};

    pub(crate) struct Fake {
        pub(crate) namespaces: &'static str,
        pub(crate) calls: Mutex<Vec<Vec<String>>>,
        pub(crate) fail_delete: bool,
    }

    #[allow(
        unknown_lints,
        clippy::unused_async_trait_impl,
        reason = "a fake answers at once; the trait is async for the real kubectl"
    )]
    impl Kubectl for Fake {
        async fn run(&self, argv: &[String]) -> Result<Output, String> {
            self.calls.lock().unwrap().push(argv.to_vec());
            if argv[0] == "get" {
                return Ok(Output {
                    success: true,
                    stdout: self.namespaces.to_owned(),
                    stderr: String::new(),
                });
            }
            Ok(Output {
                success: !self.fail_delete,
                stdout: format!("pod \"x\" deleted from {}", argv[4]),
                stderr: if self.fail_delete {
                    "forbidden".to_owned()
                } else {
                    String::new()
                },
            })
        }
    }

    pub(crate) fn fake(namespaces: &'static str, fail_delete: bool) -> Fake {
        Fake {
            namespaces,
            calls: Mutex::new(Vec::new()),
            fail_delete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::fake;
    use super::*;
    use serde_json::json;

    fn doc(params: &Value) -> Value {
        json!({
            "action": DELETE_FAILED_PODS,
            "runner_id": "rnr_00000000000000000000000001",
            "params": params,
            "effect": "Deletes every failed pod in namespace example.",
        })
    }

    #[test]
    fn the_examples_validate_and_every_invalid_file_is_refused() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/host-action");
        let read = |p: std::path::PathBuf| -> Value {
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
        };
        let mut n = 0;
        for e in std::fs::read_dir(dir.join("schema/examples")).unwrap() {
            let p = e.unwrap().path();
            assert_eq!(validate(&read(p.clone())), Ok(()), "{}", p.display());
            n += 1;
        }
        assert_eq!(n, 2);
        let mut n = 0;
        for e in std::fs::read_dir(dir.join("schema/invalid")).unwrap() {
            let p = e.unwrap().path();
            assert!(validate(&read(p.clone())).is_err(), "{}", p.display());
            n += 1;
        }
        assert_eq!(n, 7);
    }

    #[test]
    fn the_patterns_hold_at_their_edges() {
        assert!(is_action_id("k8s.delete-failed-pods"));
        assert!(!is_action_id("k8s"));
        assert!(!is_action_id("k8s..x"));
        assert!(!is_action_id("k8s.-x"));
        assert!(!is_action_id("K8s.x"));
        assert!(!is_action_id(&format!("a.{}", "b".repeat(63))));
        assert!(is_runner_id("rnr_01M5Z0WF3MRKDQ75T7F72E3F9N"));
        assert!(!is_runner_id("rnr_01m5z0wf3mrkdq75t7f72e3f9n"));
        assert!(!is_runner_id("rnr_01M5Z0WF3MRKDQ75T7F72E3F9"));
        assert!(is_dns_label("a"));
        assert!(is_dns_label("tbd-dev"));
        assert!(is_dns_label(&"a".repeat(63)));
        assert!(!is_dns_label(&"a".repeat(64)));
        assert!(!is_dns_label("-a"));
        assert!(!is_dns_label("a-"));
        assert!(!is_dns_label("A"));
        assert!(!is_dns_label("example; rm -rf /"));
        assert!(!is_dns_label(""));
    }

    #[test]
    fn an_id_outside_the_catalog_is_an_unknown_action() {
        let mut d = doc(&json!({}));
        d["action"] = json!("k8s.delete-namespace");
        assert_eq!(validate(&d), Err("/action: unknown action".to_owned()));
        assert_eq!(lookup(&d), Err("unknown action".to_owned()));
        assert_eq!(
            lookup(&doc(&json!({"namespace": "example"}))),
            Ok(Action::DeleteFailedPods {
                namespace: Some("example".to_owned())
            })
        );
    }

    #[test]
    fn the_cluster_namespaces_and_bad_names_are_refused() {
        for ns in DENIED_NAMESPACES {
            assert!(delete_failed_pods_argv(ns).is_err(), "{ns}");
        }
        assert!(delete_failed_pods_argv("example; rm -rf /").is_err());
        assert!(delete_failed_pods_argv("$(id)").is_err());
        assert!(delete_failed_pods_argv("").is_err());
    }

    #[test]
    fn the_argv_is_kubectl_arguments_with_no_shell() {
        let argv = delete_failed_pods_argv("example").unwrap();
        assert_eq!(
            argv,
            [
                "delete",
                "pods",
                "--field-selector=status.phase=Failed",
                "-n",
                "example"
            ]
        );
        // No shell between us and kubectl: no `sh`, no `-c`, nothing a shell would read.
        assert!(argv.iter().all(|a| a != "sh" && a != "-c" && a != "bash"));
        assert!(
            argv.iter()
                .all(|a| !a.contains([';', '|', '&', '$', '`', ' ', '\n']))
        );
    }

    #[tokio::test]
    async fn every_namespace_skips_the_cluster_s_own() {
        let k = fake(
            "default kube-system tbd kube-public kube-node-lease BAD",
            false,
        );
        let said = carry_out(&Action::DeleteFailedPods { namespace: None }, &k)
            .await
            .unwrap();
        let calls = k.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], list_namespaces_argv());
        assert_eq!(calls[1][4], "default");
        assert_eq!(calls[2][4], "tbd");
        assert!(said.contains("default:") && said.contains("tbd:"));
    }

    #[tokio::test]
    async fn a_named_cluster_namespace_runs_nothing() {
        let k = fake("", false);
        let r = carry_out(
            &Action::DeleteFailedPods {
                namespace: Some("kube-system".to_owned()),
            },
            &k,
        )
        .await;
        assert!(r.is_err());
        assert!(k.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_delete_is_a_failure_with_kubectl_s_words() {
        let k = fake("", true);
        let r = carry_out(
            &Action::DeleteFailedPods {
                namespace: Some("example".to_owned()),
            },
            &k,
        )
        .await;
        assert_eq!(r, Err("example: forbidden".to_owned()));
    }
}
