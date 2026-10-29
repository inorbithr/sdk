//! Host actions (platform RFC 0172 slice 3, `schemas/runner/host-action/v0.json`): the
//! few things an approved pinned run may make this runner do on its own host, each a
//! native function with typed parameters, never a script or a shell line.
//!
//! [`validate`] is the schema by hand, rule for rule (the crate has no JSON Schema
//! validator and does not need one for a closed catalog); the core verifier
//! (`approval_messages::pinned::check_action`) holds the same document with
//! `jsonschema`, and both read the same examples and invalid files in their tests.
//!
//! The catalog, each entry one or two argument vectors for one host tool:
//!
//! - `k8s.delete-failed-pods`: `kubectl delete pods --field-selector=status.phase=Failed
//!   -n <namespace>`, in the namespace named or, with none named, every namespace but the
//!   cluster's own ([`DENIED_NAMESPACES`]).
//! - `k8s.rollout-restart`: `kubectl rollout restart deployment/<d> -n <ns>`, then
//!   `kubectl rollout status` until it is out or [`ROLLOUT_WAIT`] passes; never in the
//!   cluster's own namespaces.
//! - `systemd.user-restart`: `systemctl --user daemon-reload`, then `systemctl --user
//!   restart <unit>`. Linux only; any other runner refuses it before it is started.
//! - `docker.prune-images`: `docker image prune -af --filter until=<h>h`. Never volumes,
//!   networks or `system prune`.
//! - `gh.cancel-run`: `gh run cancel <id> -R inorbithr/<repo>`, with the host's own `gh`
//!   sign-in (the one place the runner lets `gh` use it).
//! - `iohr.upgrade`: this runner's own executable with `upgrade`: the `iohr upgrade` path
//!   (Homebrew, or the install script's archive checked against the release's checksums
//!   and attestations), never a downloaded script.
//!
//! Every tool runs through one [`ArgvRunner`]: [`HostRunner`] starts the program from
//! the runner's `PATH` directly (no shell) with the allowlisted environment, no standard
//! input and a time limit per tool. Every `kubectl` call is pinned to the cluster the
//! runner's config names (`kube_context` and/or `kubeconfig` under `[runner]`,
//! [`KubeTarget`]); with neither set a Kubernetes action is refused and nothing runs.
//! What a tool says back is read as it comes, in bounded memory ([`Capture`]): the
//! first [`OUTPUT_CAP`] bytes are kept, cut only between words so no secret is left
//! half there, and the counts a summary needs are taken from the whole stream. It goes
//! into a report only after [`redact_output`], and the report is cut to [`OUTPUT_CAP`]
//! after that.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Map, Value};

use super::env;
use super::exec::GroupGuard;
use super::provider;

/// The schema a run's `schema_id` names.
pub(crate) const HOST_ACTION: &str = "runner/host-action/v0";

pub(crate) const DELETE_FAILED_PODS: &str = "k8s.delete-failed-pods";
pub(crate) const ROLLOUT_RESTART: &str = "k8s.rollout-restart";
pub(crate) const USER_RESTART: &str = "systemd.user-restart";
pub(crate) const PRUNE_IMAGES: &str = "docker.prune-images";
pub(crate) const CANCEL_RUN: &str = "gh.cancel-run";
pub(crate) const UPGRADE: &str = "iohr.upgrade";

/// Namespaces no host action touches: the cluster's own.
pub(crate) const DENIED_NAMESPACES: &[&str] = &["kube-system", "kube-public", "kube-node-lease"];

/// How long `kubectl rollout status` waits for a restarted deployment.
pub(crate) const ROLLOUT_WAIT: &str = "240s";

/// The most output kept from one call, and the most a report says, in bytes.
pub(crate) const OUTPUT_CAP: usize = 16 * 1024;

/// The largest `until_hours`: 30 days.
const MAX_UNTIL_HOURS: u64 = 720;

/// The largest run id: 2^53 - 1, exact in every language.
const MAX_RUN_ID: u64 = 9_007_199_254_740_991;

/// The only repository owner `gh.cancel-run` reaches.
const REPO_OWNER: &str = "inorbithr/";

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

/// A Kubernetes name (a DNS label): 1 to 63 characters, `^[a-z0-9]([-a-z0-9]*[a-z0-9])?$`.
pub(crate) fn is_dns_label(s: &str) -> bool {
    let b = s.as_bytes();
    let edge = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    (1..=63).contains(&b.len())
        && edge(b[0])
        && edge(b[b.len() - 1])
        && b.iter().all(|&c| edge(c) || c == b'-')
}

