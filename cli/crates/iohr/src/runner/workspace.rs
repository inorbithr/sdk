//! The job's checkout, owned by the runner (RFC 0147 slice A, PRD 0152, ADR 0141).
//!
//! A job with a repository runs in `<workdir>/checkouts/<job_id>` on the branch
//! `job/<job_id>`. The runner makes that checkout before it accepts the lease: a linked
//! worktree of one bare mirror per repository (`<workdir>/../mirrors/<owner>/<repo>.git`,
//! fetched, never cloned again), started from `origin/main`, or from `origin/job/<id>`
//! when an earlier attempt pushed it. A retry finds the checkout and reuses it as it is:
//! the same job's work, never another's, and nothing is ever deleted. A checkout that
//! cannot be made means the lease is not accepted.
//!
//! After a shaped reply the runner (never the model, which has no shell) commits what the
//! turn changed, by name, pushes `job/<id>` without force, and opens the pull request, or
//! updates it when one is open. git and `gh` run with the runner's own environment, so
//! the host's existing sign-in applies; the model's environment stays scrubbed and no
//! token passes through here.

use std::fmt::{self, Write as _};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt as _;
use tokio::process::Command;

use super::claude::Reply;
use super::gate::{GitOut, git_in};

/// The branch every job starts from and its pull request goes into.
pub(crate) const BASE: &str = "main";
/// Where GitHub repositories are fetched from and pushed to.
pub(crate) const GITHUB: &str = "https://github.com";
/// A fetch of a large repository the first time may take a while.
const FETCH_TIMEOUT: Duration = Duration::from_mins(15);
/// Every other git call.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// One `gh` call.
const GH_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest pull request body sent (GitHub's own cap is 65,536 characters).
const BODY_MAX: usize = 60_000;
/// The longest title.
const TITLE_MAX: usize = 100;
/// Paths given to one `git add`.
const ADD_CHUNK: usize = 200;

/// The checkouts of this runner's jobs, and how their pull requests are opened.
pub(crate) struct Workspaces {
    /// The turns' working directory (`ClaudeExec::workdir`); checkouts are in it.
    workdir: PathBuf,
    /// One bare mirror per repository, beside the working directory.
    mirrors: PathBuf,
    /// Where `<owner>/<repo>.git` is fetched from: [`GITHUB`], a directory in tests.
    remote_base: String,
    forge: Arc<dyn Forge>,
    /// One mirror fetch or worktree change at a time.
    lock: tokio::sync::Mutex<()>,
}

impl fmt::Debug for Workspaces {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workspaces")
            .field("workdir", &self.workdir)
            .field("mirrors", &self.mirrors)
            .field("remote_base", &self.remote_base)
            .finish_non_exhaustive()
    }
}

/// A job's checkout, ready for its turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prepared {
    pub(crate) repository: String,
    pub(crate) job_id: String,
    pub(crate) branch: String,
    pub(crate) path: PathBuf,
    /// The path relative to the working directory, as the pack names it.
    pub(crate) rel: String,
    /// An earlier attempt's checkout, found and kept.
    pub(crate) reused: bool,
}

/// A pull request the runner asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequest {
    pub(crate) repository: String,
    pub(crate) head: String,
    pub(crate) base: String,
    pub(crate) title: String,
    pub(crate) body: String,
}

/// The pull request that now carries the branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Opened {
    pub(crate) number: u64,
    /// Opened now, rather than an open one updated.
    pub(crate) created: bool,
}

/// What shipping a turn did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shipped {
    pub(crate) repository: String,
    pub(crate) branch: String,
    pub(crate) head: String,
    pub(crate) number: u64,
    pub(crate) created: bool,
    /// Changed files left out because their names look like secrets.
    pub(crate) withheld: Vec<String>,
    /// Main moved during the turn and did not merge cleanly: the branch was pushed
    /// without it, so the gate check holds until a later turn resolves the conflict.
    pub(crate) conflict: bool,
}

impl Shipped {
    /// The evidence line the turn's report carries.
    pub(crate) fn evidence(&self) -> String {
        format!(
            "runner: pull request {}#{} {} on {} at {}",
            self.repository,
            self.number,
            if self.created { "opened" } else { "updated" },
            self.branch,
            self.head
        ) + if self.conflict {
            "; main did not merge cleanly, the branch is behind main"
        } else {
            ""
        }
    }
}

/// A commit's title and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) title: String,
    pub(crate) body: String,
}

type Answer<'a> = Pin<Box<dyn Future<Output = Result<Opened, String>> + Send + 'a>>;

/// Where pull requests are opened: `gh` on the host, a fake in tests.
pub(crate) trait Forge: Send + Sync {
    /// Opens the pull request for `pr.head`, or updates the title and body of the one
    /// that is open.
    fn open_or_update<'a>(&'a self, pr: &'a PullRequest) -> Answer<'a>;
}

/// The host's `gh`, signed in as whoever signed it in; the runner never sees a token.
#[derive(Debug, Default)]
pub(crate) struct GhForge;

