//! `agent.install-checks` (platform RFC 0172): puts one block of monitor checks from core
//! into the production agent's checks file, between its `# BEGIN <block>` and
//! `# END <block>` lines, and restarts the agent. It replaces the hand-written
//! `install-contract-checks-server.sh`.
//!
//! The block name fixes the source file in the core checkout ([`SOURCES`]); the ask never
//! carries a path. Steps, each refusing before anything is written when it fails:
//!
//! 1. `git fetch origin main`, then the commit must be an ancestor of `origin/main`.
//! 2. `git show <commit>:<source>` (checked against `git cat-file -s`, so a source over
//!    [`OUTPUT_CAP`] bytes is refused, never cut).
//! 3. The new file is built beside the old one, `iohr agent checks lint --file` reads it,
//!    and only a file that lints is moved into place; the old one is first copied to
//!    `checks.toml.bak-<UTC time>`.
//! 4. `systemctl --user daemon-reload` and `restart <service>.service`.
//! 5. `iohr agent status --json` until the agent is connected and holds exactly the
//!    entries the file declares (at most [`MAX_DECLARED_CHECKS`]). When it never does, the
//!    backup is put back and the agent restarted once more, and the run fails.
//!
//! What the local agent cannot say is whether the platform accepted each check: that report
//! is the Agent API's `checks.rejected`, read by the coordinator's post check, not here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use time::OffsetDateTime;
use time::macros::format_description;

use super::{Action, ArgvRunner, OUTPUT_CAP, Tool, said, strings, user_restart_calls};

/// The platform's limit on declared checks (core `monitor_class::MAX_DECLARED_CHECKS`).
pub(crate) const MAX_DECLARED_CHECKS: usize = 100;

/// Each block and its source in the core checkout: the catalog's `block` enum.
pub(crate) const SOURCES: [(&str, &str); 3] = [
    ("contract", "devops/checks/generated/agent.toml"),
    ("gates", "devops/gates/gate-checks.toml"),
    ("status-components", "devops/checks/status-components.toml"),
];

/// The agent service when the ask names none.
pub(crate) const DEFAULT_SERVICE: &str = "iohr-agent";

/// How many times the post check reads the agent's status, one [`ArgvRunner::settle`] apart.
const STATUS_TRIES: u32 = 12;

/// The source file of `block`, `None` for a block outside the catalog.
pub(crate) fn source_of(block: &str) -> Option<&'static str> {
    SOURCES.iter().find(|(b, _)| *b == block).map(|(_, p)| *p)
}

/// Whether `s` is one of the catalog's blocks.
pub(crate) fn is_block(s: &str) -> bool {
    source_of(s).is_some()
}

/// Whether `line` is the first line of `block`: `# BEGIN <block>`, then nothing or a space
/// and a note (a block of another name that starts the same is not it).
fn is_begin(line: &str, block: &str) -> bool {
    line.strip_prefix("# BEGIN ")
        .and_then(|rest| rest.strip_prefix(block))
        .is_some_and(|tail| tail.is_empty() || tail.starts_with(' '))
}

fn is_end(line: &str, block: &str) -> bool {
    line.trim_end() == format!("# END {block}")
}

/// `current` with `block` put in: the lines from its `# BEGIN` to its `# END` replaced by
/// `source` between fresh marker lines (the BEGIN line names the source and the commit),
/// or appended after a blank line when `current` has no such block.
///
/// # Errors
/// A source that has marker lines of its own, a block that begins twice or never ends.
pub(crate) fn put_block(
    current: &str,
    block: &str,
    path: &str,
    commit: &str,
    source: &str,
) -> Result<String, String> {
    if source
        .lines()
        .any(|l| l.starts_with("# BEGIN ") || l.starts_with("# END "))
    {
        return Err(format!(
            "refused: {path} has BEGIN or END marker lines of its own"
        ));
    }
    let mut fresh = format!("# BEGIN {block} (from core {path} at {commit})\n");
    fresh.push_str(source.trim_end());
    fresh.push_str("\n# END ");
    fresh.push_str(block);
    fresh.push('\n');
    let lines: Vec<&str> = current.split_inclusive('\n').collect();
    let begins: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_begin(l.trim_end(), block))
        .map(|(i, _)| i)
        .collect();
    match begins[..] {
        [] => {
            let mut out = current.to_owned();
            if !out.is_empty() {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
            out.push_str(&fresh);
            Ok(out)
        }
        [start] => {
            let end = lines[start..]
                .iter()
                .position(|l| is_end(l, block))
                .map(|n| start + n)
                .ok_or_else(|| {
                    format!("refused: the file's # BEGIN {block} has no # END {block}")
                })?;
            let mut out: String = lines[..start].concat();
            out.push_str(&fresh);
            out.push_str(&lines[end + 1..].concat());
            Ok(out)
        }
        _ => Err(format!(
            "refused: the file has more than one # BEGIN {block}"
        )),
    }
}

