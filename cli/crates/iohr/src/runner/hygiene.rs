//! Disk hygiene of the runner's job checkouts (RFC 0189, ADR 0177).
//!
//! Three rules keep many jobs from filling the build host's disk:
//!
//! 1. **One build directory per repository.** A job's gate check and its model's `cargo`
//!    build into `<state>/target/<owner>/<repo>` (`CARGO_TARGET_DIR`), not into a `target/`
//!    of each checkout. Cargo takes a lock on that directory, so two builds of one
//!    repository wait for each other ("Blocking waiting for file lock"): slower together,
//!    but one copy of the dependencies on disk, not one per job.
//! 2. **A checkout goes when its job does.** [`sweep`] releases the checkout of every job
//!    that is no longer held (done, failed, superseded, or left by an earlier run of the
//!    runner): its own `target/` first, then the worktree (`git worktree remove`, which
//!    refuses a dirty one, so unshipped work is never lost; the job's branch stays in the
//!    mirror).
//! 3. **A free-space floor before a lease.** Under [`MIN_FREE_BYTES`] (default 60 GB,
//!    `IOHR_RUNNER_MIN_FREE_GB`, 0 turns it off) free on the checkouts' filesystem, the
//!    runner takes no new lease and says so on its heartbeat (`disk_free_bytes`,
//!    `lease_paused_reason`).
//!
//! The decisions are pure functions and a [`Checkouts`] trait, so tests use a fake.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::env;

/// Free space the checkouts' filesystem must keep for a new lease: 60 GB.
pub(crate) const MIN_FREE_BYTES: u64 = 60 * 1_000_000_000;
/// Sets the floor, in GB; `0` turns the check off.
pub(crate) const MIN_FREE_ENV: &str = "IOHR_RUNNER_MIN_FREE_GB";
/// A checkout younger than this is never swept: its lease may be in the middle of being
/// accepted, which the loop's held set does not show yet.
pub(crate) const GRACE: Duration = Duration::from_secs(120);

/// The floor in bytes: `value` is [`MIN_FREE_ENV`]'s, in GB; unset or not a number is the
/// default.
pub(crate) fn floor_from(value: Option<&str>) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(MIN_FREE_BYTES, |gb| gb.saturating_mul(1_000_000_000))
}

/// The filesystem to watch and the free space it must keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskFloor {
    /// Any path on the checkouts' filesystem.
    pub(crate) volume: PathBuf,
    pub(crate) bytes: u64,
}

impl DiskFloor {
    /// The checkouts are in `workdir`; the floor from [`MIN_FREE_ENV`] (given as `value`).
    pub(crate) fn for_workdir(workdir: &Path, value: Option<&str>) -> Self {
        Self {
            volume: workdir.to_path_buf(),
            bytes: floor_from(value),
        }
    }
}

/// Why no new lease is taken, or `None` when there is room. Unknown free space (`df`
/// failed) never pauses: a broken probe must not stop the runner for good.
pub(crate) fn pause_reason(free: Option<u64>, floor: u64) -> Option<String> {
    let free = free?;
    (free < floor).then(|| {
        format!(
            "{} GB free on the checkouts' disk, under the {} GB floor",
            free / 1_000_000_000,
            floor / 1_000_000_000
        )
    })
}

