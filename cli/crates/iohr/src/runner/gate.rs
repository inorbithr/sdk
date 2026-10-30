//! The check a build job's PR is held to (RFC 0120, the Checked gate): what the runner
//! runs in the job's checkout before it signs a gate report.
//!
//! The runner fetches `origin/main`, refuses a checkout that is not merged with it or
//! that has uncommitted changes (the report would not describe the PR's head), runs the
//! check with the scrubbed environment under a time cap, and keeps only the command,
//! its exit code, the head, main's commit and the SHA-256 of the output. The output
//! itself never leaves the machine; of a failing check only a short redacted tail
//! ([`TAIL_BYTES`]) is kept, for the runner's own log and `iohr runner gate-report`, so a
//! job held at Checked says why. The signed report carries the hash only.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;

use super::claude::hex;
use super::env;
use super::exec::GroupGuard;
use super::service::{GateAnswer, GateReport, Service, ServiceError};
use super::workspace::redact;

/// The check a build job's PR is held to when nothing says otherwise.
pub(crate) const DEFAULT_COMMAND: &str = "mise run ci:changed";
/// How long that check may run: 30 minutes.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 1800;
/// How much of a failing check's output is kept: its last 4 KB.
pub(crate) const TAIL_BYTES: usize = 4096;

/// The branch the head must contain.
const MAIN: &str = "origin/main";
/// How long a `git` call may take.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// What the check showed, ready to be a `GateReport` with the lease's ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Checked {
    pub(crate) command: String,
    pub(crate) exit_code: i32,
    pub(crate) head: String,
    pub(crate) main_commit: String,
    pub(crate) output_sha256: String,
    /// The last [`TAIL_BYTES`] of the output of a failing check, whole lines only and
    /// every line through [`redact`]; empty when the check passed. Never signed or sent.
    pub(crate) output_tail: String,
}

/// Why no report can be made. Nothing here is a result: a job with no report holds at
/// Checked with its reason, which is the honest answer.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Skip {
    #[error("the command is empty or over 500 characters")]
    BadCommand,
    #[error("git failed: {0}")]
    Git(String),
    #[error("no GitHub access to fetch main: {0}")]
    Access(String),
    #[error("the checkout has uncommitted changes; the check would not describe its head")]
    Dirty,
    #[error(
        "the head {head} does not contain origin/main ({main}): merge main into the branch, then check again"
    )]
    NotMerged { head: String, main: String },
    #[error("the check did not finish within {0} seconds")]
    TimedOut(u64),
    #[error("the check could not run: {0}")]
    Spawn(String),
    #[error("the head changed while the check ran")]
    Moved,
}

/// What one `git` call printed: its exit code, its stdout as it is, and the last line of
/// its stderr (for a log line; git prints no credential there, and the caller redacts
/// anything shaped like a token anyway).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitOut {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr_tail: String,
}

/// The git subcommand in `args`, past any `-c key=value`: for an error line.
fn subcommand<'a>(args: &[&'a str]) -> &'a str {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if *a == "-c" {
            it.next();
        } else if !a.starts_with('-') {
            return a;
        }
    }
    ""
}

/// `git <args>` in `repo` under `limit`: never a prompt, never another repository named
/// by the environment, nothing on stdin. The runner's own environment, which holds no
/// GitHub token (a call that reaches GitHub goes through [`git_in_with`]); the model's
/// scrubbed one is not involved.
pub(crate) async fn git_in(repo: &Path, args: &[&str], limit: Duration) -> Result<GitOut, Skip> {
    git_in_with(repo, args, limit, &[]).await
}

/// [`git_in`] with `extra` set on this one child only (the job's token for the
/// credential helper, `access::TOKEN_VAR`); nothing else sees it.
pub(crate) async fn git_in_with(
    repo: &Path,
    args: &[&str],
    limit: Duration,
    extra: &[(&str, &str)],
) -> Result<GitOut, Skip> {
    let mut c = Command::new("git");
    c.args(args)
        .current_dir(repo)
        .envs(extra.iter().copied())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(limit, c.output())
        .await
        .map_err(|_| Skip::Git(format!("git {} took too long", subcommand(args))))?
        .map_err(|e| Skip::Git(e.to_string()))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    Ok(GitOut {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr_tail: stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .chars()
            .take(300)
            .collect(),
    })
}

