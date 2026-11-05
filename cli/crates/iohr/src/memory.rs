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

/// The project root for `from`: the nearest folder up that has `.inorbit/`, `atlas/` or a
/// `.git`. The home folder's own `.inorbit/` is the person's global state, not a project: it
/// counts only when it is also a git repository.
#[must_use]
pub fn find_root(from: &Path) -> Option<PathBuf> {
    let home = etcetera::home_dir()
        .ok()
        .map(|h| h.canonicalize().unwrap_or(h));
    from.ancestors()
        .find(|d| {
            let is_home = home
                .as_deref()
                .is_some_and(|h| h == *d || d.canonicalize().is_ok_and(|c| c == h));
            d.join(".git").exists()
                || (!is_home && (d.join(".inorbit").is_dir() || d.join("atlas").is_dir()))
        })
        .map(Path::to_path_buf)
}
