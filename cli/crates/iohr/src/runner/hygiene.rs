//! Disk hygiene of the runner's job checkouts (RFC 0189, ADR 0177).
//!
//! **One build directory per repository.** A job's gate check and its model's `cargo`
//! build into `<state>/target/<owner>/<repo>` (`CARGO_TARGET_DIR`), not into a `target/`
//! in each checkout. Cargo locks that directory, so two builds of one repository take
//! turns ("Blocking waiting for file lock"): slower together, but one copy of the
//! dependencies on disk, not one per job.
//!
//! **Nothing unused stays for ever.** A job's checkout (`<state>/coordinate/checkouts/<job>`),
//! a repository's mirror (`<state>/mirrors/<owner>/<repo>.git`) and its build directory
//! are removed once nothing has used them for `workspace_keep_days` ([`KEEP_DAYS`] by
//! default); with `workspace_max_gb` set, the oldest unused also go while everything
//! takes more than that. `iohr runner prune` does it on demand (`--dry-run` lists only);
//! the runner does it while it holds no lease, at most hourly ([`sweep`]).
//!
//! A checkout a live lease uses is never touched: the runner holds an exclusive lock on
//! `checkouts/.held/<job>.lock` from before the checkout is made until the lease's work
//! has ended ([`try_hold`]), and a prune removes a checkout only while it holds that lock
//! itself. The lock is the operating system's, so it holds across every runner process on
//! the machine and goes with the process that held it. A mirror goes only when no
//! checkout is left on it, a build directory only with its mirror (or over the cap), and
//! nothing used in the last [`GRACE`] goes even over the cap. The job's branch stays in a
//! mirror that stays, so a job that comes back finds its commits.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::workspace::{is_repository_part, valid_job_id};

/// The variable that points a child's `cargo` at the shared directory.
pub(crate) const TARGET_VAR: &str = "CARGO_TARGET_DIR";

/// The shared build directory of a checkout's repository, from the checkout's `.git`
/// file (`gitdir: <state>/mirrors/<owner>/<repo>.git/worktrees/<job>`). `None` for
/// anything that is not one of this runner's worktrees.
pub(crate) fn target_from_gitfile(text: &str, state: &Path) -> Option<PathBuf> {
    let (owner, name) = repo_from_gitfile(text, state)?;
    Some(state.join("target").join(owner).join(name))
}

/// [`target_from_gitfile`] for the checkout at `checkout`, under the runner's `workdir`
/// (whose parent is the state directory).
pub(crate) fn target_for(workdir: &Path, checkout: &Path) -> Option<PathBuf> {
    // Git writes the real path into the `.git` file; a state directory reached through a
    // symlink (macOS `/var` -> `/private/var`) must be compared by its real path.
    let state = workdir.parent()?.canonicalize().ok()?;
    let text = std::fs::read_to_string(checkout.join(".git")).ok()?;
    target_from_gitfile(&text, &state)
}

/// `env` with [`TARGET_VAR`] set to `target` (replacing any other); `env` as it was when
/// there is no shared directory.
pub(crate) fn with_target(
    mut env: Vec<(OsString, OsString)>,
    target: Option<&Path>,
) -> Vec<(OsString, OsString)> {
    if let Some(t) = target {
        env.retain(|(k, _)| !k.to_string_lossy().eq_ignore_ascii_case(TARGET_VAR));
        env.push((TARGET_VAR.into(), t.as_os_str().to_owned()));
    }
    env
}

/// Days an unused checkout, mirror or build directory is kept by default.
pub(crate) const KEEP_DAYS: u32 = 14;
/// The most `workspace_keep_days` may say (ten years).
pub(crate) const MAX_KEEP_DAYS: u32 = 3650;
/// Nothing used this recently is removed, not even over the size cap.
pub(crate) const GRACE: Duration = Duration::from_hours(1);
/// How often a runner that holds no lease sweeps.
pub(crate) const SWEEP_EVERY: Duration = Duration::from_hours(1);
/// The directory in `checkouts/` that holds the checkouts' locks.
const HELD: &str = ".held";
const DAY: u64 = 86_400;
const GB: u64 = 1_000_000_000;

