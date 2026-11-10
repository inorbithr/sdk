//! The where-to-save question of `iohr project init` (light PRD 0210).
//!
//! The answer is `[project] data` in inorbit.toml; every writer asks
//! [`crate::project::place::Layout`] where to put things, so this file only decides the answer
//! and applies it (record it, ignore the machine state, seed the notes, copy on request).
//! Precedence: `--keep-in` (or `IOHR_INIT_KEEP_IN`), then what the project recorded, then
//! the question at a terminal, else nothing is recorded and the older layout stays.

use std::fmt::Write as _;
use std::path::Path;

use crate::cli::InitArgs;
use crate::error::Error;
use crate::project::place::{self, Layout, Place};
use crate::project::{Step, step};
use crate::ui::{Choice, Ui};

/// What init settled about the place.
pub(super) struct Decision {
    /// Where the data goes from now on.
    pub layout: Layout,
    /// Where it was before this run (what a copy reads).
    old: Layout,
    /// Copy the old place's files into the new one.
    copy: bool,
}

/// The place for the project at `dir` named `name`.
///
/// # Errors
/// A bad `--keep-in`, or the person cancelling the question.
pub(super) fn decide(
    a: &InitArgs,
    dir: &Path,
    name: &str,
    interactive: bool,
    ui: Ui,
) -> Result<Decision, Error> {
    let old = Layout::of(dir);
    let asked = a
        .place_args
        .keep_in
        .as_deref()
        .map(Place::parse)
        .transpose()
        .map_err(Error::Usage)?;
    let chosen = match asked.or(old.place()) {
        Some(p) => Some(p),
        None if interactive => Some(ask(ui, name)?),
        None => None,
    };
    let layout = Layout::new(dir, name, chosen);
    let copy = chosen != old.place()
        && chosen.is_some()
        && place::movable(&old, &layout)
        && (a.place_args.copy_data
            || (interactive
                && ui.confirm(
                    "Copy what is kept in the old place into the new one? The old copy stays.",
                    true,
                )?));
    Ok(Decision { layout, old, copy })
}

fn ask(ui: Ui, name: &str) -> Result<Place, Error> {
    let choice = |value: &str, label: &str, hint: String| Choice {
        value: value.into(),
        label: label.into(),
        hint,
        available: true,
        key: None,
    };
    let said = ui.select(
        "Where should this project keep its ADRs, notes and crawl?",
        &[
            choice(
                "project",
                "In the project folder",
                ".inorbit/ in the project: travels with the repository".into(),
            ),
            choice(
                "home",
                "In your home folder",
                format!("~/.inorbit/projects/{name}/: stays on this machine"),
            ),
        ],
        "project",
    )?;
    Place::parse(&said).map_err(Error::Usage)
}

impl Decision {
    /// What a dry run says.
    pub(super) fn would(&self) -> String {
        let at: Vec<String> = self
            .layout
            .describe()
            .into_iter()
            .map(|(k, p)| format!("{k} in {}", self.layout.shown(&p)))
            .collect();
        let mut s = format!("keep the project's data: {}", at.join(", "));
        if let Some(p) = self.layout.place() {
            let _ = write!(s, " (recorded as [project] data = \"{}\")", p.word());
        }
        if self.copy {
            s.push_str("; copy the old place's files over, deleting nothing");
        }
        s
    }

    /// True when the starter notes of the scaffold have to be written by [`Decision::apply`]
    /// (they sit outside the project folder).
    pub(super) fn seeds_elsewhere(&self) -> bool {
        self.layout.memory_rel().is_none()
    }

    /// Records the choice, ignores the machine state when it sits in the project, writes the
    /// starter notes that belong outside it, and copies when asked. Nothing is deleted.
    ///
    /// # Errors
    /// Any file that cannot be written.
    pub(super) fn apply(&self, dir: &Path, seed: &[(String, String)]) -> Result<Step, String> {
        let l = &self.layout;
        if let Some(p) = l.place() {
            if Layout::of(dir).place() != Some(p) {
                place::record(dir, p)?;
            }
            if p == Place::Project {
                place::ignore_state(dir)?;
            }
        }
        for (rel, text) in seed {
            let tail = rel.strip_prefix(".inorbit/memory/").unwrap_or(rel);
            let to = l.memory().join(tail);
            if !to.exists() {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                std::fs::write(&to, text)
                    .map_err(|e| format!("cannot write {}: {e}", to.display()))?;
            }
        }
        let mut said = l
            .describe()
            .into_iter()
            .map(|(k, p)| format!("{k}: {}", l.shown(&p)))
            .collect::<Vec<_>>()
            .join("; ");
        if self.copy {
            let moved = place::copy_layout(&self.old, l)?;
            let n: usize = moved.iter().map(|(_, n)| n).sum();
            let _ = write!(said, "; copied {n} files, the old copy stays");
        }
        if l.place().is_none() {
            said.push_str("; no choice recorded, the older layout stays (--keep-in project|home)");
        }
        Ok(step("place", "Where it keeps its data", "done", said, ""))
    }
}

/// The scaffold's files with the starter notes taken out when they belong outside the
/// project folder; the taken-out files are returned to write later.
pub(super) fn split_seed(d: &Decision, files: &mut Vec<(String, String)>) -> Vec<(String, String)> {
    if !d.seeds_elsewhere() {
        return Vec::new();
    }
    let (seed, keep): (Vec<_>, Vec<_>) = std::mem::take(files)
        .into_iter()
        .partition(|(p, _)| p.starts_with(".inorbit/memory/"));
    *files = keep;
    seed
}