/// `git <args>` in `repo`: the exit code and trimmed stdout.
async fn git(repo: &Path, args: &[&str]) -> Result<(i32, String), Skip> {
    let o = git_in(repo, args, GIT_TIMEOUT).await?;
    Ok((o.code, o.stdout.trim().to_owned()))
}

/// Where the check's `origin/main` comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MainFrom {
    /// `git fetch origin main` in the checkout, as the host's own git configuration says,
    /// with no token: a checkout the runner does not own, or `iohr runner gate-report`
    /// run by a person on their machine.
    Checkout,
    /// The runner's mirror, already fetched behind its guard
    /// ([`super::workspace::Workspaces::fetch_main`]): the checkout is a linked worktree
    /// of it and shares its refs. Nothing is fetched in the checkout, whose configuration
    /// the turn can write.
    Mirror,
}

/// `git fetch origin main` in `repo`, with no token ([`MainFrom::Checkout`]); a failure
/// says git's last stderr line, redacted.
async fn fetch_main(repo: &Path) -> Result<(), Skip> {
    let o = git_in(repo, &["fetch", "--quiet", "origin", "main"], GIT_TIMEOUT).await?;
    if o.code == 0 {
        Ok(())
    } else {
        Err(Skip::Git(format!(
            "git fetch exited {}: {}",
            o.code,
            redact(&o.stderr_tail)
        )))
    }
}

async fn git_ok(repo: &Path, args: &[&str]) -> Result<String, Skip> {
    match git(repo, args).await? {
        (0, out) => Ok(out),
        (code, _) => Err(Skip::Git(format!(
            "git {} exited {code}",
            args.first().unwrap_or(&"")
        ))),
    }
}

/// The command through the platform's shell, its stderr joined to its stdout.
fn shell(command: &str) -> Command {
    #[cfg(windows)]
    #[allow(unused_mut)] // only the unix arm mutates it (process_group below)
    let mut c = {
        // `raw_arg`: the standard library would quote the whole command, and cmd.exe
        // then takes the quoted text for one program name as soon as it holds `&`, `|`
        // or `>`. `/S` strips exactly the outer pair, and the parentheses put the
        // redirect on the whole command, not only its last part.
        let mut c = Command::new("cmd");
        c.raw_arg(format!("/S /C \"({command}) 2>&1\""));
        c
    };
    #[cfg(not(windows))]
    let mut c = {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", &format!("exec 2>&1; {command}")]);
        c
    };
    // Its own process group, so a timeout takes the whole tree with it.
    #[cfg(unix)]
    c.process_group(0);
    c
}

/// Fetches main (where `main` says), checks `repo` is clean and contains it, runs
/// `command` and returns what to report. `env` is the runner's own environment; it is
/// scrubbed here. No call here carries a token: a checkout the runner owns has main
/// fetched in the runner's mirror first ([`MainFrom::Mirror`]).
pub(crate) async fn check(
    repo: &Path,
    command: &str,
    limit: Duration,
    parent_env: Vec<(OsString, OsString)>,
    main: MainFrom,
) -> Result<Checked, Skip> {
    let command = command.trim();
    if command.is_empty() || command.chars().count() > 500 {
        return Err(Skip::BadCommand);
    }
    if main == MainFrom::Checkout {
        fetch_main(repo).await?;
    }
    if !git_ok(repo, &["status", "--porcelain"]).await?.is_empty() {
        return Err(Skip::Dirty);
    }
    let head = git_ok(repo, &["rev-parse", "HEAD"]).await?;
    let main_commit = git_ok(repo, &["rev-parse", MAIN]).await?;
    match git(repo, &["merge-base", "--is-ancestor", MAIN, "HEAD"]).await? {
        (0, _) => {}
        (1, _) => {
            return Err(Skip::NotMerged {
                head,
                main: main_commit,
            });
        }
        (code, _) => return Err(Skip::Git(format!("git merge-base exited {code}"))),
    }
    let Ran {
        exit_code,
        output_sha256,
        output_tail,
    } = run(repo, command, limit, parent_env).await?;
    if git_ok(repo, &["rev-parse", "HEAD"]).await? != head {
        return Err(Skip::Moved);
    }
    Ok(Checked {
        command: command.to_owned(),
        exit_code,
        head,
        main_commit,
        output_sha256,
        output_tail,
    })
}