/// How long unused things stay, and the most they may take together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Retention {
    pub(crate) keep: Duration,
    pub(crate) max_bytes: Option<u64>,
}

impl Retention {
    /// From `workspace_keep_days` and `workspace_max_gb`.
    pub(crate) fn of(keep_days: u32, max_gb: Option<u64>) -> Self {
        Self {
            keep: Duration::from_secs(u64::from(keep_days) * DAY),
            max_bytes: max_gb.map(|g| g.saturating_mul(GB)),
        }
    }
}

/// The runner's idle sweep: its state directory and how long things stay.
#[derive(Debug, Clone)]
pub(crate) struct Sweep {
    pub(crate) state: PathBuf,
    pub(crate) retention: Retention,
}

/// What a prune removes; in the order it removes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Checkout,
    Mirror,
    /// A repository's shared build directory.
    Target,
}

impl Kind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Checkout => "checkout",
            Self::Mirror => "mirror",
            Self::Target => "build directory",
        }
    }
}

type Repo = Option<(String, String)>;

/// One thing a prune removes, and why.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Item {
    pub(crate) kind: Kind,
    pub(crate) path: PathBuf,
    /// Whole days since anything in it was last used.
    pub(crate) idle_days: u64,
    /// Its size; measured only when a size cap is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bytes: Option<u64>,
    /// `unused` (unused for longer than the days kept) or `over_cap` (the oldest while
    /// everything takes more than the cap).
    pub(crate) why: &'static str,
    #[serde(skip)]
    idle: Duration,
    /// A checkout's job, whose lock its removal takes.
    #[serde(skip)]
    job: Option<String>,
    /// The repository (`owner`, `name`) a checkout is on, or a mirror's or build
    /// directory's own.
    #[serde(skip)]
    repo: Repo,
}

/// What a prune would remove, and the checkouts it leaves because a lease holds them.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Plan {
    pub(crate) remove: Vec<Item>,
    pub(crate) in_use: Vec<PathBuf>,
}

/// A checkout held for a live lease: the lock goes when this is dropped, or with the
/// process.
#[derive(Debug)]
pub(crate) struct Hold {
    _file: File,
}

fn held_path(checkouts: &Path, job_id: &str) -> PathBuf {
    checkouts.join(HELD).join(format!("{job_id}.lock"))
}

/// Takes the lock of `job_id`'s checkout in `checkouts`; `None` when another holder (in
/// this process or another) has it. Never waits.
pub(crate) fn try_hold(checkouts: &Path, job_id: &str) -> io::Result<Option<Hold>> {
    valid_job_id(job_id).map_err(io::Error::other)?;
    let path = held_path(checkouts, job_id);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // A prune removes the lock file of a checkout it removed: a file opened just before
    // that is no longer the one at the path, and is opened again.
    for _ in 0..3 {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        if is_at(&file, &path) {
            // Its time says when the checkout was last taken.
            let _ = file.set_modified(SystemTime::now());
            return Ok(Some(Hold { _file: file }));
        }
    }
    Err(io::Error::other("its lock file keeps being replaced"))
}

/// Whether `file` is the file at `path` now.
#[cfg(unix)]
fn is_at(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    match (file.metadata(), fs::metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_at(_file: &File, path: &Path) -> bool {
    path.is_file()
}

/// Whether a holder has `job_id`'s lock now; makes nothing. A child process started
/// while the lock was held keeps it until that child runs its program, so a lock just
/// let go may read as held for a moment: never the other way round.
fn is_held(checkouts: &Path, job_id: &str) -> bool {
    let Ok(file) = File::open(held_path(checkouts, job_id)) else {
        return false;
    };
    // Any answer but "free" keeps the checkout.
    file.try_lock_shared().is_err()
}

/// The newest modification time of `paths` (of those that exist).
fn newest<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Option<SystemTime> {
    paths
        .into_iter()
        .filter_map(|p| fs::symlink_metadata(p).and_then(|m| m.modified()).ok())
        .max()
}

/// The newest of a directory and what is in it, to `depth` levels: a build directory's
/// newest is in `debug/` or `release/`.
fn newest_within(dir: &Path, depth: u32) -> Option<SystemTime> {
    let mut best = newest([dir]);
    if depth > 0
        && let Ok(entries) = fs::read_dir(dir)
    {
        for e in entries.flatten() {
            let t = match e.file_type() {
                Ok(t) if t.is_dir() => newest_within(&e.path(), depth - 1),
                Ok(_) => newest([e.path().as_path()]),
                Err(_) => None,
            };
            best = best.max(t);
        }
    }
    best
}

/// The bytes of the files under `dir`; symlinks are not followed.
fn size_of(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => total += e.metadata().map_or(0, |m| m.len()),
                Err(_) => {}
            }
        }
    }
    total
}