/// How many `[[check]]` and `[[refuse]]` entries `text` declares: what the agent counts.
pub(crate) fn declared(text: &str) -> usize {
    text.lines()
        .filter(|l| matches!(l.trim_end(), "[[check]]" | "[[refuse]]"))
        .count()
}

/// The backup's name: `checks.toml.bak-<UTC time>` beside the file.
fn backup_path(file: &Path, now: OffsetDateTime) -> Result<PathBuf, String> {
    let stamp = now
        .format(format_description!(
            "[year][month][day]T[hour][minute][second]Z"
        ))
        .map_err(|e| format!("time: {e}"))?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the checks file has no name")?;
    Ok(file.with_file_name(format!("{name}.bak-{stamp}")))
}

/// `from` copied over `to` in one rename, with `from`'s permissions (the agent's file is
/// private): a reader sees the old file or the new, never half.
fn replace(from: &Path, to: &Path) -> Result<(), String> {
    let tmp = to.with_file_name(format!(
        ".{}.new-{}",
        to.file_name().and_then(|n| n.to_str()).unwrap_or("checks"),
        std::process::id()
    ));
    let put = || -> std::io::Result<()> {
        std::fs::copy(from, &tmp)?;
        std::fs::rename(&tmp, to)
    };
    put().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not write the checks file: {}", e.kind())
    })
}

/// What `git` printed, or why the call failed (redacted).
async fn git(tools: &impl ArgvRunner, argv: &[&str]) -> Result<String, String> {
    let argv: Vec<String> = argv.iter().map(|a| (*a).to_owned()).collect();
    let out = tools.run(Tool::Git, &argv).await?;
    if out.success {
        Ok(out.stdout)
    } else {
        Err(format!("git {}: failed\n{}", argv[0], said(&out))
            .trim_end()
            .to_owned())
    }
}

/// The source of `block` at `commit`, read from the core checkout, once the commit is
/// known to be on `origin/main`.
async fn read_source(
    tools: &impl ArgvRunner,
    block: &str,
    commit: &str,
) -> Result<(&'static str, String), String> {
    let path = source_of(block).ok_or("block: not a block of the catalog")?;
    git(tools, &["fetch", "--quiet", "origin", "main"]).await?;
    let out = tools
        .run(
            Tool::Git,
            &strings(["merge-base", "--is-ancestor", commit, "origin/main"]),
        )
        .await?;
    if !out.success {
        return Err(format!(
            "refused: commit {} is not on origin/main of the core checkout",
            &commit[..12]
        ));
    }
    let spec = format!("{commit}:{path}");
    let size = git(tools, &["cat-file", "-s", &spec]).await?;
    let size: usize = size
        .trim()
        .parse()
        .map_err(|_| format!("refused: git did not say the size of {path}"))?;
    if size > OUTPUT_CAP {
        return Err(format!(
            "refused: {path} is {size} bytes, over the {OUTPUT_CAP} the runner reads"
        ));
    }
    let source = git(tools, &["show", &spec]).await?;
    if source.len() != size {
        return Err(format!(
            "refused: {path} was read short ({} of {size} bytes)",
            source.len()
        ));
    }
    if declared(&source) == 0 {
        return Err(format!(
            "refused: {path} at {} declares no check",
            &commit[..12]
        ));
    }
    Ok((path, source))
}

/// `iohr agent status --json`: whether the agent is connected and how many entries it
/// holds, or why it cannot be read yet.
async fn agent_status(tools: &impl ArgvRunner) -> Result<(bool, usize), String> {
    let out = tools
        .run(Tool::Iohr, &strings(["agent", "status", "--json"]))
        .await?;
    if !out.success {
        return Err("the agent does not answer yet".to_owned());
    }
    let v: Value = serde_json::from_str(&out.stdout)
        .map_err(|_| "the agent's status is not JSON".to_owned())?;
    let connected = v.get("connected").and_then(Value::as_bool).unwrap_or(false);
    let held = v
        .pointer("/policy/checks")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("the agent's status does not say how many checks it holds")?;
    Ok((connected, held))
}