/// Whose lease the report is signed for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Held<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) lease_id: &'a str,
    pub(crate) fencing_token: i64,
    pub(crate) runner_id: &'a str,
}

/// What the check showed, as the report the service is sent.
pub(crate) fn report_of(held: Held<'_>, checked: &Checked) -> GateReport {
    GateReport {
        job_id: held.job_id.to_owned(),
        lease_id: held.lease_id.to_owned(),
        fencing_token: held.fencing_token,
        runner_id: held.runner_id.to_owned(),
        command: checked.command.clone(),
        exit_code: checked.exit_code,
        head: checked.head.clone(),
        main_commit: checked.main_commit.clone(),
        output_sha256: checked.output_sha256.clone(),
    }
}

/// Sends a report. An older coordinator has no such call: that is `Ok(None)`, not an
/// error (the caller says so). Every other refusal is the error.
pub(crate) async fn send<S: Service>(
    svc: &S,
    report: &GateReport,
) -> Result<Option<GateAnswer>, ServiceError> {
    match svc.submit_gate_report(report).await {
        Ok(a) => Ok(Some(a)),
        Err(e) if e.is_missing() => Ok(None),
        Err(e) => Err(e),
    }
}

/// What one run of the check left: its exit code, the hash of all its output, and the
/// tail of that output when it failed.
struct Ran {
    exit_code: i32,
    output_sha256: String,
    output_tail: String,
}

/// The last [`TAIL_BYTES`] of `bytes` as redacted whole lines: a cut that lands inside a
/// line drops that partial line first (half a token would slip past [`redact`]).
fn tail_of(bytes: &[u8]) -> String {
    let cut = bytes.len() > TAIL_BYTES;
    let mut text =
        String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(TAIL_BYTES)..]).into_owned();
    if cut && let Some(nl) = text.find('\n') {
        text.drain(..=nl);
    }
    text.lines()
        .map(|l| redact(l.trim_end_matches('\r')))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_owned()
}