/// The directories in `dir` (never a symlink, never a hidden one: the locks and what a
/// prune is removing), by name.
fn dirs_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, _)| !name.starts_with('.'))
        .collect();
    out.sort();
    out
}

/// The repository (`owner`, `name`) of the mirror a checkout's `.git` file names, when
/// it is one of `state`'s.
fn repo_from_gitfile(text: &str, state: &Path) -> Repo {
    let gitdir = Path::new(text.lines().next()?.strip_prefix("gitdir:")?.trim());
    let mirror = gitdir.parent()?.parent()?;
    let owner_dir = mirror.parent()?;
    if owner_dir.parent()? != state.join("mirrors") {
        return None;
    }
    let owner = owner_dir.file_name()?.to_str()?;
    let name = mirror.file_name()?.to_str()?.strip_suffix(".git")?;
    (is_repository_part(owner) && is_repository_part(name))
        .then(|| (owner.to_owned(), name.to_owned()))
}

fn idle_since(t: Option<SystemTime>, now: SystemTime) -> Duration {
    // Nothing readable in it: as old as can be.
    t.map_or(Duration::MAX, |t| now.duration_since(t).unwrap_or_default())
}

fn checkout_idle(checkouts: &Path, job: &str, path: &Path, now: SystemTime) -> Duration {
    let (git, lock) = (path.join(".git"), held_path(checkouts, job));
    idle_since(newest([path, git.as_path(), lock.as_path()]), now)
}

fn mirror_idle(path: &Path, now: SystemTime) -> Duration {
    let (fetched, trees) = (path.join("FETCH_HEAD"), path.join("worktrees"));
    idle_since(newest([path, fetched.as_path(), trees.as_path()]), now)
}

fn item(kind: Kind, path: PathBuf, idle: Duration, repo: Repo) -> Item {
    Item {
        kind,
        path,
        idle_days: idle.as_secs() / DAY,
        bytes: None,
        why: "",
        idle,
        job: None,
        repo,
    }
}

/// Every checkout, mirror and build directory under `state`; a held checkout goes into
/// `in_use` instead, its repository into `held`.
fn scan(state: &Path, now: SystemTime, in_use: &mut Vec<PathBuf>, held: &mut Vec<Repo>) -> Vec<Item> {
    let mut items = Vec::new();
    let checkouts = state.join("coordinate").join("checkouts");
    for (job, path) in dirs_in(&checkouts) {
        if valid_job_id(&job).is_err() {
            continue;
        }
        let repo = fs::read_to_string(path.join(".git"))
            .ok()
            .and_then(|t| repo_from_gitfile(&t, state));
        if is_held(&checkouts, &job) {
            held.push(repo);
            in_use.push(path);
            continue;
        }
        let idle = checkout_idle(&checkouts, &job, &path, now);
        let mut it = item(Kind::Checkout, path, idle, repo);
        it.job = Some(job);
        items.push(it);
    }
    for (owner, owner_dir) in dirs_in(&state.join("mirrors")) {
        for (dir, path) in dirs_in(&owner_dir) {
            if let Some(name) = dir.strip_suffix(".git") {
                let idle = mirror_idle(&path, now);
                items.push(item(Kind::Mirror, path, idle, Some((owner.clone(), name.to_owned()))));
            }
        }
    }
    for (owner, owner_dir) in dirs_in(&state.join("target")) {
        for (name, path) in dirs_in(&owner_dir) {
            let idle = idle_since(newest_within(&path, 2), now);
            items.push(item(Kind::Target, path, idle, Some((owner.clone(), name))));
        }
    }
    items
}

