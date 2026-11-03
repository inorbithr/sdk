//! Disk hygiene of the runner's job checkouts (RFC 0189, ADR 0177).
//!
//! **One build directory per repository.** A job's gate check and its model's `cargo`
//! build into `<state>/target/<owner>/<repo>` (`CARGO_TARGET_DIR`), not into a `target/`
//! in each checkout. Cargo locks that directory, so two builds of one repository take
//! turns ("Blocking waiting for file lock"): slower together, but one copy of the
//! dependencies on disk, not one per job.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::workspace::is_repository_part;

/// The variable that points a child's `cargo` at the shared directory.
pub(crate) const TARGET_VAR: &str = "CARGO_TARGET_DIR";

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
    (is_repository_part(owner) && is_repository_part(name))
        .then(|| state.join("target").join(owner).join(name))
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
}
