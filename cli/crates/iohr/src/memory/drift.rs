//! `iohr memory drift`: where what the memories say may no longer match the code.
//!
//! The code check asks git, for each memory with path refs, whether those paths changed after
//! the memory was last confirmed (`updated`), and whether they have changes not committed yet.
//! It reads; it never changes a file. A folder that is not a git repository is reported as
//! skipped, not as clean. The service check is the sync plan ([`super::remote::plan`]) read
//! without acting.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

use super::check::{command_of, inside};
use super::store::Entry;
use crate::front;

/// One memory whose refs moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Drift {
    pub id: String,
    pub rule: &'static str,
    pub path: String,
    pub message: String,
}

/// The result of comparing memories with the code.
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub drifts: Vec<Drift>,
    /// Why the check did not run, when it did not.
    pub skipped: Option<String>,
}

/// Seconds a git call may take before it is stopped (a hung network disk never stalls a card).
const GIT_SECS: u64 = 5;

/// One read-only git call in `root`: no pager, no file-system monitor, no optional locks (a
/// status never writes the index), never a prompt, a time limit. None when git cannot run,
/// exits non-zero or takes too long. The one runner of `iohr ask`'s git facts and the drift
/// check.
pub(crate) fn git(root: &Path, args: &[&str]) -> Option<String> {
    use std::io::Read as _;
    use std::time::{Duration, Instant};
    let mut child = Command::new("git")
        .args([
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
        ])
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread so a large output never fills the pipe while this waits.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(GIT_SECS);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
        }
    };
    let bytes = reader.join().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Whether `root` is inside a git work tree.
#[must_use]
pub fn is_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|s| s.trim() == "true")
}

/// The commit times (unix seconds) that touched `path`, newest first, at most 200.
fn commits(root: &Path, path: &str) -> Vec<i64> {
    git(root, &["log", "--format=%ct", "-n", "200", "--", path])
        .map(|s| s.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// Whether `path` has changes git has not committed.
fn uncommitted(root: &Path, path: &str) -> bool {
    git(root, &["status", "--porcelain", "--", path]).is_some_and(|s| !s.trim().is_empty())
}

/// Compare each memory's path refs with git history since the memory was last updated.
#[must_use]
pub fn code(entries: &[Entry], root: &Path) -> Report {
    let mut report = Report::default();
    if !is_repo(root) {
        report.skipped = Some("not a git repository, so there is no history to compare".into());
        return report;
    }
    for e in entries {
        let Some(since) = front::unix_of(&e.updated) else {
            continue;
        };
        for r in &e.refs {
            if command_of(r).is_some() || !inside(r) || !root.join(r).exists() {
                continue;
            }
            let after = commits(root, r).into_iter().filter(|t| *t > since).count();
            if after > 0 {
                report.drifts.push(Drift {
                    id: e.id.clone(),
                    rule: "ref-changed",
                    path: r.clone(),
                    message: format!(
                        "{after} commit{} since it was last confirmed ({})",
                        if after == 1 { "" } else { "s" },
                        e.updated
                    ),
                });
            }
            if uncommitted(root, r) {
                report.drifts.push(Drift {
                    id: e.id.clone(),
                    rule: "ref-uncommitted",
                    path: r.clone(),
                    message: "has changes that are not committed yet".to_owned(),
                });
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::Kind;

    fn run(root: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {args:?}");
    }

    fn commit_at(root: &Path, msg: &str, date: &str) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                msg,
            ])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("git")
            .success();
        assert!(ok);
    }

    fn memory(refs: &[&str], updated: &str) -> Entry {
        let mut e = Entry::new(
            "Queries live in db.rs.",
            Kind::Fact,
            "why",
            refs.iter().map(|s| (*s).to_owned()).collect(),
            false,
        );
        e.created = updated.into();
        e.updated = updated.into();
        e
    }

    #[test]
    fn a_ref_changed_after_the_memory_drifts_and_one_before_does_not() {
        let t = tempfile::tempdir().expect("tempdir");
        let root = t.path();
        run(root, &["init", "-q"]);
        std::fs::write(root.join("a.rs"), "1").expect("write");
        std::fs::write(root.join("b.rs"), "1").expect("write");
        run(root, &["add", "a.rs", "b.rs"]);
        commit_at(root, "first", "2026-01-01T00:00:00Z");
        std::fs::write(root.join("a.rs"), "2").expect("write");
        run(root, &["add", "a.rs"]);
        commit_at(root, "second", "2026-03-01T00:00:00Z");
        let e = memory(&["a.rs", "b.rs"], "2026-02-01T00:00:00Z");
        let r = code(std::slice::from_ref(&e), root);
        assert!(r.skipped.is_none());
        assert_eq!(r.drifts.len(), 1, "{:?}", r.drifts);
        assert_eq!(
            (r.drifts[0].id.as_str(), r.drifts[0].path.as_str()),
            (e.id.as_str(), "a.rs")
        );
        assert_eq!(r.drifts[0].rule, "ref-changed");
    }

    #[test]
    fn an_uncommitted_change_to_a_ref_drifts() {
        let t = tempfile::tempdir().expect("tempdir");
        let root = t.path();
        run(root, &["init", "-q"]);
        std::fs::write(root.join("a.rs"), "1").expect("write");
        run(root, &["add", "a.rs"]);
        commit_at(root, "first", "2026-01-01T00:00:00Z");
        std::fs::write(root.join("a.rs"), "2").expect("write");
        let r = code(&[memory(&["a.rs"], "2026-02-01T00:00:00Z")], root);
        assert!(
            r.drifts.iter().any(|d| d.rule == "ref-uncommitted"),
            "{:?}",
            r.drifts
        );
    }

    #[test]
    fn a_folder_without_git_is_skipped_not_clean() {
        let t = tempfile::tempdir().expect("tempdir");
        let r = code(&[memory(&["a.rs"], "2026-02-01T00:00:00Z")], t.path());
        assert!(r.skipped.is_some());
        assert_eq!(r.drifts, Vec::new());
    }
}
