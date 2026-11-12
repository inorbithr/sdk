//! Upgrading every installed extension, one at a time: what `iohr ext upgrade`, `iohr
//! upgrade` (after the binary) and the runner's own upgrade share.
//!
//! One extension failing never stops the others: each ends as a row of the report (name,
//! from, to, status), and the caller decides what is an error. A release that declares
//! new privileges is refused without `--yes` or a terminal; that is a row too, left for a
//! person, and not a failure.

use std::future::Future;
use std::path::Path;

use serde::Serialize;

use super::{Ask, Want, confirm_privileges, fetch, installed_note, keep, newest, setup, terminal};
use crate::Env;
use crate::cli::Global;
use crate::error::Error;
use crate::ext::install::Store;
use crate::ext::manifest::check_version;
use crate::output::Out;

/// How one extension ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    /// A newer release was installed.
    Upgraded,
    /// Already the newest.
    Current,
    /// The newest release declares new privileges nobody confirmed: left for a person.
    Refused,
    /// Something went wrong; the old version is still installed.
    Failed,
}

/// One extension's row of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExtUpgrade {
    pub(crate) name: String,
    /// The version installed before.
    pub(crate) from: String,
    /// The version installed now (none when refused or failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) to: Option<String>,
    pub(crate) status: Status,
    /// The refusal or the failure, in a line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
}

/// What one extension's upgrade came to, short of an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Upgraded(String),
    Current,
    Refused(String),
}

impl ExtUpgrade {
    pub(crate) fn failed(&self) -> bool {
        self.status == Status::Failed
    }
}

/// How many rows failed (a refusal is not a failure).
pub(crate) fn failures(rows: &[ExtUpgrade]) -> usize {
    rows.iter().filter(|r| r.failed()).count()
}

/// Runs `one` for each `(name, installed version)`, in order, and records how each
/// ended: an `Err` is that extension's failure and the loop goes on.
pub(crate) async fn each<F, Fut>(installed: Vec<(String, String)>, mut one: F) -> Vec<ExtUpgrade>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Step, Error>>,
{
    let mut rows = Vec::new();
    for (name, from) in installed {
        let row = match one(name.clone()).await {
            Ok(Step::Upgraded(to)) => ExtUpgrade {
                name,
                from,
                to: Some(to),
                status: Status::Upgraded,
                detail: None,
            },
            Ok(Step::Current) => ExtUpgrade {
                name,
                to: Some(from.clone()),
                from,
                status: Status::Current,
                detail: None,
            },
            Ok(Step::Refused(why)) => {
                Out::note(&format!("{name}: not upgraded: {why}"));
                ExtUpgrade {
                    name,
                    from,
                    to: None,
                    status: Status::Refused,
                    detail: Some(why),
                }
            }
            Err(e) => {
                Out::note(&format!("{name}: upgrade failed: {e}"));
                ExtUpgrade {
                    name,
                    from,
                    to: None,
                    status: Status::Failed,
                    detail: Some(e.to_string()),
                }
            }
        };
        rows.push(row);
    }
    rows
}

/// The privilege confirmation for an upgrade: `Ok(None)` to go on, `Ok(Some(why))` when
/// it was refused (no `--yes`, no terminal, or the person did not type `yes`: the same
/// message as [`confirm_privileges`] gives), `Err` for anything else.
pub(crate) fn gate(
    subject: &str,
    more: &[&str],
    yes: bool,
    ask: Option<&mut Ask<'_>>,
) -> Result<Option<String>, Error> {
    match confirm_privileges(subject, more, true, yes, ask) {
        Ok(()) => Ok(None),
        Err(Error::Usage(why)) => Ok(Some(why)),
        Err(e) => Err(e),
    }
}