/// A user service unit: `^[a-z0-9][a-z0-9@._-]{0,62}\.service$`.
pub(crate) fn is_user_unit(s: &str) -> bool {
    s.strip_suffix(".service").is_some_and(|stem| {
        let b = stem.as_bytes();
        (1..=63).contains(&b.len())
            && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
            && b.iter().all(|&c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, b'@' | b'.' | b'_' | b'-')
            })
    })
}

/// A repository of the one owner: `^inorbithr/(?!\.\.?$)[a-z0-9._-]{1,100}$`, the name
/// held to [`super::workspace::is_repository_part`] and lower case.
pub(crate) fn is_repo(s: &str) -> bool {
    s.strip_prefix(REPO_OWNER).is_some_and(|name| {
        super::workspace::is_repository_part(name) && !name.bytes().any(|c| c.is_ascii_uppercase())
    })
}

/// The effect sentence: 1 to 200 characters, no control character.
fn is_effect(s: &str) -> bool {
    let n = s.chars().count();
    (1..=200).contains(&n)
        && !s
            .chars()
            .any(|c| matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
}

/// What `action` with `params` does, one plain sentence in the catalog's own words, built
/// from the params alone: the same function as core's
/// `approval_messages::pinned::effect_of`, which refuses a run whose `effect` differs. The
/// owner approves this sentence, so the runner, which executes, refuses a run whose
/// `effect` is not it. `None` for an action outside the catalog or a param of the wrong
/// type; `None` means refuse.
#[must_use]
pub(crate) fn effect_of(action: &str, params: &Value) -> Option<String> {
    let text = |k: &str| params.get(k).and_then(Value::as_str);
    let int = |k: &str| params.get(k).and_then(Value::as_i64);
    Some(match action {
        DELETE_FAILED_PODS => match params.get("namespace") {
            None => "Deletes every failed pod in every namespace.".to_owned(),
            Some(_) => format!(
                "Deletes every failed pod in namespace {}.",
                text("namespace")?
            ),
        },
        ROLLOUT_RESTART => format!(
            "Restarts deployment {} in namespace {} and waits for the new pods.",
            text("deployment")?,
            text("namespace")?
        ),
        USER_RESTART => format!(
            "Restarts the user service {} on the runner's host.",
            text("unit")?
        ),
        PRUNE_IMAGES => format!(
            "Removes the unused Docker images older than {} hours on the runner's host.",
            int("until_hours")?
        ),
        CANCEL_RUN => format!(
            "Cancels GitHub Actions run {} of {}.",
            int("run_id")?,
            text("repo")?
        ),
        UPGRADE => "Upgrades iohr on the runner's host to the newest release.".to_owned(),
        _ => return None,
    })
}

fn closed(m: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    match m.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(_) => Err(format!("{at}: a field the schema does not allow")),
        None => Ok(()),
    }
}

/// A required string parameter that `ok` accepts.
fn string_param<'a>(
    params: &'a Map<String, Value>,
    key: &str,
    ok: fn(&str) -> bool,
    what: &str,
) -> Result<&'a str, String> {
    let v = params
        .get(key)
        .ok_or_else(|| format!("/params/{key}: required"))?;
    let s = v
        .as_str()
        .ok_or_else(|| format!("/params/{key}: not a string"))?;
    if !ok(s) {
        return Err(format!("/params/{key}: not {what}"));
    }
    Ok(s)
}

/// A required integer parameter in `lo..=hi`. `24.0` is not an integer here, as it is not
/// to the core verifier (which refuses every float).
fn integer_param(params: &Map<String, Value>, key: &str, lo: u64, hi: u64) -> Result<u64, String> {
    let v = params
        .get(key)
        .ok_or_else(|| format!("/params/{key}: required"))?;
    v.as_u64()
        .filter(|n| (lo..=hi).contains(n))
        .ok_or_else(|| format!("/params/{key}: not an integer from {lo} to {hi}"))
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
            if params.contains_key("namespace") {
                string_param(params, "namespace", is_dns_label, "a namespace name")?;
            }
        }
        ROLLOUT_RESTART => {
            closed(params, &["namespace", "deployment"], "/params")?;
            string_param(params, "namespace", is_dns_label, "a namespace name")?;
            string_param(params, "deployment", is_dns_label, "a deployment name")?;
        }
        USER_RESTART => {
            closed(params, &["unit"], "/params")?;
            string_param(params, "unit", is_user_unit, "a user service unit")?;
        }
        PRUNE_IMAGES => {
            closed(params, &["until_hours"], "/params")?;
            integer_param(params, "until_hours", 1, MAX_UNTIL_HOURS)?;
        }
        CANCEL_RUN => {
            closed(params, &["repo", "run_id"], "/params")?;
            string_param(params, "repo", is_repo, "an inorbithr repository")?;
            integer_param(params, "run_id", 1, MAX_RUN_ID)?;
        }
        UPGRADE => closed(params, &[], "/params")?,
        _ => return Err("/action: unknown action".to_owned()),
    }
    if effect_of(id, &m["params"]).as_deref() != Some(effect) {
        return Err("/effect: not the catalog's sentence for the action and its params".to_owned());
    }
    Ok(())
}