impl GhForge {
    async fn gh(args: &[&str], stdin: Option<&str>) -> Result<String, String> {
        let mut c = Command::new("gh");
        c.args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let run = async {
            let mut child = c.spawn().map_err(|e| format!("gh could not run: {e}"))?;
            if let (Some(text), Some(mut w)) = (stdin, child.stdin.take()) {
                w.write_all(text.as_bytes())
                    .await
                    .map_err(|e| format!("gh stdin: {e}"))?;
            }
            child
                .wait_with_output()
                .await
                .map_err(|e| format!("gh: {e}"))
        };
        let out = tokio::time::timeout(GH_TIMEOUT, run)
            .await
            .map_err(|_| format!("gh {} took too long", args.first().unwrap_or(&"")))??;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "gh {} exited {}: {}",
            args.first().unwrap_or(&""),
            out.status.code().unwrap_or(-1),
            redact(
                err.lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or_default()
            )
        ))
    }
}

impl Forge for GhForge {
    fn open_or_update<'a>(&'a self, pr: &'a PullRequest) -> Answer<'a> {
        Box::pin(async move {
            let r = pr.repository.as_str();
            let open = Self::gh(
                &[
                    "pr",
                    "list",
                    "-R",
                    r,
                    "--head",
                    &pr.head,
                    "--state",
                    "open",
                    "--json",
                    "number",
                    "--jq",
                    ".[0].number // empty",
                ],
                None,
            )
            .await?;
            if let Ok(number) = open.trim().parse::<u64>() {
                let n = number.to_string();
                Self::gh(
                    &[
                        "pr",
                        "edit",
                        &n,
                        "-R",
                        r,
                        "--title",
                        &pr.title,
                        "--body-file",
                        "-",
                    ],
                    Some(&pr.body),
                )
                .await?;
                return Ok(Opened {
                    number,
                    created: false,
                });
            }
            let url = Self::gh(
                &[
                    "pr",
                    "create",
                    "-R",
                    r,
                    "--head",
                    &pr.head,
                    "--base",
                    &pr.base,
                    "--title",
                    &pr.title,
                    "--body-file",
                    "-",
                ],
                Some(&pr.body),
            )
            .await?;
            let number = url
                .lines()
                .rev()
                .find_map(|l| l.trim().rsplit_once("/pull/")?.1.parse::<u64>().ok())
                .ok_or_else(|| "gh pr create named no pull request".to_owned())?;
            Ok(Opened {
                number,
                created: true,
            })
        })
    }
}