/// Waits for the restarted agent to be connected and to hold `want` entries.
async fn settled(tools: &impl ArgvRunner, want: usize) -> Result<(), String> {
    let mut last = "the agent did not answer".to_owned();
    for _ in 0..STATUS_TRIES {
        tools.settle().await;
        match agent_status(tools).await {
            Ok((true, held)) if held == want => return Ok(()),
            Ok((connected, held)) => {
                last = format!(
                    "the agent is {} and holds {held} of {want} checks",
                    if connected {
                        "connected"
                    } else {
                        "not connected"
                    }
                );
            }
            Err(why) => last = why,
        }
    }
    Err(last)
}

/// Carries out `agent.install-checks`; the answer says what was done.
///
/// # Errors
/// A refusal before the file is touched (commit, source, lint), or a failed restart or
/// post check, after the backup is put back.
pub(crate) async fn install(action: &Action, tools: &impl ArgvRunner) -> Result<String, String> {
    let Action::InstallChecks {
        block,
        commit,
        service,
    } = action
    else {
        return Err("not agent.install-checks".to_owned());
    };
    let file = tools.agent_checks_file()?;
    let (path, source) = read_source(tools, block, commit).await?;
    let current = std::fs::read_to_string(&file)
        .map_err(|e| format!("could not read the checks file: {}", e.kind()))?;
    let next = put_block(&current, block, path, commit, &source)?;
    let want = declared(&next);
    if want > MAX_DECLARED_CHECKS {
        return Err(format!(
            "refused: the file would declare {want} checks, over the platform's {MAX_DECLARED_CHECKS}"
        ));
    }
    // Built beside the file and linted there: only a file that lints replaces the agent's.
    let candidate = file.with_file_name(format!(".checks.toml.lint-{}", std::process::id()));
    std::fs::write(&candidate, &next)
        .map_err(|e| format!("could not write the checks file: {}", e.kind()))?;
    let lint = tools
        .run(
            Tool::Iohr,
            &[
                "agent".to_owned(),
                "checks".to_owned(),
                "lint".to_owned(),
                "--file".to_owned(),
                candidate.to_string_lossy().into_owned(),
            ],
        )
        .await;
    let lint = match lint {
        Ok(out) if out.success => Ok(()),
        Ok(out) => Err(format!(
            "iohr agent checks lint: failed, the checks file is unchanged\n{}",
            said(&out)
        )
        .trim_end()
        .to_owned()),
        Err(why) => Err(why),
    };
    if let Err(why) = lint {
        let _ = std::fs::remove_file(&candidate);
        return Err(why);
    }
    let backup = backup_path(&file, OffsetDateTime::now_utc())?;
    std::fs::copy(&file, &backup)
        .map_err(|e| format!("could not back up the checks file: {}", e.kind()))?;
    let moved = replace(&candidate, &file);
    let _ = std::fs::remove_file(&candidate);
    moved?;
    let unit = format!("{service}.service");
    let outcome = async {
        for (tool, argv) in user_restart_calls(&unit) {
            let out = tools.run(tool, &argv).await?;
            if !out.success {
                return Err(format!("systemctl {}: failed\n{}", argv[1], said(&out))
                    .trim_end()
                    .to_owned());
            }
        }
        settled(tools, want).await
    }
    .await;
    let backup_name = backup
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("the backup");
    if let Err(why) = outcome {
        // Put back what ran before, and start it again.
        let back = replace(&backup, &file);
        let mut restart = String::new();
        if back.is_ok() {
            for (tool, argv) in user_restart_calls(&unit) {
                if tools.run(tool, &argv).await.is_err() {
                    "; it did not restart either".clone_into(&mut restart);
                    break;
                }
            }
        }
        return Err(match back {
            Ok(()) => format!("{why}; the old checks file is back from {backup_name}{restart}"),
            Err(e) => {
                format!("{why}; and the old file could not be put back ({e}): {backup_name} has it")
            }
        });
    }
    Ok(format!(
        "{block}: {want} checks in {} at {}, {unit} restarted and connected (backup {backup_name})",
        file.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("checks.toml"),
        &commit[..12]
    ))
}

/// The pause between two status reads.
pub(crate) const SETTLE: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests;