/// An action this runner knows, with its parameters read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// `k8s.delete-failed-pods`: in `namespace`, or in every namespace when `None`.
    DeleteFailedPods { namespace: Option<String> },
    /// `k8s.rollout-restart`.
    RolloutRestart {
        namespace: String,
        deployment: String,
    },
    /// `systemd.user-restart`.
    UserRestart { unit: String },
    /// `docker.prune-images`.
    PruneImages { until_hours: u64 },
    /// `gh.cancel-run`.
    CancelRun { repo: String, run_id: u64 },
    /// `iohr.upgrade`.
    Upgrade,
}

impl Action {
    /// The catalog id: what a log line names, never the parameters.
    pub(crate) fn id(&self) -> &'static str {
        match self {
            Self::DeleteFailedPods { .. } => DELETE_FAILED_PODS,
            Self::RolloutRestart { .. } => ROLLOUT_RESTART,
            Self::UserRestart { .. } => USER_RESTART,
            Self::PruneImages { .. } => PRUNE_IMAGES,
            Self::CancelRun { .. } => CANCEL_RUN,
            Self::Upgrade => UPGRADE,
        }
    }

    /// The one tool every call of this action runs.
    pub(crate) fn tool(&self) -> Tool {
        match self {
            Self::DeleteFailedPods { .. } | Self::RolloutRestart { .. } => Tool::Kubectl,
            Self::UserRestart { .. } => Tool::Systemctl,
            Self::PruneImages { .. } => Tool::Docker,
            Self::CancelRun { .. } => Tool::Gh,
            Self::Upgrade => Tool::Iohr,
        }
    }
}

/// The registry: the action `doc` names, from a document [`validate`] passed, on this
/// runner's system. An id that is not in the catalog is `unknown action`.
///
/// # Errors
/// `unknown action`, a parameter of the wrong type, a namespace refused by
/// [`namespace_allowed`], or an action this system cannot carry out.
pub(crate) fn lookup(doc: &Value) -> Result<Action, String> {
    lookup_on(doc, std::env::consts::OS)
}

/// [`lookup`] for a runner on `os` (`std::env::consts::OS`).
pub(crate) fn lookup_on(doc: &Value, os: &str) -> Result<Action, String> {
    let text = |key: &str| -> Result<String, String> {
        doc.pointer(&format!("/params/{key}"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("params.{key}: not a string"))
    };
    let int = |key: &str| -> Result<u64, String> {
        doc.pointer(&format!("/params/{key}"))
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("params.{key}: not an integer"))
    };
    // Refused here, before anything is reported started; carry_out checks again.
    let action = match doc.get("action").and_then(Value::as_str) {
        Some(DELETE_FAILED_PODS) => Action::DeleteFailedPods {
            namespace: match doc.pointer("/params/namespace") {
                None => None,
                Some(_) => Some(text("namespace")?),
            },
        },
        Some(ROLLOUT_RESTART) => Action::RolloutRestart {
            namespace: text("namespace")?,
            deployment: text("deployment")?,
        },
        Some(USER_RESTART) => Action::UserRestart {
            unit: text("unit")?,
        },
        Some(PRUNE_IMAGES) => Action::PruneImages {
            until_hours: int("until_hours")?,
        },
        Some(CANCEL_RUN) => Action::CancelRun {
            repo: text("repo")?,
            run_id: int("run_id")?,
        },
        Some(UPGRADE) => Action::Upgrade,
        _ => return Err("unknown action".to_owned()),
    };
    plan(&action, os)?;
    Ok(action)
}

/// Whether `ns` may be touched: a namespace name, and not the cluster's own.
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

fn strings<const N: usize>(argv: [&str; N]) -> Vec<String> {
    argv.into_iter().map(str::to_owned).collect()
}

/// `kubectl`'s arguments to delete the failed pods of one namespace: an argument vector,
/// never a shell line, and only for an allowed namespace.
///
/// # Errors
/// The namespace is not allowed ([`namespace_allowed`]).
pub(crate) fn delete_failed_pods_argv(ns: &str) -> Result<Vec<String>, String> {
    namespace_allowed(ns)?;
    Ok(strings([
        "delete",
        "pods",
        "--field-selector=status.phase=Failed",
        "-n",
        ns,
    ]))
}

/// `kubectl`'s arguments to list every namespace's name, space-separated.
pub(crate) fn list_namespaces_argv() -> Vec<String> {
    strings([
        "get",
        "namespaces",
        "-o",
        "jsonpath={.items[*].metadata.name}",
    ])
}