/// Whether a checkout that stays is on `repo`.
fn has_checkout(items: &[Item], why: &[Option<&'static str>], held: &[Repo], repo: &Repo) -> bool {
    held.contains(repo)
        || items
            .iter()
            .zip(why)
            .any(|(i, w)| w.is_none() && i.kind == Kind::Checkout && &i.repo == repo)
}

/// Whether `repo`'s mirror stays.
fn has_mirror(items: &[Item], why: &[Option<&'static str>], repo: &Repo) -> bool {
    items
        .iter()
        .zip(why)
        .any(|(i, w)| w.is_none() && i.kind == Kind::Mirror && &i.repo == repo)
}

/// What a prune of the runner's `state` directory would remove now; reads only.
pub(crate) fn plan(state: &Path, r: Retention, now: SystemTime) -> Plan {
    let mut plan = Plan::default();
    // Git writes real paths into the `.git` files: compare with the real one.
    let Ok(state) = state.canonicalize() else {
        return plan;
    };
    let mut held = Vec::new();
    let mut items = scan(&state, now, &mut plan.in_use, &mut held);
    let mut why: Vec<Option<&'static str>> = items
        .iter()
        .map(|i| (i.kind == Kind::Checkout && i.idle > r.keep).then_some("unused"))
        .collect();
    // A mirror no checkout is left on; then a build directory whose mirror is gone.
    for kind in [Kind::Mirror, Kind::Target] {
        for k in 0..items.len() {
            let it = &items[k];
            let go = it.kind == kind
                && it.idle > r.keep
                && match kind {
                    Kind::Mirror => !has_checkout(&items, &why, &held, &it.repo),
                    _ => !has_mirror(&items, &why, &it.repo),
                };
            if go {
                why[k] = Some("unused");
            }
        }
    }
    if let Some(cap) = r.max_bytes {
        for it in &mut items {
            it.bytes = Some(size_of(&it.path));
        }
        let mut total: u64 = plan.in_use.iter().map(|p| size_of(p)).sum::<u64>()
            + items
                .iter()
                .zip(&why)
                .filter(|(_, w)| w.is_none())
                .filter_map(|(i, _)| i.bytes)
                .sum::<u64>();
        let mut order: Vec<usize> = (0..items.len())
            .filter(|&k| why[k].is_none() && items[k].idle > GRACE)
            .collect();
        // The oldest first; a mirror waits until no checkout is left on it.
        order.sort_by(|&a, &b| items[b].idle.cmp(&items[a].idle));
        let mut changed = true;
        while total > cap && changed {
            changed = false;
            for &k in &order {
                if total <= cap {
                    break;
                }
                let it = &items[k];
                if why[k].is_some()
                    || (it.kind == Kind::Mirror && has_checkout(&items, &why, &held, &it.repo))
                {
                    continue;
                }
                why[k] = Some("over_cap");
                total = total.saturating_sub(it.bytes.unwrap_or(0));
                changed = true;
            }
        }
    }
    let mut remove: Vec<Item> = items
        .into_iter()
        .zip(why)
        .filter_map(|(mut i, w)| {
            i.why = w?;
            Some(i)
        })
        .collect();
    remove.sort_by_key(|i| i.kind);
    plan.remove = remove;
    plan
}

/// What one removal did.
pub(crate) type Done = (Item, Result<(), String>);

/// Removes what `plan` lists, each checked again first: a checkout only while this holds
/// its lock, anything only if still unused for as long as the plan said. What is in use
/// now stays, and its result says so.
pub(crate) fn apply(state: &Path, r: Retention, plan: Plan, now: SystemTime) -> Vec<Done> {
    let Ok(state) = state.canonicalize() else {
        return Vec::new();
    };
    let checkouts = state.join("coordinate").join("checkouts");
    // What an earlier prune was stopped in the middle of removing.
    let mut parents = vec![checkouts.clone()];
    for top in ["mirrors", "target"] {
        parents.extend(dirs_in(&state.join(top)).into_iter().map(|(_, p)| p));
    }
    for dir in parents {
        for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') && name.ends_with(REMOVING) {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    let mut done: Vec<Done> = Vec::new();
    for it in plan.remove {
        let limit = if it.why == "over_cap" { GRACE } else { r.keep };
        let result = match (it.kind, it.job.as_deref()) {
            (Kind::Checkout, Some(job)) => remove_checkout(&checkouts, job, &it.path, limit, now),
            (Kind::Mirror, _) if mirror_idle(&it.path, now) <= limit => {
                Err("used again since: kept".into())
            }
            (Kind::Target, _) if idle_since(newest_within(&it.path, 2), now) <= limit => {
                Err("used again since: kept".into())
            }
            _ => remove_whole(&it.path),
        };
        done.push((it, result));
    }
    // The mirrors that stay forget the worktrees just removed (best effort: the next
    // checkout made in one prunes it too); emptied owner directories go.
    let mut pruned: Vec<PathBuf> = Vec::new();
    for (it, result) in &done {
        if result.is_err() {
            continue;
        }
        match (&it.kind, &it.repo) {
            (Kind::Checkout, Some((owner, name))) => {
                let m = state.join("mirrors").join(owner).join(format!("{name}.git"));
                if m.is_dir() && !pruned.contains(&m) {
                    let _ = std::process::Command::new("git")
                        .arg("--git-dir")
                        .arg(&m)
                        .args(["worktree", "prune"])
                        .stdin(std::process::Stdio::null())
                        .output();
                    pruned.push(m);
                }
            }
            (Kind::Mirror | Kind::Target, _) => {
                if let Some(owner_dir) = it.path.parent() {
                    let _ = fs::remove_dir(owner_dir);
                }
            }
            _ => {}
        }
    }
    done
}

fn remove_checkout(checkouts: &Path, job: &str, path: &Path, limit: Duration, now: SystemTime) -> Result<(), String> {
    let hold = match try_hold(checkouts, job) {
        Ok(Some(h)) => h,
        Ok(None) => return Err("a lease holds it now: kept".into()),
        Err(e) => return Err(format!("its lock could not be taken: {e}")),
    };
    // Taking the lock touched its file: the checkout's own times say whether it was used.
    let (git, at) = (path.join(".git"), now);
    if idle_since(newest([path, git.as_path()]), at) <= limit {
        return Err("used again since: kept".into());
    }
    remove_whole(path)?;
    // Removed while held: a holder that opened the old file opens the new one.
    let _ = fs::remove_file(held_path(checkouts, job));
    drop(hold);
    Ok(())
}

/// The suffix of a directory being removed.
const REMOVING: &str = ".removing";

/// Removes `path` whole: first renamed out of the way in one step, so a prune stopped in
/// the middle never leaves a half-removed checkout or mirror where the runner would use
/// it again; the next prune removes the rest.
fn remove_whole(path: &Path) -> Result<(), String> {
    let name = path.file_name().ok_or("no name")?.to_string_lossy().into_owned();
    let gone = path.with_file_name(format!(".{name}{REMOVING}"));
    let _ = fs::remove_dir_all(&gone);
    fs::rename(path, &gone).map_err(|e| e.to_string())?;
    fs::remove_dir_all(&gone).map_err(|e| e.to_string())
}

/// `bytes` as GB with one decimal.
pub(crate) fn gb(bytes: u64) -> String {
    format!("{}.{} GB", bytes / GB, (bytes % GB) / (GB / 10))
}

/// One line for the log: what went and what stayed.
pub(crate) fn summary(done: &[Done]) -> String {
    let removed: Vec<&Item> = done.iter().filter(|(_, r)| r.is_ok()).map(|(i, _)| i).collect();
    let count = |k: Kind| removed.iter().filter(|i| i.kind == k).count();
    let mut line = format!(
        "workspace prune: removed {} checkouts, {} mirrors, {} build directories",
        count(Kind::Checkout),
        count(Kind::Mirror),
        count(Kind::Target)
    );
    let freed: u64 = removed.iter().filter_map(|i| i.bytes).sum();
    if freed > 0 {
        let _ = write!(line, " ({} freed)", gb(freed));
    }
    let kept: Vec<String> = done
        .iter()
        .filter_map(|(i, r)| r.as_ref().err().map(|e| format!("{}: {e}", i.path.display())))
        .collect();
    if !kept.is_empty() {
        let _ = write!(line, "; not removed: {}", kept.join("; "));
    }
    line
}

/// The idle sweep: a prune with the runner's settings; `None` when nothing was due.
pub(crate) fn sweep(s: &Sweep) -> Option<String> {
    let now = SystemTime::now();
    let plan = plan(&s.state, s.retention, now);
    if plan.remove.is_empty() {
        return None;
    }
    Some(summary(&apply(&s.state, s.retention, plan, now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worktree_builds_into_its_repository_shared_directory() {
        let state = Path::new("/s/runner");
        let git = "gitdir: /s/runner/mirrors/inorbithr/core.git/worktrees/job_1\n";
        assert_eq!(
            target_from_gitfile(git, state),
            Some(PathBuf::from("/s/runner/target/inorbithr/core"))
        );
        // Another repository's job shares nothing with it; the same one shares all.
        let other = "gitdir: /s/runner/mirrors/inorbithr/sdk.git/worktrees/job_2";
        assert_ne!(
            target_from_gitfile(other, state),
            target_from_gitfile(git, state)
        );
        let same = "gitdir: /s/runner/mirrors/inorbithr/core.git/worktrees/job_9";
        assert_eq!(
            target_from_gitfile(same, state),
            target_from_gitfile(git, state)
        );
    }

    #[test]
    fn a_checkout_that_is_not_this_runners_worktree_gets_no_shared_directory() {
        let state = Path::new("/s/runner");
        for bad in [
            "",
            "not a gitfile",
            "gitdir: /elsewhere/mirrors/o/r.git/worktrees/job_1",
            "gitdir: /s/runner/mirrors/o/r/worktrees/job_1",
            "gitdir: /s/runner/mirrors/../r.git/worktrees/job_1",
            "gitdir: /s/runner/mirrors/o/r.git",
        ] {
            assert_eq!(target_from_gitfile(bad, state), None, "{bad}");
        }
    }

    #[test]
    fn the_target_variable_replaces_one_the_runner_was_started_with() {
        let env: Vec<(OsString, OsString)> = vec![
            ("PATH".into(), "/bin".into()),
            ("cargo_target_dir".into(), "/elsewhere".into()),
        ];
        let out = with_target(env.clone(), Some(Path::new("/t")));
        assert_eq!(out.len(), 2);
        assert!(out.contains(&(TARGET_VAR.into(), "/t".into())));
        assert_eq!(with_target(env.clone(), None), env);
    }

    #[test]
    fn a_real_checkout_is_found_through_its_git_file() {
        let state = tempfile::tempdir().unwrap();
        let state_path = state.path().canonicalize().unwrap();
        let workdir = state_path.join("coordinate");
        let checkout = workdir.join("o").join("r").join("job_1");
        std::fs::create_dir_all(&checkout).unwrap();
        let gitdir = state_path.join("mirrors/o/r.git/worktrees/job_1");
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();
        assert_eq!(
            target_for(&workdir, &checkout),
            Some(state_path.join("target/o/r"))
        );
        // A folder with no `.git` file is not one of ours.
        assert_eq!(target_for(&workdir, &workdir), None);
    }

    const D: Duration = Duration::from_secs(DAY);

    /// Waits until `job_id`'s lock is free: one let go may stay held for a moment in a
    /// child another test is starting ([`is_held`]).
    fn until_free(checkouts: &Path, job_id: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while is_held(checkouts, job_id) {
            assert!(std::time::Instant::now() < deadline, "{job_id} is still held");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Sets the modification time of `path` and everything under it to `ago` before now.
    fn age(path: &Path, ago: Duration) {
        let t = SystemTime::now() - ago;
        if path.is_dir() {
            for e in fs::read_dir(path).unwrap().flatten() {
                age(&e.path(), ago);
            }
        }
        File::open(path).unwrap().set_modified(t).unwrap();
    }

    /// A runner state with a checkout of `job` on `owner/repo` of `bytes` bytes.
    fn checkout(state: &Path, job: &str, repo: &str, bytes: usize) -> PathBuf {
        let dir = state.join("coordinate/checkouts").join(job);
        fs::create_dir_all(&dir).unwrap();
        let gitdir = state.join("mirrors").join(format!("{repo}.git/worktrees/{job}"));
        fs::write(dir.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        fs::write(dir.join("file"), vec![b'x'; bytes]).unwrap();
        dir
    }

    fn mirror(state: &Path, repo: &str, bytes: usize) -> PathBuf {
        let dir = state.join("mirrors").join(format!("{repo}.git"));
        fs::create_dir_all(dir.join("worktrees")).unwrap();
        fs::write(dir.join("FETCH_HEAD"), vec![b'x'; bytes]).unwrap();
        dir
    }

    fn target(state: &Path, repo: &str, bytes: usize) -> PathBuf {
        let dir = state.join("target").join(repo);
        fs::create_dir_all(dir.join("debug")).unwrap();
        fs::write(dir.join("debug/out"), vec![b'x'; bytes]).unwrap();
        dir
    }

    fn state() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let s = t.path().canonicalize().unwrap().join("runner");
        fs::create_dir_all(&s).unwrap();
        (t, s)
    }

    fn removed(p: &Plan) -> Vec<(Kind, String)> {
        p.remove
            .iter()
            .map(|i| (i.kind, i.path.file_name().unwrap().to_string_lossy().into_owned()))
            .collect()
    }

    #[test]
    fn what_nothing_used_for_the_days_kept_goes_and_what_a_kept_checkout_uses_stays() {
        let (_t, s) = state();
        let old = checkout(&s, "job_old", "o/r", 10);
        let young = checkout(&s, "job_young", "o/u", 10);
        let (m_r, m_u) = (mirror(&s, "o/r", 10), mirror(&s, "o/u", 10));
        let (t_r, t_u) = (target(&s, "o/r", 10), target(&s, "o/u", 10));
        for p in [&old, &m_r, &m_u, &t_r, &t_u] {
            age(p, 20 * D);
        }
        age(&young, 2 * D);
        let r = Retention::of(KEEP_DAYS, None);
        let plan = plan(&s, r, SystemTime::now());
        // In order: the checkout, then its mirror (no checkout left on it), then its build
        // directory (its mirror gone); `o/u`'s mirror stays for its young checkout, and
        // its build directory with it.
        assert_eq!(
            removed(&plan),
            [
                (Kind::Checkout, "job_old".into()),
                (Kind::Mirror, "r.git".into()),
                (Kind::Target, "r".into()),
            ]
        );
        assert!(plan.remove.iter().all(|i| i.why == "unused" && i.idle_days >= 19));
        // A dry run is the plan alone: nothing changed.
        for p in [&old, &young, &m_r, &m_u, &t_r, &t_u] {
            assert!(p.is_dir(), "{}", p.display());
        }
        let done = apply(&s, r, plan, SystemTime::now());
        assert!(done.iter().all(|(_, r)| r.is_ok()), "{done:?}");
        for p in [&old, &m_r, &t_r] {
            assert!(!p.exists(), "{}", p.display());
        }
        for p in [&young, &m_u, &t_u] {
            assert!(p.is_dir(), "{}", p.display());
        }
        // Nothing half removed is left behind, and the line says what went.
        assert!(fs::read_dir(s.join("coordinate/checkouts")).unwrap().flatten().all(|e| {
            !e.file_name().to_string_lossy().ends_with(REMOVING)
        }));
        let line = summary(&done);
        assert!(line.contains("removed 1 checkouts, 1 mirrors, 1 build directories"), "{line}");
    }

    #[test]
    fn a_checkout_a_lease_holds_is_never_removed_nor_its_mirror() {
        let (_t, s) = state();
        let co = checkout(&s, "job_1", "o/r", 10);
        let m = mirror(&s, "o/r", 10);
        let checkouts = s.join("coordinate/checkouts");
        let r = Retention::of(1, Some(1));
        // Held before the plan: in use, and its mirror stays even over the cap.
        let hold = try_hold(&checkouts, "job_1").unwrap().unwrap();
        age(&co, 30 * D);
        age(&m, 30 * D);
        let p = plan(&s, r, SystemTime::now());
        assert_eq!(p.in_use, [co.clone()]);
        assert!(removed(&p).is_empty(), "{:?}", removed(&p));
        drop(hold);
        until_free(&checkouts, "job_1");
        // Taking the lock counts as a use: young again until the lock is old too.
        assert!(removed(&plan(&s, Retention::of(1, None), SystemTime::now())).is_empty());
        age(&checkouts.join(HELD), 30 * D);
        // Held after the plan, before the removal: the removal leaves it, and says why.
        let p = plan(&s, Retention::of(1, None), SystemTime::now());
        assert_eq!(removed(&p)[0], (Kind::Checkout, "job_1".into()));
        let hold = try_hold(&checkouts, "job_1").unwrap().unwrap();
        let done = apply(&s, Retention::of(1, None), p, SystemTime::now());
        assert!(
            done[0].1.as_ref().is_err_and(|e| e.contains("a lease holds it")),
            "{done:?}"
        );
        assert!(co.is_dir());
        drop(hold);
    }

    #[test]
    fn over_the_cap_the_oldest_go_first_but_never_what_was_used_within_the_grace() {
        let (_t, s) = state();
        let a = checkout(&s, "job_a", "o/r", 1000);
        let b = checkout(&s, "job_b", "o/r", 1000);
        let c = checkout(&s, "job_c", "o/r", 1000);
        let m = mirror(&s, "o/r", 1000);
        let t = target(&s, "o/r", 1000);
        age(&a, 10 * D);
        age(&b, 5 * D);
        age(&t, 3 * D);
        age(&m, D);
        age(&c, Duration::from_mins(10));
        let r = Retention {
            keep: 365 * D,
            max_bytes: Some(3500),
        };
        let p = plan(&s, r, SystemTime::now());
        // About 5 kB: the two oldest checkouts bring it under 3.5 kB; the build directory
        // is younger, the mirror is used by a kept checkout, `job_c` is in its grace.
        assert_eq!(
            removed(&p),
            [(Kind::Checkout, "job_a".into()), (Kind::Checkout, "job_b".into())]
        );
        assert!(p.remove.iter().all(|i| i.why == "over_cap" && i.bytes > Some(1000)));
        // A cap nothing can meet removes all it may and no more.
        let p = plan(&s, Retention { max_bytes: Some(1), ..r }, SystemTime::now());
        let gone = removed(&p);
        assert!(gone.contains(&(Kind::Target, "r".into())), "{gone:?}");
        assert!(!gone.contains(&(Kind::Checkout, "job_c".into())), "{gone:?}");
        assert!(!gone.contains(&(Kind::Mirror, "r.git".into())), "{gone:?}");
        assert!([a, b, c, m, t].iter().all(|p| p.is_dir()));
    }

    #[test]
    fn a_checkout_lock_has_one_holder_at_a_time() {
        let (_t, s) = state();
        let checkouts = s.join("coordinate/checkouts");
        let first = try_hold(&checkouts, "job_1").unwrap();
        assert!(first.is_some());
        assert!(try_hold(&checkouts, "job_1").unwrap().is_none());
        assert!(is_held(&checkouts, "job_1"));
        // Another job's lock is its own.
        assert!(try_hold(&checkouts, "job_2").unwrap().is_some());
        drop(first);
        until_free(&checkouts, "job_1");
        assert!(try_hold(&checkouts, "job_1").unwrap().is_some());
        assert!(try_hold(&checkouts, "../job_1").is_err());
    }

    #[test]
    fn a_prune_stopped_halfway_is_finished_by_the_next_and_never_listed() {
        let (_t, s) = state();
        let left = s.join("coordinate/checkouts/.job_1.removing");
        let left_mirror = s.join("mirrors/o/.r.git.removing");
        fs::create_dir_all(left.join("src")).unwrap();
        fs::create_dir_all(&left_mirror).unwrap();
        age(&left, 30 * D);
        let p = plan(&s, Retention::of(1, None), SystemTime::now());
        assert!(removed(&p).is_empty(), "{:?}", removed(&p));
        apply(&s, Retention::of(1, None), p, SystemTime::now());
        assert!(!left.exists() && !left_mirror.exists());
    }

    #[test]
    fn retention_is_counted_in_days_and_gigabytes() {
        let r = Retention::of(14, Some(50));
        assert_eq!(r.keep, Duration::from_secs(14 * 86_400));
        assert_eq!(r.max_bytes, Some(50_000_000_000));
        assert_eq!(gb(1_250_000_000), "1.2 GB");
    }
}