/// Words that look like a GitHub token, and the user part of a URL, are replaced.
pub(crate) fn redact(line: &str) -> String {
    const PREFIXES: [&str; 6] = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"];
    line.split(' ')
        .map(|w| {
            if PREFIXES.iter().any(|p| w.contains(p)) {
                "[redacted]".to_owned()
            } else if let Some((scheme, rest)) = w.split_once("://")
                && let Some((_, host)) = rest.split_once('@')
            {
                format!("{scheme}://[redacted]@{host}")
            } else {
                w.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `owner/name`: letters, digits, `-`, `_` and `.`, never `.` or `..` alone.
fn valid_repository(r: &str) -> Result<(&str, &str), String> {
    let part = |p: &str| {
        !p.is_empty()
            && p.len() <= 100
            && p != "."
            && p != ".."
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    match r.split_once('/') {
        Some((o, n)) if part(o) && part(n) => Ok((o, n.trim_end_matches(".git"))),
        _ => Err(format!("the job's repository {r:?} is not owner/name")),
    }
}

/// `job_` and letters or digits: it names a directory and a branch.
fn valid_job_id(id: &str) -> Result<(), String> {
    let ok = id.len() <= 64
        && id
            .strip_prefix("job_")
            .is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric()));
    if ok {
        Ok(())
    } else {
        Err(format!("the job id {id:?} cannot name a checkout"))
    }
}

/// A changed file the runner never commits: its name looks like a secret.
fn secret_like(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    name.starts_with(".env")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name.starts_with("credentials")
        || matches!(name.as_str(), ".netrc" | ".npmrc" | ".pypirc")
        || [".pem", ".key", ".p12", ".pfx", ".keystore"]
            .iter()
            .any(|s| name.ends_with(s))
}

/// The paths `git status --porcelain=v1 -z` lists (a rename's new name only).
fn changed_paths(status: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut parts = status.split('\0').filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        let (xy, path) = entry.split_at(entry.len().min(3));
        out.push(path.to_owned());
        if xy.starts_with('R') || xy.starts_with('C') {
            // The old name follows.
            parts.next();
        }
    }
    out
}

async fn git(dir: &Path, args: &[&str]) -> Result<GitOut, String> {
    let limit = if args
        .iter()
        .any(|a| matches!(*a, "fetch" | "push" | "ls-remote"))
    {
        FETCH_TIMEOUT
    } else {
        GIT_TIMEOUT
    };
    git_in(dir, args, limit).await.map_err(|e| e.to_string())
}

async fn git_ok(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = git(dir, args).await?;
    if o.code == 0 {
        Ok(o.stdout.trim().to_owned())
    } else {
        Err(format!(
            "git {} exited {}: {}",
            args.iter().find(|a| !a.starts_with('-')).unwrap_or(&""),
            o.code,
            redact(&o.stderr_tail)
        ))
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

impl Workspaces {
    pub(crate) fn new(
        workdir: PathBuf,
        mirrors: PathBuf,
        remote_base: String,
        forge: Arc<dyn Forge>,
    ) -> Self {
        Self {
            workdir,
            mirrors,
            remote_base,
            forge,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    /// The runner's own: `<workdir>/checkouts`, mirrors beside `workdir`, GitHub, `gh`.
    pub(crate) fn for_runner(workdir: PathBuf) -> Self {
        let mirrors = workdir
            .parent()
            .map_or_else(|| workdir.join("..mirrors"), |p| p.join("mirrors"));
        Self::new(workdir, mirrors, GITHUB.to_owned(), Arc::new(GhForge))
    }

    /// The mirror of `owner/name`, made once, with `main` fetched now.
    async fn mirror(&self, owner: &str, name: &str) -> Result<PathBuf, String> {
        let dir = self.mirrors.join(owner).join(format!("{name}.git"));
        if !dir.join("HEAD").is_file() {
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| format!("the mirror directory could not be made: {e}"))?;
            git_ok(&dir, &["init", "--bare", "--quiet"]).await?;
            let url = format!("{}/{owner}/{name}.git", self.remote_base);
            git_ok(&dir, &["remote", "add", "origin", &url]).await?;
            if self.remote_base == GITHUB {
                // The host's `gh` sign-in answers for this repository; set here, in the
                // runner's own mirror, never in the host's git configuration.
                git_ok(
                    &dir,
                    &[
                        "config",
                        "credential.https://github.com.helper",
                        "!gh auth git-credential",
                    ],
                )
                .await?;
            }
        }
        let main = format!("+refs/heads/{BASE}:refs/remotes/origin/{BASE}");
        git_ok(&dir, &["fetch", "--quiet", "--no-tags", "origin", &main]).await?;
        Ok(dir)
    }

    /// Makes, or finds again, the checkout of `job_id` in `repository`.
    pub(crate) async fn prepare(&self, job_id: &str, repository: &str) -> Result<Prepared, String> {
        let (owner, name) = valid_repository(repository)?;
        valid_job_id(job_id)?;
        let _one = self.lock.lock().await;
        let mirror = self.mirror(owner, name).await?;
        let branch = format!("job/{job_id}");
        let rel = format!("checkouts/{job_id}");
        let path = std::path::absolute(self.workdir.join("checkouts").join(job_id))
            .map_err(|e| format!("the checkout path: {e}"))?;
        let remote = git(
            &mirror,
            &[
                "ls-remote",
                "--exit-code",
                "--heads",
                "origin",
                &format!("refs/heads/{branch}"),
            ],
        )
        .await?;
        let pushed = match remote.code {
            0 => true,
            2 => false,
            c => {
                return Err(format!(
                    "git ls-remote exited {c}: {}",
                    redact(&remote.stderr_tail)
                ));
            }
        };
        if pushed {
            let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
            git_ok(&mirror, &["fetch", "--quiet", "--no-tags", "origin", &spec]).await?;
        }
        let reused = tokio::fs::symlink_metadata(&path).await.is_ok();
        if !reused {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("the checkouts directory could not be made: {e}"))?;
            }
            git_ok(&mirror, &["worktree", "prune"]).await?;
            let p = path.to_string_lossy().into_owned();
            let local = git(
                &mirror,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .await?
            .code
                == 0;
            if local {
                git_ok(&mirror, &["worktree", "add", "--quiet", &p, &branch]).await?;
            } else {
                let start = if pushed {
                    format!("origin/{branch}")
                } else {
                    format!("origin/{BASE}")
                };
                git_ok(
                    &mirror,
                    &[
                        "worktree",
                        "add",
                        "--quiet",
                        "--no-track",
                        "-b",
                        &branch,
                        &p,
                        &start,
                    ],
                )
                .await?;
            }
        }
        verify(&path, &mirror, &branch).await?;
        Ok(Prepared {
            repository: format!("{owner}/{name}"),
            job_id: job_id.to_owned(),
            branch,
            path,
            rel,
            reused,
        })
    }

    /// Commits what the turn changed, pushes the job's branch and opens or updates its
    /// pull request. `Ok(None)`: the branch has nothing main does not, so there is no
    /// pull request to open.
    pub(crate) async fn ship(&self, p: &Prepared, m: &Message) -> Result<Option<Shipped>, String> {
        if !p.branch.starts_with("job/") || p.branch == BASE {
            return Err(format!("refused to push {:?}: not a job branch", p.branch));
        }
        let dir = p.path.as_path();
        verify(dir, dir, &p.branch).await.or_else(|e| {
            // `verify` compares the mirror too; here only the branch and the path matter.
            if e.contains("another repository") {
                Ok(())
            } else {
                Err(e)
            }
        })?;
        // Untrimmed: the first entry's status may start with a space.
        let status = git(
            dir,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )
        .await?;
        if status.code != 0 {
            return Err(format!(
                "git status exited {}: {}",
                status.code,
                redact(&status.stderr_tail)
            ));
        }
        let status = status.stdout;
        let (withheld, paths): (Vec<String>, Vec<String>) = changed_paths(&status)
            .into_iter()
            .partition(|p| secret_like(p));
        for chunk in paths.chunks(ADD_CHUNK) {
            let mut args = vec!["--literal-pathspecs", "add", "--all", "--"];
            args.extend(chunk.iter().map(String::as_str));
            git_ok(dir, &args).await?;
        }
        let staged = git(dir, &["diff", "--cached", "--quiet"]).await?;
        match staged.code {
            0 => {}
            1 => {
                git_ok(dir, &["commit", "--quiet", "-m", &m.title, "-m", &m.body]).await?;
            }
            c => return Err(format!("git diff --cached exited {c}")),
        }
        let conflict = !merge_main(dir, &p.branch).await?;
        let ahead = git_ok(
            dir,
            &["rev-list", "--count", &format!("origin/{BASE}..HEAD")],
        )
        .await?;
        if ahead.trim() == "0" {
            return Ok(None);
        }
        let head = git_ok(dir, &["rev-parse", "HEAD"]).await?;
        git_ok(
            dir,
            &[
                "push",
                "--quiet",
                "origin",
                &format!("HEAD:refs/heads/{}", p.branch),
            ],
        )
        .await?;
        let opened = self
            .forge
            .open_or_update(&PullRequest {
                repository: p.repository.clone(),
                head: p.branch.clone(),
                base: BASE.to_owned(),
                title: m.title.clone(),
                body: m.body.clone(),
            })
            .await?;
        Ok(Some(Shipped {
            repository: p.repository.clone(),
            branch: p.branch.clone(),
            head,
            number: opened.number,
            created: opened.created,
            withheld,
            conflict,
        }))
    }
}

/// Brings the job branch up to date with origin's main, so the gate check runs on a
/// head that holds main (it skips a branch behind main, and the job then waits for the
/// reap). A branch with no commit of its own fast-forwards to main. `Ok(false)`: the
/// merge conflicted and was aborted; the branch stays as it was.
async fn merge_main(dir: &Path, branch: &str) -> Result<bool, String> {
    git_ok(
        dir,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "origin",
            &format!("+refs/heads/{BASE}:refs/remotes/origin/{BASE}"),
        ],
    )
    .await?;
    let main = format!("origin/{BASE}");
    let ancestor = git(dir, &["merge-base", "--is-ancestor", &main, "HEAD"]).await?;
    match ancestor.code {
        0 => return Ok(true),
        1 => {}
        c => {
            return Err(format!(
                "git merge-base exited {c}: {}",
                redact(&ancestor.stderr_tail)
            ));
        }
    }
    let title = format!("chore: merge {BASE} into {branch}");
    let merged = git(dir, &["merge", "--quiet", "--no-edit", "-m", &title, &main]).await?;
    if merged.code == 0 {
        return Ok(true);
    }
    // Leave the checkout as the turn left it; a failed abort leaves nothing to undo.
    let _ = git(dir, &["merge", "--abort"]).await;
    Ok(false)
}

/// The checkout at `path` is a linked worktree of `mirror` (not a main checkout), its
/// top level is `path` itself, and it is on `branch`.
async fn verify(path: &Path, mirror: &Path, branch: &str) -> Result<(), String> {
    let not_ours = || {
        format!(
            "{} is there but is not this job's checkout; it is left as it is",
            path.display()
        )
    };
    let top = git_ok(path, &["rev-parse", "--show-toplevel"])
        .await
        .map_err(|_| not_ours())?;
    if !same_path(Path::new(&top), path) {
        return Err(not_ours());
    }
    let on = git_ok(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .map_err(|_| not_ours())?;
    if on != branch {
        return Err(format!(
            "{} is on {on:?}, not {branch:?}; it is left as it is",
            path.display()
        ));
    }
    let dirs = git_ok(
        path,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    )
    .await?;
    let mut lines = dirs.lines();
    let (git_dir, common) = (
        lines.next().unwrap_or_default(),
        lines.next().unwrap_or_default(),
    );
    if same_path(Path::new(git_dir), Path::new(common)) {
        return Err(format!(
            "{} is a main checkout, not a job's worktree",
            path.display()
        ));
    }
    if mirror != path && !same_path(Path::new(common), mirror) {
        return Err(format!(
            "{} belongs to another repository; it is left as it is",
            path.display()
        ));
    }
    Ok(())
}

/// One line of text from a reply's value: a string as it is, anything else as JSON.
fn line_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The `open_pull_request` action's payload, if the reply has one.
fn pr_payload(reply: &Reply) -> Option<serde_json::Map<String, Value>> {
    reply.actions.iter().find_map(|a| {
        if a.get("kind").and_then(Value::as_str) != Some("open_pull_request") {
            return None;
        }
        match a.get("payload")? {
            Value::Object(m) => Some(m.clone()),
            Value::String(s) => serde_json::from_str::<Value>(s)
                .ok()
                .and_then(|v| v.as_object().cloned()),
            _ => None,
        }
    })
}

/// A Conventional Commits title: `type(scope)!: description`, one line, at most 100.
pub(crate) fn is_conventional(t: &str) -> bool {
    const TYPES: [&str; 11] = [
        "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore",
        "revert",
    ];
    if t.chars().count() > TITLE_MAX || t.chars().any(char::is_control) {
        return false;
    }
    let Some((head, desc)) = t.split_once(": ") else {
        return false;
    };
    if desc.trim().is_empty() {
        return false;
    }
    let head = head.strip_suffix('!').unwrap_or(head);
    let ty = match head.split_once('(') {
        Some((ty, scope)) => {
            let Some(scope) = scope.strip_suffix(')') else {
                return false;
            };
            if scope.is_empty()
                || !scope
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/' | b'.'))
            {
                return false;
            }
            ty
        }
        None => head,
    };
    TYPES.contains(&ty)
}

fn first_line(s: &str) -> &str {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// A line that would credit a tool or a model with the commit: never written.
fn attribution(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("co-authored-by") || l.contains("generated with")
}

/// The commit and pull request text for a job's reply. The title is the reply's
/// `open_pull_request` title when that is a Conventional Commit, else the first line of
/// `say` when that is one, else `chore: ` and that line. The RFC, PRD and ADR lines are
/// the ones the reply names, never made up: a missing one is said to be missing.
pub(crate) fn compose(job_id: &str, document_id: &str, reply: &Reply) -> Message {
    let payload = pr_payload(reply).unwrap_or_default();
    let text = |k: &str| {
        payload
            .get(k)
            .and_then(Value::as_str)
            .map_or("", str::trim)
            .to_owned()
    };
    let asked = first_line(&text("title")).to_owned();
    let said = first_line(&reply.say).to_owned();
    let title = if is_conventional(&asked) {
        asked
    } else if is_conventional(&said) {
        said
    } else {
        let what = if said.is_empty() {
            format!("job {job_id}")
        } else {
            said
        };
        truncate(&format!("chore: {what}"), 72)
    };
    let mut body = String::new();
    let given = text("body");
    body.push_str(if given.is_empty() {
        reply.say.trim()
    } else {
        given.as_str()
    });
    body.push_str("\n\n");
    let mut missing = Vec::new();
    for (label, key) in [("Implements", "implements"), ("PRD", "prd"), ("ADR", "adr")] {
        let v = first_line(&text(key)).to_owned();
        if v.is_empty() {
            missing.push(label);
        } else {
            let _ = writeln!(body, "{label}: {}", truncate(&v, 200));
        }
    }
    if !missing.is_empty() {
        let _ = writeln!(
            body,
            "The turn named no {} line; add it before this is merged.",
            missing.join(", ")
        );
    }
    let _ = writeln!(body, "Job: {job_id}");
    if !document_id.is_empty() {
        let _ = writeln!(body, "Document: {document_id}");
    }
    for (heading, items) in [("Evidence", &reply.evidence), ("Left", &reply.left)] {
        if !items.is_empty() {
            let _ = write!(body, "\n## {heading}\n\n");
            for i in items {
                let _ = writeln!(body, "- {}", first_line(&line_of(i)));
            }
        }
    }
    let body: String = body
        .lines()
        .filter(|l| !attribution(l))
        .collect::<Vec<_>>()
        .join("\n");
    Message {
        title: if attribution(&title) {
            format!("chore: job {job_id}")
        } else {
            title
        },
        body: truncate(body.trim(), BODY_MAX),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;
    use crate::runner::gate::tests::{sh, write};
    use serde_json::json;
    use std::sync::Mutex;

    /// Remembers what it was asked; the first ask opens #7, later ones update it.
    #[derive(Debug, Default)]
    pub(crate) struct FakeForge {
        pub(crate) asked: Mutex<Vec<PullRequest>>,
        pub(crate) fail: bool,
    }

    impl Forge for FakeForge {
        fn open_or_update<'a>(&'a self, pr: &'a PullRequest) -> Answer<'a> {
            Box::pin(async move {
                if self.fail {
                    return Err("gh pr create exited 1: no".into());
                }
                let mut a = self.asked.lock().unwrap();
                a.push(pr.clone());
                Ok(Opened {
                    number: 7,
                    created: a.len() == 1,
                })
            })
        }
    }

    /// A place with a bare `origin/acme/app.git` holding one commit on main, and an empty
    /// working directory beside it.
    pub(crate) struct Place {
        pub(crate) root: tempfile::TempDir,
        pub(crate) origin: PathBuf,
        pub(crate) workdir: PathBuf,
    }

    pub(crate) fn place() -> Place {
        let root = tempfile::tempdir().unwrap();
        let origin = root.path().join("remote").join("acme").join("app.git");
        std::fs::create_dir_all(&origin).unwrap();
        sh(&origin, &["init", "--bare", "--quiet"]);
        let seed = root.path().join("seed");
        sh(
            root.path(),
            &["clone", "--quiet", origin.to_str().unwrap(), "seed"],
        );
        write(&seed, "a.txt", "a\n");
        sh(&seed, &["add", "a.txt"]);
        sh(&seed, &["commit", "--quiet", "-m", "main"]);
        sh(&seed, &["push", "--quiet", "origin", "HEAD:main"]);
        let workdir = root.path().join("runner").join("coordinate");
        std::fs::create_dir_all(&workdir).unwrap();
        Place {
            root,
            origin,
            workdir,
        }
    }

    pub(crate) fn spaces(p: &Place, forge: Arc<dyn Forge>) -> Workspaces {
        Workspaces::new(
            p.workdir.clone(),
            p.root.path().join("runner").join("mirrors"),
            p.root.path().join("remote").to_string_lossy().into_owned(),
            forge,
        )
    }

    /// The test identity, in the mirror's own configuration (the host's is not touched).
    pub(crate) fn identity(p: &Place) {
        let m = p.root.path().join("runner/mirrors/acme/app.git");
        sh(&m, &["config", "user.email", "t@example.com"]);
        sh(&m, &["config", "user.name", "t"]);
        sh(&m, &["config", "commit.gpgsign", "false"]);
    }

    fn origin_main(p: &Place) -> String {
        sh(&p.origin, &["rev-parse", "refs/heads/main"])
    }

    #[tokio::test]
    async fn a_fresh_checkout_is_a_worktree_on_the_job_branch_from_origin_main() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        let got = w.prepare("job_01ABC", "acme/app").await.unwrap();
        assert_eq!(got.rel, "checkouts/job_01ABC");
        assert_eq!(got.branch, "job/job_01ABC");
        assert!(!got.reused);
        assert!(same_path(&got.path, &p.workdir.join("checkouts/job_01ABC")));
        assert_eq!(
            sh(&got.path, &["symbolic-ref", "--short", "HEAD"]),
            "job/job_01ABC"
        );
        assert_eq!(sh(&got.path, &["rev-parse", "HEAD"]), origin_main(&p));
        assert!(got.path.join("a.txt").is_file());
        // A linked worktree of the mirror, never a main checkout.
        let common = sh(
            &got.path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        );
        assert!(same_path(
            Path::new(&common),
            &p.root.path().join("runner/mirrors/acme/app.git")
        ));
        // Nothing of the mirror is in the working directory but the checkouts.
        let names: Vec<_> = std::fs::read_dir(&p.workdir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["checkouts"]);
    }

    #[tokio::test]
    async fn a_retry_reuses_the_checkout_and_keeps_its_work() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        let first = w.prepare("job_1", "acme/app").await.unwrap();
        write(&first.path, "wip.txt", "half done\n");
        let again = w.prepare("job_1", "acme/app").await.unwrap();
        assert!(again.reused);
        assert_eq!(again.path, first.path);
        assert_eq!(
            std::fs::read_to_string(again.path.join("wip.txt")).unwrap(),
            "half done\n"
        );
    }

    #[tokio::test]
    async fn a_new_attempt_on_another_runner_starts_from_the_pushed_job_branch() {
        let p = place();
        let a = spaces(&p, Arc::new(FakeForge::default()));
        let first = a.prepare("job_2", "acme/app").await.unwrap();
        identity(&p);
        write(&first.path, "b.txt", "b\n");
        let shipped = a
            .ship(
                &first,
                &Message {
                    title: "feat: b".into(),
                    body: "b".into(),
                },
            )
            .await
            .unwrap()
            .unwrap();
        // The same mirror and branch gone from this machine: as if another runner.
        std::fs::remove_dir_all(p.root.path().join("runner")).unwrap();
        std::fs::create_dir_all(&p.workdir).unwrap();
        let b = spaces(&p, Arc::new(FakeForge::default()));
        let second = b.prepare("job_2", "acme/app").await.unwrap();
        assert!(!second.reused);
        assert_eq!(sh(&second.path, &["rev-parse", "HEAD"]), shipped.head);
    }

    #[tokio::test]
    async fn a_checkout_that_cannot_be_made_is_an_error_and_nothing_is_deleted() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        for bad in ["acme", "../x/y", "acme/..", "a b/c", ""] {
            assert!(w.prepare("job_1", bad).await.is_err(), "{bad}");
        }
        for bad in ["job_", "job_a/b", "../job_1", "x_1"] {
            assert!(w.prepare(bad, "acme/app").await.is_err(), "{bad}");
        }
        // A repository the origin does not have.
        let e = w.prepare("job_1", "acme/missing").await.unwrap_err();
        assert!(e.contains("fetch"), "{e}");
        // Something else already at the path: refused and left there.
        let there = p.workdir.join("checkouts/job_9");
        std::fs::create_dir_all(&there).unwrap();
        write(&there, "mine.txt", "keep\n");
        let e = w.prepare("job_9", "acme/app").await.unwrap_err();
        assert!(e.contains("left as it is"), "{e}");
        assert!(there.join("mine.txt").is_file());
    }

    #[tokio::test]
    async fn shipping_commits_by_name_pushes_the_job_branch_and_opens_then_updates_the_pr() {
        let p = place();
        let forge = Arc::new(FakeForge::default());
        let w = spaces(&p, forge.clone());
        let got = w.prepare("job_3", "acme/app").await.unwrap();
        identity(&p);
        let main_before = origin_main(&p);
        write(&got.path, "new.txt", "new\n");
        write(&got.path, "a.txt", "changed\n");
        write(&got.path, ".env", "TOKEN=nope\n");
        let m = Message {
            title: "feat(app): add new".into(),
            body: "Implements: RFC 0147".into(),
        };
        let s = w.ship(&got, &m).await.unwrap().unwrap();
        assert_eq!((s.number, s.created), (7, true));
        assert_eq!(s.withheld, [".env"]);
        assert_eq!(
            sh(&p.origin, &["rev-parse", "refs/heads/job/job_3"]),
            s.head
        );
        assert_eq!(origin_main(&p), main_before, "main is never pushed");
        let files = sh(&p.origin, &["ls-tree", "--name-only", &s.head]);
        assert_eq!(files.lines().collect::<Vec<_>>(), ["a.txt", "new.txt"]);
        assert_eq!(
            sh(&p.origin, &["log", "-1", "--format=%s%n%b", &s.head]),
            "feat(app): add new\nImplements: RFC 0147"
        );
        {
            let asked = forge.asked.lock().unwrap();
            assert_eq!(
                asked[0],
                PullRequest {
                    repository: "acme/app".into(),
                    head: "job/job_3".into(),
                    base: "main".into(),
                    title: m.title.clone(),
                    body: m.body.clone(),
                }
            );
        }
        // A later turn adds to the same branch and updates the same pull request.
        write(&got.path, "more.txt", "more\n");
        let s2 = w.ship(&got, &m).await.unwrap().unwrap();
        assert_eq!((s2.number, s2.created), (7, false));
        assert_eq!(
            sh(&p.origin, &["rev-parse", &format!("{}~1", s2.head)]),
            s.head
        );
        assert!(s.evidence().contains("acme/app#7 opened on job/job_3"));
    }

    #[tokio::test]
    async fn nothing_changed_ships_nothing_and_a_refused_pr_is_an_error() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        let got = w.prepare("job_4", "acme/app").await.unwrap();
        identity(&p);
        let m = Message {
            title: "fix: x".into(),
            body: String::new(),
        };
        assert_eq!(w.ship(&got, &m).await.unwrap(), None);
        let failing = spaces(
            &p,
            Arc::new(FakeForge {
                fail: true,
                ..FakeForge::default()
            }),
        );
        write(&got.path, "x.txt", "x\n");
        let e = failing.ship(&got, &m).await.unwrap_err();
        assert!(e.contains("gh pr create"), "{e}");
    }

    /// Main moves on origin: one more commit, from the seed clone.
    fn advance_main(p: &Place, name: &str, text: &str) -> String {
        let seed = p.root.path().join("seed");
        sh(&seed, &["pull", "--quiet", "--ff-only", "origin", "main"]);
        write(&seed, name, text);
        sh(&seed, &["add", name]);
        sh(&seed, &["commit", "--quiet", "-m", "main moved"]);
        sh(&seed, &["push", "--quiet", "origin", "HEAD:main"]);
        origin_main(p)
    }

    fn is_ancestor(dir: &Path, a: &str, b: &str) -> bool {
        std::process::Command::new("git")
            .args(["merge-base", "--is-ancestor", a, b])
            .current_dir(dir)
            .status()
            .unwrap()
            .success()
    }

    #[tokio::test]
    async fn main_moved_during_the_turn_is_merged_before_the_push() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        let got = w.prepare("job_6", "acme/app").await.unwrap();
        identity(&p);
        let main = advance_main(&p, "m.txt", "m\n");
        write(&got.path, "new.txt", "new\n");
        let m = Message {
            title: "feat(app): add new".into(),
            body: String::new(),
        };
        let s = w.ship(&got, &m).await.unwrap().unwrap();
        assert!(!s.conflict);
        assert_eq!(
            sh(&p.origin, &["rev-parse", "refs/heads/job/job_6"]),
            s.head
        );
        assert!(
            is_ancestor(&p.origin, &main, &s.head),
            "the pushed head holds main"
        );
        let files = sh(&p.origin, &["ls-tree", "--name-only", &s.head]);
        assert_eq!(
            files.lines().collect::<Vec<_>>(),
            ["a.txt", "m.txt", "new.txt"]
        );
        assert_eq!(
            sh(&p.origin, &["log", "-1", "--format=%s", &s.head]),
            "chore: merge main into job/job_6"
        );
    }

    #[tokio::test]
    async fn a_branch_with_nothing_of_its_own_follows_main_and_ships_nothing() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        let got = w.prepare("job_7", "acme/app").await.unwrap();
        identity(&p);
        let main = advance_main(&p, "m.txt", "m\n");
        let m = Message {
            title: "fix: x".into(),
            body: String::new(),
        };
        assert_eq!(w.ship(&got, &m).await.unwrap(), None);
        // The gate then sees head == main and the coordinator stops the job.
        assert_eq!(sh(&got.path, &["rev-parse", "HEAD"]), main);
    }

    #[tokio::test]
    async fn a_conflict_with_main_is_aborted_and_the_branch_ships_as_it_was() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        let got = w.prepare("job_8", "acme/app").await.unwrap();
        identity(&p);
        let main = advance_main(&p, "a.txt", "theirs\n");
        write(&got.path, "a.txt", "ours\n");
        let m = Message {
            title: "fix: a".into(),
            body: String::new(),
        };
        let s = w.ship(&got, &m).await.unwrap().unwrap();
        assert!(s.conflict);
        assert!(
            s.evidence().contains("did not merge cleanly"),
            "{}",
            s.evidence()
        );
        assert!(!is_ancestor(&p.origin, &main, &s.head));
        // Windows git checks the file out again with CRLF after the abort; the content is
        // what counts.
        assert_eq!(
            std::fs::read_to_string(got.path.join("a.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "ours\n"
        );
        let status = sh(&got.path, &["status", "--porcelain"]);
        assert_eq!(status, "", "no merge is left half done");
    }

    #[tokio::test]
    async fn shipping_refuses_a_branch_that_is_not_a_job_branch() {
        let p = place();
        let w = spaces(&p, Arc::new(FakeForge::default()));
        let mut got = w.prepare("job_5", "acme/app").await.unwrap();
        got.branch = "main".into();
        let e = w
            .ship(
                &got,
                &Message {
                    title: "fix: x".into(),
                    body: String::new(),
                },
            )
            .await
            .unwrap_err();
        assert!(e.contains("not a job branch"), "{e}");
    }

    fn reply(say: &str, actions: Vec<Value>) -> Reply {
        Reply {
            say: say.into(),
            state: String::new(),
            actions,
            evidence: vec![json!("cargo test: 3 passed")],
            left: vec![json!({"item": "docs"})],
        }
    }

    #[test]
    fn the_message_takes_the_actions_title_and_document_lines() {
        let r = reply(
            "Done.",
            vec![json!({"kind": "open_pull_request", "payload": {
                "title": "feat(cli): the runner owns the checkout",
                "body": "What it does.\nCo-Authored-By: someone\nGenerated with a tool",
                "implements": "RFC 0147", "prd": "0152", "adr": "0141"
            }})],
        );
        let m = compose("job_1", "ldoc_1", &r);
        assert_eq!(m.title, "feat(cli): the runner owns the checkout");
        assert!(m.body.starts_with("What it does.\n"));
        for want in [
            "Implements: RFC 0147",
            "PRD: 0152",
            "ADR: 0141",
            "Job: job_1",
            "Document: ldoc_1",
            "- cargo test: 3 passed",
            "- {\"item\":\"docs\"}",
        ] {
            assert!(m.body.contains(want), "{want} in {}", m.body);
        }
        assert!(!m.body.to_ascii_lowercase().contains("co-authored-by"));
        assert!(!m.body.to_ascii_lowercase().contains("generated with"));
        assert!(!m.body.contains("named no"));
    }

    #[test]
    fn the_message_never_invents_a_document_line_or_a_type() {
        let m = compose("job_2", "", &reply("Added the thing\nmore", vec![]));
        assert_eq!(m.title, "chore: Added the thing");
        assert!(
            m.body
                .contains("The turn named no Implements, PRD, ADR line")
        );
        assert!(!m.body.contains("Implements: "));
        let m = compose("job_2", "", &reply("fix(core): the bug", vec![]));
        assert_eq!(m.title, "fix(core): the bug");
        let m = compose("job_3", "", &reply("", vec![]));
        assert_eq!(m.title, "chore: job job_3");
        // A payload sent as a JSON string is read too.
        let a = json!({"kind": "open_pull_request", "payload": "{\"title\":\"docs: x\"}"});
        assert_eq!(compose("job_4", "", &reply("y", vec![a])).title, "docs: x");
    }

    #[test]
    fn conventional_titles_and_secret_names_and_redaction() {
        for ok in ["feat: a", "fix(cli): b", "feat(a/b)!: c", "chore!: d"] {
            assert!(is_conventional(ok), "{ok}");
        }
        for bad in [
            "Feat: a",
            "feat:a",
            "feat(): a",
            "wip: a",
            "feat: ",
            "nothing",
        ] {
            assert!(!is_conventional(bad), "{bad}");
        }
        for s in [
            ".env",
            "x/.env.local",
            "id_rsa",
            "a/server.pem",
            "tls.key",
            ".netrc",
        ] {
            assert!(secret_like(s), "{s}");
        }
        for s in ["src/env.rs", "keys.rs", "README.md"] {
            assert!(!secret_like(s), "{s}");
        }
        assert_eq!(
            redact("fatal: https://x:ghp_abc@github.com/a token ghs_1"),
            "fatal: [redacted] token [redacted]"
        );
        assert_eq!(
            redact("see https://user:pw@host/x"),
            "see https://[redacted]@host/x"
        );
        assert_eq!(
            changed_paths(" M a.txt\0?? new dir/x\0R  new\0old\0"),
            ["a.txt", "new dir/x", "new"]
        );
    }
}