/// The host tools an action may start, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Kubectl,
    Systemctl,
    Docker,
    Gh,
    /// This runner's own executable.
    Iohr,
}

impl Tool {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Kubectl => "kubectl",
            Self::Systemctl => "systemctl",
            Self::Docker => "docker",
            Self::Gh => "gh",
            Self::Iohr => "iohr",
        }
    }

    /// How long one call may take: `rollout status` waits [`ROLLOUT_WAIT`] itself, a
    /// prune or an upgrade may move a lot of data.
    fn timeout(self) -> Duration {
        Duration::from_secs(match self {
            Self::Kubectl => 300,
            Self::Systemctl | Self::Gh => 120,
            Self::Docker | Self::Iohr => 900,
        })
    }
}

/// The calls one action makes, in order, checked against what this system can do: every
/// parameter is checked again here, so no path to a tool skips it.
///
/// # Errors
/// A parameter the catalog does not allow, or an action this system cannot carry out.
pub(crate) fn plan(action: &Action, os: &str) -> Result<Vec<(Tool, Vec<String>)>, String> {
    Ok(match action {
        // Its calls depend on the cluster's namespaces; carry_out builds them.
        Action::DeleteFailedPods { namespace } => {
            if let Some(ns) = namespace {
                namespace_allowed(ns)?;
            }
            Vec::new()
        }
        Action::RolloutRestart {
            namespace,
            deployment,
        } => {
            namespace_allowed(namespace)?;
            if !is_dns_label(deployment) {
                return Err("deployment: not a deployment name".to_owned());
            }
            let target = format!("deployment/{deployment}");
            vec![
                (
                    Tool::Kubectl,
                    strings(["rollout", "restart", &target, "-n", namespace]),
                ),
                (
                    Tool::Kubectl,
                    strings([
                        "rollout",
                        "status",
                        &target,
                        "-n",
                        namespace,
                        &format!("--timeout={ROLLOUT_WAIT}"),
                    ]),
                ),
            ]
        }
        Action::UserRestart { unit } => {
            if os != "linux" {
                return Err(format!(
                    "{USER_RESTART}: only a Linux runner has systemd; this one runs on {os}"
                ));
            }
            if !is_user_unit(unit) {
                return Err("unit: not a user service unit".to_owned());
            }
            vec![
                (Tool::Systemctl, strings(["--user", "daemon-reload"])),
                (Tool::Systemctl, strings(["--user", "restart", unit])),
            ]
        }
        Action::PruneImages { until_hours } => {
            if !(1..=MAX_UNTIL_HOURS).contains(until_hours) {
                return Err(format!("until_hours: not from 1 to {MAX_UNTIL_HOURS}"));
            }
            vec![(
                Tool::Docker,
                strings([
                    "image",
                    "prune",
                    "-af",
                    "--filter",
                    &format!("until={until_hours}h"),
                ]),
            )]
        }
        Action::CancelRun { repo, run_id } => {
            if !is_repo(repo) {
                return Err("repo: not an inorbithr repository".to_owned());
            }
            if !(1..=MAX_RUN_ID).contains(run_id) {
                return Err("run_id: not a run id".to_owned());
            }
            vec![(
                Tool::Gh,
                strings(["run", "cancel", &run_id.to_string(), "-R", repo]),
            )]
        }
        Action::Upgrade => vec![(Tool::Iohr, strings(["upgrade"]))],
    })
}

/// What one call gave: each stream as [`Capture`] kept it, and the tally of all stdout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Output {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) tally: Tally,
}

#[cfg(test)]
impl Output {
    /// What a call that wrote `stdout` and `stderr` gives, read as [`HostRunner`] reads.
    pub(crate) fn new(success: bool, stdout: &str, stderr: &str) -> Self {
        let read = |text: &str| {
            let mut c = Capture::default();
            c.push(text.as_bytes());
            c.finish()
        };
        let (stdout, tally) = read(stdout);
        let (stderr, _) = read(stderr);
        Self {
            success,
            stdout,
            stderr,
            tally,
        }
    }
}

/// What a summary counts over a whole stream, however long it was.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    /// Lines that start with `deleted:` (`docker image prune`).
    pub(crate) deleted: usize,
    /// The last line that starts with `Total reclaimed space`, not yet redacted.
    pub(crate) total: Option<String>,
}

/// Longest line start a [`Tally`] looks at.
const LINE_HEAD: usize = 256;

/// One stream read in bounded memory: the first [`OUTPUT_CAP`] bytes, cut between words,
/// and a [`Tally`] of every line.
#[derive(Debug, Default)]
struct Capture {
    kept: Vec<u8>,
    full: bool,
    head: Vec<u8>,
    tally: Tally,
}