/// `owner/name` parts: letters, digits, `-`, `_`, `.`, never `.` or `..` alone.
fn sane(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The shared build directory of a checkout's repository, from the checkout's `.git`
/// file (`gitdir: <state>/mirrors/<owner>/<repo>.git/worktrees/<job>`). `None` for
/// anything that is not one of this runner's worktrees.
pub(crate) fn target_from_gitfile(text: &str, state: &Path) -> Option<PathBuf> {
    let gitdir = Path::new(text.lines().next()?.strip_prefix("gitdir:")?.trim());
    let mirror = gitdir.parent()?.parent()?;
    let owner_dir = mirror.parent()?;
    if owner_dir.parent()? != state.join("mirrors") {
        return None;
    }
    let owner = owner_dir.file_name()?.to_str()?;
    let name = mirror.file_name()?.to_str()?.strip_suffix(".git")?;
    (sane(owner) && sane(name)).then(|| state.join("target").join(owner).join(name))
}

/// [`target_from_gitfile`] for the checkout at `checkout`, under the runner's `workdir`
/// (whose parent is the state directory).
pub(crate) fn target_for(workdir: &Path, checkout: &Path) -> Option<PathBuf> {
    let state = workdir.parent()?;
    let text = std::fs::read_to_string(checkout.join(".git")).ok()?;
    target_from_gitfile(&text, state)
}

/// The variable that points a child's `cargo` at the shared directory.
pub(crate) const TARGET_VAR: &str = "CARGO_TARGET_DIR";

/// `env` with [`TARGET_VAR`] set to `target` (replacing any other).
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

/// The job checkouts on disk, and how one is released. A fake in tests.
pub(crate) trait Checkouts {
    /// The names of the directories in `checkouts/`.
    fn names(&self) -> io::Result<Vec<String>>;
    /// How long since the checkout was made or last found again; `None` if unknown.
    fn age(&self, name: &str) -> Option<Duration>;
    /// Removes the checkout's `target/` and its worktree; an error says why it stays.
    fn release(&self, name: &str) -> Result<(), String>;
}

/// What one [`sweep`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Swept {
    pub(crate) released: Vec<String>,
    /// Checkouts that could not be released, with why (a dirty worktree, git failing).
    pub(crate) kept: Vec<(String, String)>,
}

/// Releases every checkout whose job is not in `held`, skipping those younger than
/// `grace`. One failing never stops the rest.
pub(crate) fn sweep(fs: &dyn Checkouts, held: &HashSet<String>, grace: Duration) -> Swept {
    let mut out = Swept::default();
    let Ok(names) = fs.names() else {
        return out;
    };
    for name in names {
        if !name.starts_with("job_") || held.contains(&name) {
            continue;
        }
        if fs.age(&name).is_some_and(|a| a < grace) {
            continue;
        }
        match fs.release(&name) {
            Ok(()) => out.released.push(name),
            Err(e) => out.kept.push((name, e)),
        }
    }
    out
}

/// The checkouts under `<workdir>/checkouts` on this machine's disk.
#[derive(Debug, Clone)]
pub(crate) struct DiskCheckouts {
    pub(crate) workdir: PathBuf,
}

impl DiskCheckouts {
    fn dir(&self) -> PathBuf {
        self.workdir.join("checkouts")
    }

    fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env_clear()
            .envs(env::scrub(std::env::vars_os()))
            .envs(env::contained())
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("git did not run: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(format!(
                "git {} exited {}: {}",
                args.first().unwrap_or(&""),
                out.status.code().unwrap_or(-1),
                err.lines().next().unwrap_or_default()
            ))
        }
    }
}

