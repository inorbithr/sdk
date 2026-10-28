//! The check a build job's PR is held to (RFC 0120, the Checked gate): what the runner
//! runs in the job's checkout before it signs a gate report.
//!
//! The runner fetches `origin/main`, refuses a checkout that is not merged with it or
//! that has uncommitted changes (the report would not describe the PR's head), runs the
//! check with the scrubbed environment under a time cap, and keeps only the command,
//! its exit code, the head, main's commit and the SHA-256 of the output. The output
//! itself never leaves the machine and is never kept here.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;

use super::access::Access;
use super::claude::hex;
use super::env;
use super::exec::GroupGuard;
use super::service::{GateAnswer, GateReport, Service, ServiceError};
use super::workspace::redact;

/// The check a build job's PR is held to when nothing says otherwise.
pub(crate) const DEFAULT_COMMAND: &str = "mise run ci:changed";
/// How long that check may run: 30 minutes.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 1800;

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

/// `git fetch origin main`, with the credential helper and the token's variable of
/// `access` on this one child only (the same two calls the workspaces make to GitHub,
/// [`Access::git_args`] and [`Access::git_env`]).
async fn fetch_main(repo: &Path, access: Option<&Access>) -> Result<(), Skip> {
    let pre: Vec<String> = access.map(|a| a.git_args().to_vec()).unwrap_or_default();
    let mut args: Vec<&str> = pre.iter().map(String::as_str).collect();
    args.extend(["fetch", "--quiet", "origin", "main"]);
    let env: Vec<(&str, &str)> = access.and_then(Access::git_env).into_iter().collect();
    let o = git_in_with(repo, &args, GIT_TIMEOUT, &env).await?;
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

/// Fetches main, checks `repo` is clean and contains it, runs `command` and returns what
/// to report. `env` is the runner's own environment; it is scrubbed here.
///
/// `access` is how the fetch signs in to GitHub: the lease's own token (or the host's
/// sign-in) on that one git call, as for every other call that reaches GitHub. `None`
/// leaves it to the host's own git configuration (`iohr runner gate-report`, run by a
/// person on their machine).
pub(crate) async fn check(
    repo: &Path,
    command: &str,
    limit: Duration,
    parent_env: Vec<(OsString, OsString)>,
    access: Option<&Access>,
) -> Result<Checked, Skip> {
    let command = command.trim();
    if command.is_empty() || command.chars().count() > 500 {
        return Err(Skip::BadCommand);
    }
    fetch_main(repo, access).await?;
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
    let (exit_code, output_sha256) = run(repo, command, limit, parent_env).await?;
    if git_ok(repo, &["rev-parse", "HEAD"]).await? != head {
        return Err(Skip::Moved);
    }
    Ok(Checked {
        command: command.to_owned(),
        exit_code,
        head,
        main_commit,
        output_sha256,
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

/// Runs `command` in `repo` with the scrubbed environment; streams its output into a
/// hash (nothing is kept) and returns its exit code with the hash.
async fn run(
    repo: &Path,
    command: &str,
    limit: Duration,
    parent_env: Vec<(OsString, OsString)>,
) -> Result<(i32, String), Skip> {
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
        if let Some(out) = stdout.as_mut() {
            let mut buf = vec![0_u8; 16 * 1024];
            loop {
                match out.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => hash.update(&buf[..n]),
                }
            }
        }
        let status = child.wait().await.map_err(|e| Skip::Spawn(e.to_string()))?;
        Ok::<_, Skip>((status.code().unwrap_or(-1), hex(&hash.finalize())))
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
        let got = check(&work, TWO_STREAMS, LIMIT, parent(), None)
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
        let got = check(&work, "exit 3", LIMIT, parent(), None).await.unwrap();
        assert_eq!(got.exit_code, 3);
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
        let err = check(&work, "true", LIMIT, parent(), None)
            .await
            .unwrap_err();
        let Skip::NotMerged { head, main } = err else {
            panic!("expected NotMerged, got {err:?}");
        };
        assert_eq!(head, sh(&work, &["rev-parse", "HEAD"]));
        assert_eq!(main, sh(&other, &["rev-parse", "HEAD"]));
        // After the merge it is checked, on main's new commit.
        sh(&work, &["merge", "--quiet", "--no-edit", "origin/main"]);
        let got = check(&work, "true", LIMIT, parent(), None).await.unwrap();
        assert_eq!(got.main_commit, main);
        assert_eq!(got.exit_code, 0);
    }

    #[tokio::test]
    async fn uncommitted_changes_are_refused() {
        let (_root, _o, work) = repos();
        write(&work, "b.txt", "changed\n");
        assert_eq!(
            check(&work, "true", LIMIT, parent(), None)
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
        let got = check(&work, SCRUBBED, LIMIT, env, None).await.unwrap();
        assert_eq!(got.exit_code, 0);
        assert_eq!(got.output_sha256, hex(&Sha256::digest(SCRUBBED_OUT)));
    }

    #[tokio::test]
    async fn a_check_over_the_time_cap_is_stopped_and_makes_no_report() {
        let (_root, _o, work) = repos();
        let err = check(&work, "sleep 30", Duration::from_secs(1), parent(), None)
            .await
            .unwrap_err();
        assert_eq!(err, Skip::TimedOut(1));
    }

    #[tokio::test]
    async fn an_empty_or_long_command_is_refused_before_anything_runs() {
        let (_root, _o, work) = repos();
        assert_eq!(
            check(&work, "  ", LIMIT, parent(), None).await.unwrap_err(),
            Skip::BadCommand
        );
        assert_eq!(
            check(&work, &"x".repeat(501), LIMIT, parent(), None)
                .await
                .unwrap_err(),
            Skip::BadCommand
        );
    }

    #[tokio::test]
    async fn a_directory_that_is_not_a_checkout_is_a_git_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            check(dir.path(), "true", LIMIT, parent(), None).await,
            Err(Skip::Git(_))
        ));
    }

    /// The fetch of main is where the check reaches GitHub: it gets the lease's token
    /// variable and the lease's `-c` arguments (hooks off, so a hook never inherits the
    /// token) on that one child, and the check's own command gets no token. The origin
    /// here is local; its `uploadpack` program records what the fetch's child saw, and a
    /// `reference-transaction` hook, which the fetch would run, records itself.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_fetch_of_main_gets_the_leases_arguments_and_token_only() {
        use crate::runner::access::{TOKEN_VAR, tests::token_for};
        use std::os::unix::fs::PermissionsExt as _;
        let script = |path: &Path, body: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let (root, _o, work) = repos();
        let seen = root.path().join("saw.txt");
        let hooked = root.path().join("hook-ran.txt");
        script(
            &root.path().join("upload.sh"),
            &format!(
                "echo \"token=${TOKEN_VAR}\" >> '{}'\nexec git-upload-pack \"$@\"",
                seen.display()
            ),
        );
        script(
            &work.join(".git/hooks/reference-transaction"),
            &format!("echo ran >> '{}'", hooked.display()),
        );
        sh(
            &work,
            &[
                "config",
                "remote.origin.uploadpack",
                &root.path().join("upload.sh").display().to_string(),
            ],
        );
        // A new commit on main, so the fetch has a reference to update.
        let newer = |name: &str| {
            let other = root.path().join(name);
            sh(root.path(), &["clone", "--quiet", "origin", name]);
            write(&other, "n.txt", name);
            sh(&other, &["add", "n.txt"]);
            sh(&other, &["commit", "--quiet", "-m", name]);
            sh(&other, &["push", "--quiet", "origin", "HEAD:main"]);
        };
        newer("one");
        // The check's command prints whether the token's variable reached it.
        let command = format!("test -z \"${TOKEN_VAR}\"");
        let access = token_for("acme/app");
        // Not merged with the new main, so no command runs; the fetch is what is seen.
        let err = check(&work, &command, LIMIT, parent(), Some(&access))
            .await
            .unwrap_err();
        assert!(matches!(err, Skip::NotMerged { .. }), "{err:?}");
        let saw = std::fs::read_to_string(&seen).unwrap();
        assert_eq!(
            saw,
            format!("token={}\n", crate::runner::access::tests::FAKE)
        );
        assert!(
            !hooked.exists(),
            "a hook ran in the fetch that had the token"
        );
        // With the main merged, the check's command runs and sees no token.
        sh(&work, &["merge", "--quiet", "--no-edit", "origin/main"]);
        let got = check(&work, &command, LIMIT, parent(), Some(&access))
            .await
            .unwrap();
        assert_eq!(
            got.exit_code, 0,
            "the check's command saw the token variable"
        );
        // No access: the fetch is left to the host's own configuration, no token and
        // the host's hooks.
        std::fs::remove_file(&seen).unwrap();
        newer("two");
        check(&work, "true", LIMIT, parent(), None)
            .await
            .unwrap_err();
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "token=\n");
        assert!(
            hooked.exists(),
            "the fetch without access has the host's hooks"
        );
    }

    #[tokio::test]
    async fn a_failed_fetch_says_why_without_the_token() {
        let (_root, _o, work) = repos();
        sh(
            &work,
            &["remote", "set-url", "origin", "/nonexistent/origin.git"],
        );
        let access = crate::runner::access::tests::token_for("acme/app");
        let Err(Skip::Git(why)) = check(&work, "true", LIMIT, parent(), Some(&access)).await else {
            panic!("expected a git failure");
        };
        assert!(why.starts_with("git fetch exited"), "{why}");
        assert!(!why.contains("ghs_"), "{why}");
    }
}