impl Capture {
    fn push(&mut self, chunk: &[u8]) {
        if !self.full {
            self.kept.extend_from_slice(chunk);
            if self.kept.len() > OUTPUT_CAP {
                self.full = true;
                // A word cut at the cap could be half a secret: drop it.
                if !self.kept[OUTPUT_CAP].is_ascii_whitespace()
                    && let Some(at) = self.kept[..OUTPUT_CAP]
                        .iter()
                        .rposition(u8::is_ascii_whitespace)
                {
                    self.kept.truncate(at + 1);
                } else {
                    self.kept.truncate(OUTPUT_CAP);
                }
            }
        }
        for &b in chunk {
            if b == b'\n' {
                self.end_line();
            } else if self.head.len() < LINE_HEAD {
                self.head.push(b);
            }
        }
    }

    fn end_line(&mut self) {
        let line = String::from_utf8_lossy(&self.head);
        let line = line.trim_end_matches('\r');
        if line.trim_start().starts_with("deleted:") {
            self.tally.deleted += 1;
        } else if line.starts_with("Total reclaimed space") {
            self.tally.total = Some(line.to_owned());
        }
        self.head.clear();
    }

    fn finish(mut self) -> (String, Tally) {
        if !self.head.is_empty() {
            self.end_line();
        }
        (
            clipped(String::from_utf8_lossy(&self.kept).into_owned()),
            self.tally,
        )
    }
}

/// Reads `stream` to its end through a [`Capture`].
async fn drain(
    stream: Option<impl tokio::io::AsyncRead + Unpin>,
) -> std::io::Result<(String, Tally)> {
    use tokio::io::AsyncReadExt as _;
    let mut capture = Capture::default();
    if let Some(mut stream) = stream {
        let mut buf = vec![0u8; 8 * 1024];
        loop {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            capture.push(&buf[..n]);
        }
    }
    Ok(capture.finish())
}

/// Runs one host tool with an argument vector: the only way a host action starts a
/// process.
pub(crate) trait ArgvRunner {
    async fn run(&self, tool: Tool, argv: &[String]) -> Result<Output, String>;

    /// Why this host does not allow `action` though the catalog does, asked before the run
    /// is reported started.
    fn refuses(&self, _action: &Action) -> Option<String> {
        None
    }
}

/// The cluster every `kubectl` call is pinned to: a context of the kubeconfig, a
/// kubeconfig file, or both. Never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KubeTarget {
    context: Option<String>,
    kubeconfig: Option<PathBuf>,
}

impl KubeTarget {
    /// The target the runner's config names.
    ///
    /// # Errors
    /// Neither is set (blank counts as not set), a context begins with `-` (it would be
    /// read as a flag) or holds a control character, or the kubeconfig path is not
    /// absolute.
    pub(crate) fn new(context: Option<&str>, kubeconfig: Option<&str>) -> Result<Self, String> {
        let context = context.map(str::trim).filter(|c| !c.is_empty());
        let kubeconfig = kubeconfig.map(str::trim).filter(|c| !c.is_empty());
        if context.is_none() && kubeconfig.is_none() {
            return Err(
                "no kube context is configured (set `kube_context` or `kubeconfig` under [runner])"
                    .to_owned(),
            );
        }
        if let Some(c) = context
            && (c.starts_with('-') || c.chars().any(char::is_control))
        {
            return Err("the configured kube context is not a context name".to_owned());
        }
        let kubeconfig = match kubeconfig {
            Some(k) if k.starts_with('-') || k.chars().any(char::is_control) => {
                return Err("the configured kubeconfig is not a path".to_owned());
            }
            Some(k) if !std::path::Path::new(k).is_absolute() => {
                return Err("the configured kubeconfig is not an absolute path".to_owned());
            }
            Some(k) => Some(PathBuf::from(k)),
            None => None,
        };
        Ok(Self {
            context: context.map(str::to_owned),
            kubeconfig,
        })
    }

    /// The arguments that go before every `kubectl` call's own.
    pub(crate) fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(k) = &self.kubeconfig {
            args.push("--kubeconfig".to_owned());
            args.push(k.to_string_lossy().into_owned());
        }
        if let Some(c) = &self.context {
            args.push("--context".to_owned());
            args.push(c.clone());
        }
        args
    }
}