/// Runs `command` in `repo` with the scrubbed environment; streams its output into a
/// hash and a bounded buffer of its last bytes, and returns its exit code, the hash and,
/// when it failed, the redacted tail.
async fn run(
    repo: &Path,
    command: &str,
    limit: Duration,
    parent_env: Vec<(OsString, OsString)>,
) -> Result<Ran, Skip> {
    let mut c = shell(command);
    c.current_dir(repo)
        .env_clear()
        .envs(env::scrub(parent_env))
        .envs(env::contained())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = c.spawn().map_err(|e| Skip::Spawn(e.to_string()))?;
    let mut group = GroupGuard::new(child.id());
    let mut stdout = child.stdout.take();
    let work = async {
        let mut hash = Sha256::new();
        let mut tail: Vec<u8> = Vec::new();
        if let Some(out) = stdout.as_mut() {
            let mut buf = vec![0_u8; 16 * 1024];
            loop {
                match out.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        hash.update(&buf[..n]);
                        tail.extend_from_slice(&buf[..n]);
                        // Bounded: at twice the tail, keep the last part.
                        if tail.len() > 2 * TAIL_BYTES {
                            tail.drain(..tail.len() - TAIL_BYTES);
                        }
                    }
                }
            }
        }
        let status = child.wait().await.map_err(|e| Skip::Spawn(e.to_string()))?;
        let exit_code = status.code().unwrap_or(-1);
        Ok::<_, Skip>(Ran {
            exit_code,
            output_sha256: hex(&hash.finalize()),
            output_tail: if exit_code == 0 {
                String::new()
            } else {
                tail_of(&tail)
            },
        })
    };
    let res = tokio::time::timeout(limit, work).await;
    match res {
        Ok(r) => {
            group.disarm();
            r
        }
        // The guard kills the process group as it drops.
        Err(_) => Err(Skip::TimedOut(limit.as_secs())),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;
    use std::fmt::Write as _;
    use std::path::PathBuf;

    pub(crate) fn sh(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    pub(crate) fn write(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    /// An origin with one commit on main, and a clone on a branch one commit ahead of it.
    pub(crate) fn repos() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let origin = root.path().join("origin");
        let work = root.path().join("work");
        std::fs::create_dir_all(&origin).unwrap();
        sh(&origin, &["init", "--bare", "--quiet"]);
        sh(root.path(), &["clone", "--quiet", "origin", "work"]);
        write(&work, "a.txt", "a\n");
        sh(&work, &["add", "a.txt"]);
        sh(&work, &["commit", "--quiet", "-m", "main"]);
        sh(&work, &["push", "--quiet", "origin", "HEAD:main"]);
        sh(&work, &["checkout", "--quiet", "-b", "feature"]);
        write(&work, "b.txt", "b\n");
        sh(&work, &["add", "b.txt"]);
        sh(&work, &["commit", "--quiet", "-m", "feature"]);
        (root, origin, work)
    }

    fn parent() -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }

    const LIMIT: Duration = Duration::from_secs(20);

    /// One line on stdout, then one on stderr, in the platform's shell.
    #[cfg(not(windows))]
    const TWO_STREAMS: &str = "echo hello; echo oops 1>&2";
    #[cfg(not(windows))]
    const TWO_STREAMS_OUT: &[u8] = b"hello\noops\n";
    #[cfg(windows)]
    const TWO_STREAMS: &str = "echo hello& echo oops>&2";
    #[cfg(windows)]
    const TWO_STREAMS_OUT: &[u8] = b"hello\r\noops\r\n";

    /// Exits 1 when either credential reached the shell or the containment is missing
    /// (`gh` signed out, git with no credential helper), else prints `clean`. cmd.exe
    /// keeps an unset `%NAME%` as literal text, so the Windows form uses `if defined`.
    #[cfg(not(windows))]
    const SCRUBBED: &str = "test -z \"$GH_TOKEN$FOO_SECRET\" \
        && test \"$GH_CONFIG_DIR\" = /nonexistent/iohr-gh \
        && test \"$GIT_CONFIG_KEY_0=$GIT_CONFIG_VALUE_0\" = credential.helper= \
        && echo clean";
    #[cfg(not(windows))]
    const SCRUBBED_OUT: &[u8] = b"clean\n";
    #[cfg(windows)]
    const SCRUBBED: &str =
        "(if defined GH_TOKEN exit 1)& (if defined FOO_SECRET exit 1)& echo clean";
    #[cfg(windows)]
    const SCRUBBED_OUT: &[u8] = b"clean\r\n";

    #[tokio::test]
    async fn a_branch_merged_with_main_is_checked_and_reported_with_the_hash_of_the_output() {
        let (_root, _o, work) = repos();
        let got = check(&work, TWO_STREAMS, LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap();
        assert_eq!(got.exit_code, 0);
        assert_eq!(got.head, sh(&work, &["rev-parse", "HEAD"]));
        assert_eq!(got.main_commit, sh(&work, &["rev-parse", "origin/main"]));
        // stdout and stderr, in order, as one stream.
        assert_eq!(
            got.output_sha256,
            hex(&Sha256::digest(TWO_STREAMS_OUT)),
            "the output hash covers both streams"
        );
        assert_eq!(got.command, TWO_STREAMS);
    }

    #[tokio::test]
    async fn a_failing_check_is_reported_with_its_exit_code() {
        let (_root, _o, work) = repos();
        let got = check(&work, "exit 3", LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap();
        assert_eq!(got.exit_code, 3);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn a_failing_check_keeps_the_redacted_tail_of_its_output() {
        let (_root, _o, work) = repos();
        let cmd = "echo warning: too_many_lines; echo token ghp_abcdefghijklmnopqrstuvwxyz0123 leaked 1>&2; \
                   echo clone https://user:pw@github.com/o/r; exit 101";
        let got = check(&work, cmd, LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap();
        assert_eq!(got.exit_code, 101);
        assert_eq!(
            got.output_tail,
            "warning: too_many_lines\ntoken [redacted] leaked\nclone https://[redacted]@github.com/o/r"
        );
        assert!(!got.output_tail.contains("ghp_"));
        assert!(!got.output_tail.contains("pw@"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn a_passing_check_keeps_no_output() {
        let (_root, _o, work) = repos();
        let got = check(&work, "echo fine", LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap();
        assert_eq!((got.exit_code, got.output_tail.as_str()), (0, ""));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn a_long_output_keeps_only_its_last_whole_lines_within_four_kilobytes() {
        let (_root, _o, work) = repos();
        // 2000 numbered lines, far over the cap, the hash still covering all of it.
        let cmd = "i=1; while [ $i -le 2000 ]; do echo line-$i; i=$((i+1)); done; exit 1";
        let got = check(&work, cmd, LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap();
        assert!(
            got.output_tail.len() <= TAIL_BYTES,
            "{}",
            got.output_tail.len()
        );
        assert!(
            got.output_tail.len() > TAIL_BYTES - 16,
            "the tail fills the cap"
        );
        assert!(got.output_tail.ends_with("line-2000"));
        // Whole lines only: the first one is a complete `line-N`.
        let first = got.output_tail.lines().next().unwrap();
        assert!(
            first.starts_with("line-") && first[5..].parse::<u32>().is_ok(),
            "{first}"
        );
        let mut all = String::new();
        for i in 1..=2000 {
            writeln!(all, "line-{i}").unwrap();
        }
        assert_eq!(got.output_sha256, hex(&Sha256::digest(all.as_bytes())));
    }

    #[test]
    fn a_cut_inside_a_line_drops_that_partial_line_before_redacting() {
        // A token split by the cut would not look like one: its half line goes.
        let mut bytes = b"ghp_secrettokenpartxx".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', TAIL_BYTES - 20));
        bytes.extend_from_slice(b"\nwhole line\nlast\n");
        let t = tail_of(&bytes);
        assert_eq!(t, "whole line\nlast");
    }

    #[tokio::test]
    async fn a_head_that_lacks_a_newer_main_is_not_checked() {
        let (root, _o, work) = repos();
        // Someone else moves main.
        let other = root.path().join("other");
        sh(root.path(), &["clone", "--quiet", "origin", "other"]);
        write(&other, "c.txt", "c\n");
        sh(&other, &["add", "c.txt"]);
        sh(&other, &["commit", "--quiet", "-m", "more main"]);
        sh(&other, &["push", "--quiet", "origin", "HEAD:main"]);
        let err = check(&work, "true", LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap_err();
        let Skip::NotMerged { head, main } = err else {
            panic!("expected NotMerged, got {err:?}");
        };
        assert_eq!(head, sh(&work, &["rev-parse", "HEAD"]));
        assert_eq!(main, sh(&other, &["rev-parse", "HEAD"]));
        // After the merge it is checked, on main's new commit.
        sh(&work, &["merge", "--quiet", "--no-edit", "origin/main"]);
        let got = check(&work, "true", LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap();
        assert_eq!(got.main_commit, main);
        assert_eq!(got.exit_code, 0);
    }

    #[tokio::test]
    async fn uncommitted_changes_are_refused() {
        let (_root, _o, work) = repos();
        write(&work, "b.txt", "changed\n");
        assert_eq!(
            check(&work, "true", LIMIT, parent(), MainFrom::Checkout)
                .await
                .unwrap_err(),
            Skip::Dirty
        );
    }

    #[tokio::test]
    async fn the_check_gets_the_scrubbed_environment_only() {
        let (_root, _o, work) = repos();
        let mut env = parent();
        env.push(("GH_TOKEN".into(), "secret-value".into()));
        env.push(("FOO_SECRET".into(), "secret-value".into()));
        let got = check(&work, SCRUBBED, LIMIT, env, MainFrom::Checkout)
            .await
            .unwrap();
        assert_eq!(got.exit_code, 0);
        assert_eq!(got.output_sha256, hex(&Sha256::digest(SCRUBBED_OUT)));
    }

    #[tokio::test]
    async fn a_check_over_the_time_cap_is_stopped_and_makes_no_report() {
        let (_root, _o, work) = repos();
        let err = check(
            &work,
            "sleep 30",
            Duration::from_secs(1),
            parent(),
            MainFrom::Checkout,
        )
        .await
        .unwrap_err();
        assert_eq!(err, Skip::TimedOut(1));
    }

    #[tokio::test]
    async fn an_empty_or_long_command_is_refused_before_anything_runs() {
        let (_root, _o, work) = repos();
        assert_eq!(
            check(&work, "  ", LIMIT, parent(), MainFrom::Checkout)
                .await
                .unwrap_err(),
            Skip::BadCommand
        );
        assert_eq!(
            check(&work, &"x".repeat(501), LIMIT, parent(), MainFrom::Checkout)
                .await
                .unwrap_err(),
            Skip::BadCommand
        );
    }

    #[tokio::test]
    async fn a_directory_that_is_not_a_checkout_is_a_git_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            check(dir.path(), "true", LIMIT, parent(), MainFrom::Checkout).await,
            Err(Skip::Git(_))
        ));
    }

    /// A program planted in the checkout's configuration (the turn's check command can
    /// write it) never runs when main comes from the runner's mirror: nothing is fetched
    /// in the checkout. With [`MainFrom::Checkout`] the host's own fetch runs there, as
    /// before, with no token.
    #[cfg(unix)]
    #[tokio::test]
    async fn main_from_the_mirror_fetches_nothing_in_the_checkout() {
        use std::os::unix::fs::PermissionsExt as _;
        let (root, _o, work) = repos();
        let seen = root.path().join("saw.txt");
        let upload = root.path().join("upload.sh");
        std::fs::write(
            &upload,
            format!(
                "#!/bin/sh\necho \"token=${}\" >> '{}'\nexec git-upload-pack \"$@\"\n",
                crate::runner::access::TOKEN_VAR,
                seen.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&upload, std::fs::Permissions::from_mode(0o755)).unwrap();
        sh(
            &work,
            &[
                "config",
                "remote.origin.uploadpack",
                &upload.display().to_string(),
            ],
        );
        sh(
            &work,
            &["config", "core.sshCommand", &upload.display().to_string()],
        );
        let got = check(&work, "true", LIMIT, parent(), MainFrom::Mirror)
            .await
            .unwrap();
        assert_eq!(got.exit_code, 0);
        assert!(!seen.exists(), "a fetch ran in the checkout");
        // The host's own fetch in the checkout: the planted program runs, with no token.
        check(&work, "true", LIMIT, parent(), MainFrom::Checkout)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "token=\n");
    }

    /// The fetch in the checkout fails with git's reason and no credential, from an
    /// origin whose URL holds a token.
    #[tokio::test]
    async fn a_failed_fetch_says_why_without_the_token() {
        let (_root, _o, work) = repos();
        sh(
            &work,
            &[
                "remote",
                "set-url",
                "origin",
                "https://x-access-token:ghs_test@127.0.0.1:1/",
            ],
        );
        let Err(Skip::Git(why)) = check(&work, "true", LIMIT, parent(), MainFrom::Checkout).await
        else {
            panic!("expected a git failure");
        };
        assert!(why.starts_with("git fetch exited 128: "), "{why}");
        assert!(why.contains("127.0.0.1"), "{why}");
        assert!(
            !why.contains("ghs_") && !why.contains("x-access-token"),
            "{why}"
        );
    }
}
