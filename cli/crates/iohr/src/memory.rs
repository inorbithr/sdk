//! A project's memory, local first (RFC 0205, ADR 0216, RFC 0124.1): one Markdown file per
//! memory in `.inorbit/memory/` (committed with the code) or in the person's own notes, found,
//! checked and compared with the code on this machine, and synced with InOrbit's memory
//! service only when the person turns that on or runs `iohr memory sync`.
//!
//! - [`store`]: the files, the front matter, `remember`.
//! - [`recall`]: ranking by topic.
//! - [`check`]: dangling refs, missing why, stale and duplicate memories, uncovered topics.
//! - [`drift`]: refs changed in git after a memory was last confirmed.
//! - [`remote`]: the memory service's shapes and the plan that compares both sides.
//!
//! Everything here is a pure function over text or a folder; the commands print. Memory text
//! is never logged, and never in a URL.

pub mod check;
pub mod drift;
pub mod recall;
pub mod remote;
pub mod seed;
pub mod store;
pub mod text;

use std::path::{Path, PathBuf};

pub use store::{Entry, Kind, Remembered, Scope, Store};

/// The project root for `from`, the one rule for memory, the notes and `IO.md` files: the
/// nearest folder up with `inorbit.toml`, `.inorbit/` or `atlas/`; failing that the nearest
/// with a `.git`. The home folder's own `.inorbit/` is the person's global state, not a
/// project: it counts only when it is also a git repository. The walk stops at the home
/// folder: a folder above it is never the person's project.
#[must_use]
pub fn find_root(from: &Path) -> Option<PathBuf> {
    let home = etcetera::home_dir()
        .ok()
        .map(|h| h.canonicalize().unwrap_or(h));
    find_root_below(from, home.as_deref())
}

/// [`find_root`] with the home folder given, canonical.
pub(crate) fn find_root_below(from: &Path, home: Option<&Path>) -> Option<PathBuf> {
    walk_up(from, home, |d, is_home| {
        !is_home
            && (d.join("inorbit.toml").is_file()
                || d.join(".inorbit").is_dir()
                || d.join("atlas").is_dir())
    })
    .or_else(|| walk_up(from, home, |d, _| d.join(".git").exists()))
}

/// The one walk up from `from` to the home folder (inclusive): the first folder `marker`
/// accepts, given whether it is the home folder. Nothing at or above home after home.
fn walk_up(
    from: &Path,
    home: Option<&Path>,
    marker: impl Fn(&Path, bool) -> bool,
) -> Option<PathBuf> {
    for d in from.ancestors() {
        let is_home = home.is_some_and(|h| h == d || d.canonicalize().is_ok_and(|c| c == h));
        if marker(d, is_home) {
            return Some(d.to_path_buf());
        }
        if is_home {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::find_root_below;

    #[test]
    fn the_walk_stops_at_the_home_folder() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let here = home.join("notes");
        std::fs::create_dir_all(&here).unwrap();
        // A folder above the home looks like a project (another home's `.inorbit/`).
        std::fs::create_dir(t.path().join(".inorbit")).unwrap();
        std::fs::create_dir(home.join(".inorbit")).unwrap();
        let canonical = home.canonicalize().unwrap();
        assert_eq!(find_root_below(&here, Some(&canonical)), None);
        assert_eq!(find_root_below(&home, Some(&canonical)), None);
        // Without a home folder the walk goes on up, and a git home is still a project.
        assert_eq!(find_root_below(&here, None), Some(home.clone()));
        std::fs::create_dir(home.join(".git")).unwrap();
        assert_eq!(find_root_below(&here, Some(&canonical)), Some(home));
    }
}