impl Checkouts for DiskCheckouts {
    fn names(&self) -> io::Result<Vec<String>> {
        match std::fs::read_dir(self.dir()) {
            Ok(rd) => Ok(rd
                .filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    fn age(&self, name: &str) -> Option<Duration> {
        std::fs::metadata(self.dir().join(name))
            .ok()?
            .modified()
            .ok()?
            .elapsed()
            .ok()
    }

    fn release(&self, name: &str) -> Result<(), String> {
        let path = self.dir().join(name);
        match std::fs::remove_dir_all(path.join("target")) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("its target/ could not be removed: {e}")),
        }
        let common = Self::git(
            &path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        // Without --force: a worktree with changes the runner did not ship stays.
        let p = path.to_string_lossy().into_owned();
        Self::git(Path::new(&common), &["worktree", "remove", &p])?;
        Self::git(Path::new(&common), &["worktree", "prune"])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;
    use std::cell::RefCell;

    const GB: u64 = 1_000_000_000;

    #[test]
    fn under_the_floor_no_lease_is_taken_and_the_reason_says_how_much() {
        let why = pause_reason(Some(39 * GB), MIN_FREE_BYTES).unwrap();
        assert!(why.contains("39 GB free") && why.contains("60 GB floor"), "{why}");
        assert_eq!(pause_reason(Some(60 * GB), MIN_FREE_BYTES), None);
        assert_eq!(pause_reason(Some(149 * GB), MIN_FREE_BYTES), None);
    }

    #[test]
    fn an_unknown_free_space_or_a_zero_floor_never_pauses() {
        assert_eq!(pause_reason(None, MIN_FREE_BYTES), None);
        assert_eq!(pause_reason(Some(0), 0), None);
    }

    #[test]
    fn the_floor_is_60_gb_unless_the_variable_says_otherwise() {
        assert_eq!(floor_from(None), 60 * GB);
        assert_eq!(floor_from(Some("100")), 100 * GB);
        assert_eq!(floor_from(Some(" 5 ")), 5 * GB);
        assert_eq!(floor_from(Some("0")), 0);
        assert_eq!(floor_from(Some("lots")), 60 * GB);
        assert_eq!(floor_from(Some("-1")), 60 * GB);
        let f = DiskFloor::for_workdir(Path::new("/r/coordinate"), Some("10"));
        assert_eq!((f.volume, f.bytes), (PathBuf::from("/r/coordinate"), 10 * GB));
    }

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
        assert_ne!(target_from_gitfile(other, state), target_from_gitfile(git, state));
        let same = "gitdir: /s/runner/mirrors/inorbithr/core.git/worktrees/job_9";
        assert_eq!(target_from_gitfile(same, state), target_from_gitfile(git, state));
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

    /// A disk of checkouts: name, age, and whether releasing it fails.
    #[derive(Default)]
    struct Fake {
        dirs: RefCell<Vec<(String, Option<Duration>, Option<String>)>>,
        released: RefCell<Vec<String>>,
        unreadable: bool,
    }

    impl Fake {
        fn with(dirs: &[(&str, Option<u64>, Option<&str>)]) -> Self {
            Self {
                dirs: RefCell::new(
                    dirs.iter()
                        .map(|(n, a, f)| {
                            ((*n).to_owned(), a.map(Duration::from_secs), f.map(str::to_owned))
                        })
                        .collect(),
                ),
                ..Self::default()
            }
        }
    }

    impl Checkouts for Fake {
        fn names(&self) -> io::Result<Vec<String>> {
            if self.unreadable {
                return Err(io::Error::other("denied"));
            }
            Ok(self.dirs.borrow().iter().map(|d| d.0.clone()).collect())
        }
        fn age(&self, name: &str) -> Option<Duration> {
            self.dirs.borrow().iter().find(|d| d.0 == name)?.1
        }
        fn release(&self, name: &str) -> Result<(), String> {
            let mut dirs = self.dirs.borrow_mut();
            let i = dirs.iter().position(|d| d.0 == name).unwrap();
            if let Some(why) = dirs[i].2.clone() {
                return Err(why);
            }
            dirs.remove(i);
            self.released.borrow_mut().push(name.to_owned());
            Ok(())
        }
    }

    fn held(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_job_no_longer_leased_loses_its_checkout_and_a_held_one_keeps_it() {
        let fs = Fake::with(&[
            ("job_done", Some(3600), None),
            ("job_held", Some(3600), None),
            ("job_gone", Some(900), None),
        ]);
        let out = sweep(&fs, &held(&["job_held"]), GRACE);
        assert_eq!(out.released, ["job_done", "job_gone"]);
        assert!(out.kept.is_empty());
        assert_eq!(fs.names().unwrap(), ["job_held"]);
    }

    #[test]
    fn a_checkout_just_made_is_left_for_its_lease_to_finish_accepting() {
        let fs = Fake::with(&[("job_new", Some(5), None), ("job_unknown_age", None, None)]);
        let out = sweep(&fs, &held(&[]), GRACE);
        assert_eq!(out.released, ["job_unknown_age"]);
        assert_eq!(fs.names().unwrap(), ["job_new"]);
    }

    #[test]
    fn a_checkout_that_cannot_be_released_stays_and_the_rest_go() {
        let fs = Fake::with(&[
            ("job_a", Some(3600), None),
            ("job_dirty", Some(3600), Some("worktree has changes")),
            ("job_b", Some(3600), None),
        ]);
        let out = sweep(&fs, &held(&[]), GRACE);
        assert_eq!(out.released, ["job_a", "job_b"]);
        assert_eq!(
            out.kept,
            [("job_dirty".to_owned(), "worktree has changes".to_owned())]
        );
        assert_eq!(fs.names().unwrap(), ["job_dirty"]);
    }

    #[test]
    fn only_job_directories_are_touched_and_a_second_sweep_does_nothing() {
        let fs = Fake::with(&[("notes", Some(9999), None), ("job_x", Some(9999), None)]);
        let first = sweep(&fs, &held(&[]), GRACE);
        assert_eq!(first.released, ["job_x"]);
        assert_eq!(sweep(&fs, &held(&[]), GRACE), Swept::default());
        assert_eq!(fs.names().unwrap(), ["notes"]);
        let broken = Fake {
            unreadable: true,
            ..Fake::default()
        };
        assert_eq!(sweep(&broken, &held(&[]), GRACE), Swept::default());
    }

    /// The real thing, on a real worktree: its `target/` and the worktree go, the
    /// branch stays, and a worktree with unshipped changes stays whole.
    #[test]
    fn the_disk_release_removes_target_and_worktree_but_not_unshipped_work() {
        let sh = |dir: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(ok.status.success(), "{args:?}: {ok:?}");
        };
        let d = tempfile::tempdir().unwrap();
        let state = d.path();
        let workdir = state.join("coordinate");
        let mirror = state.join("mirrors/o/r.git");
        std::fs::create_dir_all(&mirror).unwrap();
        sh(&mirror, &["init", "--bare", "--quiet", "-b", "main"]);
        let seed = state.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        sh(&seed, &["init", "--quiet", "-b", "main"]);
        std::fs::write(seed.join("f"), "x").unwrap();
        sh(&seed, &["add", "f"]);
        sh(&seed, &["commit", "--quiet", "-m", "one"]);
        sh(&seed, &["push", "--quiet", mirror.to_str().unwrap(), "main"]);
        let checkouts = workdir.join("checkouts");
        std::fs::create_dir_all(&checkouts).unwrap();
        for id in ["job_clean", "job_dirty"] {
            let p = checkouts.join(id);
            sh(
                &mirror,
                &["worktree", "add", "--quiet", "-b", id, p.to_str().unwrap(), "main"],
            );
            std::fs::create_dir_all(p.join("target/debug")).unwrap();
            std::fs::write(p.join("target/debug/big"), "x").unwrap();
        }
        std::fs::write(checkouts.join("job_dirty/unshipped.txt"), "work").unwrap();

        let target = target_for(&workdir, &checkouts.join("job_clean")).unwrap();
        assert_eq!(target, state.join("target/o/r"));

        let disk = DiskCheckouts {
            workdir: workdir.clone(),
        };
        let out = sweep(&disk, &held(&[]), Duration::ZERO);
        assert_eq!(out.released, ["job_clean"]);
        assert_eq!(out.kept.len(), 1, "{out:?}");
        assert!(!checkouts.join("job_clean").exists());
        assert!(checkouts.join("job_dirty/unshipped.txt").is_file());
        assert!(!checkouts.join("job_dirty/target").exists());
        // The branch of the released job is still in the mirror.
        sh(&mirror, &["rev-parse", "--verify", "refs/heads/job_clean"]);
    }
}