/// The host's tools: each the first of its name on the runner's `PATH` (absolute entries
/// only), `iohr` this process's own executable, started directly (no shell) with the
/// runner's allowlisted environment ([`env::scrub`]), no standard input, killed when it
/// outlives its [`Tool`]'s time limit. `kubectl` gets the [`KubeTarget`] as its first
/// arguments and no `KUBECONFIG` of its own, or is refused when none is configured.
#[derive(Debug, Clone)]
pub(crate) struct HostRunner {
    pub(crate) path: Option<OsString>,
    pub(crate) env: Vec<(OsString, OsString)>,
    pub(crate) kube: Result<KubeTarget, String>,
    pub(crate) iohr: Option<PathBuf>,
    /// What `iohr upgrade` reads to tell how it was installed (`IOHR_INSTALL_DIR`,
    /// `HOMEBREW_PREFIX`), outside the allowlist; never the upgrade source overrides.
    pub(crate) install_env: Vec<(OsString, OsString)>,
    /// The only units `systemd.user-restart` may restart (the runner's `user_units`).
    pub(crate) user_units: Vec<String>,
}

impl HostRunner {
    /// The tools of this process's host, `kubectl` pinned to the configured cluster,
    /// `systemctl --user restart` to the listed units.
    pub(crate) fn find(
        context: Option<&str>,
        kubeconfig: Option<&str>,
        user_units: &[String],
    ) -> Self {
        let install_env = ["IOHR_INSTALL_DIR", "HOMEBREW_PREFIX"]
            .into_iter()
            .filter_map(|k| std::env::var_os(k).map(|v| (OsString::from(k), v)))
            .collect();
        Self {
            path: std::env::var_os("PATH"),
            env: env::scrub(std::env::vars_os()),
            kube: KubeTarget::new(context, kubeconfig),
            iohr: std::env::current_exe().ok(),
            install_env,
            user_units: user_units.to_vec(),
        }
    }

    /// Why `unit` may not be restarted here, if it may not.
    fn unit_problem(&self, unit: &str) -> Option<String> {
        (!self.user_units.iter().any(|u| u == unit)).then(|| {
            if self.user_units.is_empty() {
                "this runner lists no user_units, so it restarts none".to_owned()
            } else {
                "the unit is not in this runner's user_units".to_owned()
            }
        })
    }

    /// Why a Kubernetes action cannot run here, if it cannot.
    pub(crate) fn kube_problem(&self) -> Option<&str> {
        self.kube.as_ref().err().map(String::as_str)
    }

    fn program(&self, tool: Tool) -> Result<PathBuf, String> {
        match tool {
            Tool::Iohr => self
                .iohr
                .clone()
                .ok_or_else(|| "this runner cannot tell where its iohr is".to_owned()),
            _ => provider::find_on_path(tool.name(), self.path.as_ref())
                .ok_or_else(|| format!("{} is not on this runner's PATH", tool.name())),
        }
    }

    fn env_for(&self, tool: Tool) -> Vec<(OsString, OsString)> {
        // Not the process's KUBECONFIG: the cluster is the configured one.
        let mut env: Vec<_> = self
            .env
            .iter()
            .filter(|(k, _)| k != "KUBECONFIG")
            .cloned()
            .collect();
        match tool {
            Tool::Gh => {
                for k in ["GH_PROMPT_DISABLED", "GH_NO_UPDATE_NOTIFIER"] {
                    env.push((k.into(), "1".into()));
                }
            }
            Tool::Iohr => env.extend(self.install_env.iter().cloned()),
            Tool::Kubectl | Tool::Systemctl | Tool::Docker => {}
        }
        env
    }
}

impl ArgvRunner for HostRunner {
    fn refuses(&self, action: &Action) -> Option<String> {
        let tool = action.tool();
        if let (Tool::Kubectl, Err(why)) = (tool, &self.kube) {
            return Some(why.clone());
        }
        if let Err(why) = self.program(tool) {
            return Some(why);
        }
        match action {
            Action::UserRestart { unit } => self.unit_problem(unit),
            _ => None,
        }
    }

    async fn run(&self, tool: Tool, argv: &[String]) -> Result<Output, String> {
        // Checked again here, so no caller restarts an unlisted unit.
        if let (Tool::Systemctl, Some(i)) = (tool, argv.iter().position(|a| a == "restart"))
            && let Some(why) = argv[i + 1..]
                .iter()
                .find_map(|unit| self.unit_problem(unit))
        {
            return Err(why);
        }
        let mut full = match (tool, &self.kube) {
            (Tool::Kubectl, Ok(target)) => target.args(),
            (Tool::Kubectl, Err(why)) => return Err(why.clone()),
            _ => Vec::new(),
        };
        full.extend_from_slice(argv);
        let name = tool.name();
        let mut c = tokio::process::Command::new(self.program(tool)?);
        c.args(&full)
            .env_clear()
            .envs(self.env_for(tool))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Its own process group, so a timeout takes what the tool started with it.
        #[cfg(unix)]
        c.process_group(0);
        let mut child = c
            .spawn()
            .map_err(|e| format!("{name} did not start: {}", e.kind()))?;
        let mut group = GroupGuard::new(child.id());
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let all = async {
            let (out, err, status) = tokio::join!(drain(stdout), drain(stderr), child.wait());
            Ok::<_, std::io::Error>((out?, err?, status?))
        };
        let ((stdout, tally), (stderr, _), status) = tokio::time::timeout(tool.timeout(), all)
            .await
            .map_err(|_| format!("{name} took over {}s", tool.timeout().as_secs()))?
            .map_err(|e| format!("{name} failed: {}", e.kind()))?;
        // Reaped: its pid may be reused, so nothing is left to kill.
        group.disarm();
        Ok(Output {
            success: status.success(),
            stdout,
            stderr,
            tally,
        })
    }
}