/// Upgrades `only` (which must be installed) or every installed extension, one failure
/// at a time. `Err` only when nothing could start: the store, an unknown name, the
/// registry or the trust settings.
pub(crate) async fn all(
    g: &Global,
    env: &Env,
    store: &Store,
    only: Option<String>,
    lock: Option<&Path>,
    yes: bool,
) -> Result<Vec<ExtUpgrade>, Error> {
    let installed = store.lock()?;
    let names: Vec<String> = match only {
        Some(n) => {
            if !installed.entries.contains_key(&n) {
                return Err(Error::Usage(format!(
                    "{n} is not installed; `iohr ext install {n}` installs it"
                )));
            }
            vec![n]
        }
        None => installed.entries.keys().cloned().collect(),
    };
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let (registry, policy) = setup(g, env)?;
    let before: Vec<(String, String)> = names
        .into_iter()
        .map(|n| {
            let v = installed.entries[&n].version.clone();
            (n, v)
        })
        .collect();
    let rows = each(before, |n| {
        let (registry, policy, installed) = (&registry, &policy, &installed);
        async move {
            let current = &installed.entries[&n];
            let latest = newest(registry, &n).await?;
            let newer = match (check_version(&latest), check_version(&current.version)) {
                (Ok(l), Ok(c)) => l > c,
                _ => false,
            };
            if !newer {
                Out::note(&format!("{n} {} is the newest.", current.version));
                return Ok(Step::Current);
            }
            let scopes_before = store
                .installed(&n)?
                .map(|(_, r, _)| r.manifest.scopes)
                .unwrap_or_default();
            let f = fetch(registry, policy, &n, &Want::Version(latest)).await?;
            let added: Vec<&String> = f
                .manifest
                .scopes
                .iter()
                .filter(|s| !scopes_before.contains(s))
                .collect();
            let more = current.unconfirmed(&f.manifest.privileges);
            if let Some(why) = gate(
                &format!("{n} {}", f.manifest.version),
                &more,
                yes,
                terminal().as_mut().map(|a| a as &mut Ask),
            )? {
                return Ok(Step::Refused(why));
            }
            let entry = keep(store, registry, &f, lock, None)?;
            installed_note(&f, &entry);
            if !more.is_empty() {
                Out::note(&format!(
                    "  new privileges {} (its system service holds these)",
                    more.join(", ")
                ));
            }
            if !added.is_empty() {
                Out::note(&format!(
                    "  new scopes  {} (it may now ask for these)",
                    added
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(Step::Upgraded(f.manifest.version.clone()))
        }
    })
    .await;
    Ok(rows)
}

/// The lines a log gets for a pass: the summary, then each extension that changed, was
/// refused or failed (none when nothing is installed).
pub(crate) fn lines(rows: &[ExtUpgrade]) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut out = vec![summary(rows)];
    for r in rows {
        match (r.status, r.to.as_deref(), r.detail.as_deref()) {
            (Status::Upgraded, Some(to), _) => {
                out.push(format!("extension {}: {} -> {to}", r.name, r.from));
            }
            (Status::Refused | Status::Failed, _, Some(why)) => out.push(format!(
                "extension {} {} not upgraded ({}): {why}",
                r.name,
                r.from,
                if r.status == Status::Refused {
                    "left for a person"
                } else {
                    "failed"
                }
            )),
            _ => {}
        }
    }
    out
}

/// The report as `--json` prints it: `{"extensions": [...]}`.
pub(crate) fn json(rows: &[ExtUpgrade]) -> serde_json::Value {
    serde_json::json!({ "extensions": rows })
}

/// The one line that says what the pass came to, for a log.
pub(crate) fn summary(rows: &[ExtUpgrade]) -> String {
    let n = |s: Status| rows.iter().filter(|r| r.status == s).count();
    format!(
        "extensions: {} upgraded, {} current, {} refused, {} failed",
        n(Status::Upgraded),
        n(Status::Current),
        n(Status::Refused),
        n(Status::Failed)
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn installed(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| ((*n).to_owned(), "0.1.0".to_owned()))
            .collect()
    }

    #[tokio::test]
    async fn one_failure_does_not_stop_the_others() {
        let mut seen = Vec::new();
        let rows = each(installed(&["a", "b", "c"]), |n| {
            seen.push(n.clone());
            async move {
                match n.as_str() {
                    "a" => Ok(Step::Upgraded("0.2.0".into())),
                    "b" => Err(Error::Failed("registry unreachable".into())),
                    _ => Ok(Step::Upgraded("0.3.0".into())),
                }
            }
        })
        .await;
        assert_eq!(seen, ["a", "b", "c"]);
        let status: Vec<Status> = rows.iter().map(|r| r.status).collect();
        assert_eq!(status, [Status::Upgraded, Status::Failed, Status::Upgraded]);
        assert_eq!(failures(&rows), 1);
        assert_eq!(rows[1].detail.as_deref(), Some("registry unreachable"));
        assert_eq!(rows[2].to.as_deref(), Some("0.3.0"));
    }

    #[tokio::test]
    async fn a_privilege_increase_without_a_terminal_is_refused_and_the_others_upgrade() {
        // No --yes and no terminal (`None`): the refusal says what today's message says.
        let refused = gate("agent 0.2.0", &["CAP_NET_ADMIN"], false, None).unwrap();
        let why = refused.unwrap();
        assert!(
            why.contains("declares new privileges (CAP_NET_ADMIN)")
                && why.contains("nothing was installed"),
            "{why}"
        );
        // --yes goes on; nothing new goes on without asking.
        assert_eq!(
            gate("agent 0.2.0", &["CAP_NET_ADMIN"], true, None).unwrap(),
            None
        );
        assert_eq!(gate("agent 0.2.0", &[], false, None).unwrap(), None);

        let rows = each(installed(&["a", "capture", "c"]), |n| async move {
            if n == "capture" {
                Ok(gate(&format!("{n} 0.2.0"), &["CAP_NET_ADMIN"], false, None)
                    .unwrap()
                    .map_or(Step::Upgraded("0.2.0".into()), Step::Refused))
            } else {
                Ok(Step::Upgraded("0.2.0".into()))
            }
        })
        .await;
        let status: Vec<Status> = rows.iter().map(|r| r.status).collect();
        assert_eq!(
            status,
            [Status::Upgraded, Status::Refused, Status::Upgraded]
        );
        // A refusal is left for a person: it is not a failure, so the exit stays 0.
        assert_eq!(failures(&rows), 0);
        assert!(rows[1].to.is_none());
    }

    #[tokio::test]
    async fn the_json_report_has_name_from_to_and_status() {
        let rows = each(installed(&["up", "same", "no", "bad"]), |n| async move {
            match n.as_str() {
                "up" => Ok(Step::Upgraded("0.2.0".into())),
                "same" => Ok(Step::Current),
                "no" => Ok(Step::Refused("needs a person".into())),
                _ => Err(Error::Failed("boom".into())),
            }
        })
        .await;
        assert_eq!(
            json(&rows),
            serde_json::json!({"extensions": [
                {"name": "up", "from": "0.1.0", "to": "0.2.0", "status": "upgraded"},
                {"name": "same", "from": "0.1.0", "to": "0.1.0", "status": "current"},
                {"name": "no", "from": "0.1.0", "status": "refused", "detail": "needs a person"},
                {"name": "bad", "from": "0.1.0", "status": "failed", "detail": "boom"},
            ]})
        );
        assert_eq!(
            summary(&rows),
            "extensions: 1 upgraded, 1 current, 1 refused, 1 failed"
        );
    }
}