/// What a tool said, made safe to put in a report: any URL, `host:port`, IP address,
/// GitHub token, JWT, bearer value or long secret-looking word is replaced, line by line.
pub(crate) fn redact_output(text: &str) -> String {
    fn bare(w: &str) -> &str {
        w.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '<' | '>' | ':' | '.'
            )
        })
    }
    fn is_ip_or_host_port(w: &str) -> bool {
        let (host, port) = w.rsplit_once(':').map_or((w, None), |(h, p)| (h, Some(p)));
        let ipv4 = host.split('.').count() == 4
            && host
                .split('.')
                .all(|o| !o.is_empty() && o.len() <= 3 && o.bytes().all(|b| b.is_ascii_digit()));
        let port_ok = port.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        let dns_port = port_ok
            && host.contains('.')
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
        ipv4 || dns_port
            || (w.contains(':')
                && w.bytes().filter(|b| *b == b':').count() >= 2
                && w.bytes().all(|b| b.is_ascii_hexdigit() || b == b':'))
    }
    fn secretish(w: &str) -> bool {
        w.starts_with("eyJ")
            || (w.len() >= 32
                && w.bytes().all(|b| {
                    b.is_ascii_alphanumeric()
                        || matches!(b, b'-' | b'_' | b'+' | b'/' | b'=' | b'.')
                })
                && w.bytes().any(|b| b.is_ascii_digit())
                && w.bytes().any(|b| b.is_ascii_alphabetic()))
    }
    text.lines()
        .map(|line| {
            let line = super::workspace::redact(line);
            let mut after_bearer = false;
            // Words split on any whitespace, each kept with what followed it.
            line.split_inclusive(char::is_whitespace)
                .map(|piece| {
                    let w = piece.trim_end_matches(char::is_whitespace);
                    let core = bare(w);
                    if core.is_empty() {
                        return piece.to_owned();
                    }
                    let hide = after_bearer
                        || core.contains("://")
                        || is_ip_or_host_port(core)
                        || secretish(core);
                    after_bearer =
                        core.eq_ignore_ascii_case("bearer") || core.eq_ignore_ascii_case("token");
                    if hide && core != "[redacted]" {
                        format!("[redacted]{}", &piece[w.len()..])
                    } else {
                        piece.to_owned()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` cut to [`OUTPUT_CAP`] bytes on a character boundary.
fn clipped(mut text: String) -> String {
    if text.len() > OUTPUT_CAP {
        let mut end = OUTPUT_CAP;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Both streams of one call, redacted, on as few lines as they take.
fn said(out: &Output) -> String {
    [out.stdout.trim(), out.stderr.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(redact_output)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Carries out `action` with the host's tools; the answer says what was done.
///
/// # Errors
/// A refusal before anything ran, or the first call that failed, with what its tool said
/// (redacted), after what was done.
pub(crate) async fn carry_out(action: &Action, tools: &impl ArgvRunner) -> Result<String, String> {
    carry_out_on(action, tools, std::env::consts::OS).await
}

/// [`carry_out`] for a runner on `os`.
pub(crate) async fn carry_out_on(
    action: &Action,
    tools: &impl ArgvRunner,
    os: &str,
) -> Result<String, String> {
    let calls = plan(action, os)?;
    let result = if let Action::DeleteFailedPods { namespace } = action {
        delete_failed_pods(namespace.as_deref(), tools).await
    } else {
        let mut done = Vec::new();
        for (tool, argv) in &calls {
            let out = tools.run(*tool, argv).await?;
            let what = said(&out);
            if !out.success {
                let head: Vec<&str> = argv.iter().take(2).map(String::as_str).collect();
                done.push(format!("{} {}: failed", tool.name(), head.join(" ")));
                if !what.is_empty() {
                    done.push(what);
                }
                return Err(clipped(done.join("\n")));
            }
            if let Some(line) = summary(action, &what, &out.tally) {
                done.push(line);
            }
        }
        Ok(done.join("\n"))
    };
    result.map(clipped).map_err(clipped)
}

/// The line a person reads for one successful call, from what the tool said (redacted)
/// and the tally of all its stdout.
fn summary(action: &Action, what: &str, tally: &Tally) -> Option<String> {
    match action {
        Action::DeleteFailedPods { .. } => None,
        Action::RolloutRestart {
            namespace,
            deployment,
        } => (!what.is_empty()).then(|| format!("{namespace}/{deployment}: {what}")),
        Action::UserRestart { unit } => Some(if what.is_empty() {
            format!("{unit}: reloaded and restarted")
        } else {
            format!("{unit}: {what}")
        }),
        Action::PruneImages { .. } => {
            let total = tally.total.as_deref().map_or_else(
                || "Total reclaimed space: unknown".to_owned(),
                redact_output,
            );
            Some(format!("{} images removed; {total}", tally.deleted))
        }
        Action::CancelRun { repo, run_id } => Some(if what.is_empty() {
            format!("{repo} run {run_id}: cancel requested")
        } else {
            format!("{repo} run {run_id}: {what}")
        }),
        Action::Upgrade => Some(if what.is_empty() {
            "iohr upgrade: done".to_owned()
        } else {
            what.to_owned()
        }),
    }
}

/// `k8s.delete-failed-pods`: one line per namespace.
async fn delete_failed_pods(
    namespace: Option<&str>,
    tools: &impl ArgvRunner,
) -> Result<String, String> {
    let namespaces: Vec<String> = if let Some(ns) = namespace {
        namespace_allowed(ns)?;
        vec![ns.to_owned()]
    } else {
        let out = tools.run(Tool::Kubectl, &list_namespaces_argv()).await?;
        if !out.success {
            return Err(format!(
                "listing namespaces: {}",
                redact_output(out.stderr.trim())
            ));
        }
        out.stdout
            .split_whitespace()
            .filter(|ns| namespace_allowed(ns).is_ok())
            .map(str::to_owned)
            .collect()
    };
    let mut done = Vec::new();
    for ns in &namespaces {
        let out = tools
            .run(Tool::Kubectl, &delete_failed_pods_argv(ns)?)
            .await?;
        if !out.success {
            done.push(format!("{ns}: {}", redact_output(out.stderr.trim())));
            return Err(done.join("\n"));
        }
        let said = redact_output(out.stdout.trim());
        done.push(format!(
            "{ns}: {}",
            if said.is_empty() {
                "no failed pods"
            } else {
                &said
            }
        ));
    }
    if done.is_empty() {
        return Ok("no namespace to clean".to_owned());
    }
    Ok(done.join("\n"))
}

/// A host whose tools record each call and answer from a script, for tests.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use super::{Action, ArgvRunner, Output, Tool};

    pub(crate) struct Fake {
        /// What `kubectl get namespaces` answers.
        pub(crate) namespaces: &'static str,
        pub(crate) calls: Mutex<Vec<(Tool, Vec<String>)>>,
        /// Every other call fails, saying `forbidden`.
        pub(crate) fail: bool,
        /// Every call is refused before it starts, with this reason.
        pub(crate) missing: Option<String>,
        /// What every other call answers instead of the default.
        pub(crate) answer: Option<Output>,
        /// Why this host does not allow any action, asked before it starts.
        pub(crate) not_here: Option<String>,
    }

    impl Fake {
        pub(crate) fn calls(&self) -> Vec<(Tool, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }

        /// Whether no tool was started.
        pub(crate) fn ran_nothing(&self) -> bool {
            self.calls.lock().unwrap().is_empty()
        }
    }

    #[allow(
        unknown_lints,
        clippy::unused_async_trait_impl,
        reason = "a fake answers at once; the trait is async for the real tools"
    )]
    impl ArgvRunner for Fake {
        fn refuses(&self, _action: &Action) -> Option<String> {
            self.not_here.clone()
        }

        async fn run(&self, tool: Tool, argv: &[String]) -> Result<Output, String> {
            if let Some(why) = &self.missing {
                return Err(why.clone());
            }
            self.calls.lock().unwrap().push((tool, argv.to_vec()));
            if tool == Tool::Kubectl && argv[0] == "get" {
                return Ok(Output::new(true, self.namespaces, ""));
            }
            if let Some(out) = &self.answer {
                return Ok(out.clone());
            }
            let stdout = if tool == Tool::Kubectl && argv[0] == "delete" {
                format!("pod \"x\" deleted from {}", argv[4])
            } else {
                String::new()
            };
            Ok(Output::new(
                !self.fail,
                &stdout,
                if self.fail { "forbidden" } else { "" },
            ))
        }
    }

    pub(crate) fn fake(namespaces: &'static str, fail: bool) -> Fake {
        Fake {
            namespaces,
            calls: Mutex::new(Vec::new()),
            fail,
            missing: None,
            answer: None,
            not_here: None,
        }
    }
}

#[cfg(test)]
mod tests;
