//! Next steps as actions (PRD 0187, RFC 0203): the model ends an answer with a short
//! ```` ```next ```` block (`read <subfolder>/`, `read all <subfolder>/`, `search <text>`,
//! `run <command>`); each line
//! becomes a numbered entry in the action bar, never a shell line to copy. A step only ever
//! becomes a `??` question, asked when it is picked: reading and searching run through the same
//! `?` run (its read tools, secrets denied) and a command runs through the permission card.
//! Nothing here runs anything.

use std::path::Path;

use serde::Serialize;

use crate::askback;
use crate::card;
use crate::permissions::secrets;

/// At most this many steps are taken from the model's block; the rest is dropped.
pub(crate) const MAX: usize = 3;
/// One recursive read ([`Step::ReadAll`]) may be added beside those when a folder the model
/// offered has subfolders and the model did not offer one itself.
const EXTRA_ALL: usize = 1;
/// The most steps ever offered: what the hook sends back to the menu is cut to this.
pub(crate) const OFFERED: usize = MAX + EXTRA_ALL;
/// How many entries of a folder are looked at to find a subfolder: one `read_dir`, no walk.
const PEEK: usize = 32;
/// Folders that hold other people's code or build output: not a reason to offer a recursive read.
const NOISE: [&str; 2] = ["node_modules", "target"];
/// The longest text of a search step.
const SEARCH_MAX: usize = 80;

/// One next step the model suggested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// A folder under the working folder, with no trailing slash.
    Read(String),
    /// The same kind of folder, with everything under it: Io goes through its subfolders too.
    ReadAll(String),
    /// Text to look for in the working folder.
    Search(String),
    /// A one-line command to run through the permission card.
    Run(String),
}

/// A step as the JSON of `--json` shows it.
#[derive(Debug, Serialize)]
pub(crate) struct Shown {
    pub(crate) kind: &'static str,
    pub(crate) value: String,
}

impl Step {
    fn kind(&self) -> &'static str {
        match self {
            Self::Read(_) => "read",
            Self::ReadAll(_) => "readall",
            Self::Search(_) => "search",
            Self::Run(_) => "run",
        }
    }

    fn value(&self) -> &str {
        match self {
            Self::Read(v) | Self::ReadAll(v) | Self::Search(v) | Self::Run(v) => v,
        }
    }

    /// The name of the entry in the bar.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Read(v) => format!("Read {v}/"),
            Self::ReadAll(v) => format!("Read {v}/ (all)"),
            Self::Search(v) => format!("Search for {v}"),
            Self::Run(v) => format!("Run {v}"),
        }
    }

    /// What picking it does, in a few words.
    pub(crate) fn hint(&self) -> &'static str {
        match self {
            Self::Read(_) => "ask again about that folder",
            Self::ReadAll(_) => "ask again about that folder and every folder under it",
            Self::Search(_) => "ask Io to search for it here",
            Self::Run(_) => "ask Io to run it; you approve it on the card",
        }
    }

    /// The `??` line put on the prompt when it is picked.
    pub(crate) fn question(&self) -> String {
        match self {
            Self::Read(v) => format!("?? explain what is in {v}/"),
            Self::ReadAll(v) => format!(
                "?? go through {v}/ and every folder under it, one folder at a time, and explain what is in each"
            ),
            Self::Search(v) => format!("?? search this folder for {v} and show where it is"),
            Self::Run(v) => format!("?? run `{v}` and tell me what it shows"),
        }
    }

    pub(crate) fn shown(&self) -> Shown {
        Shown {
            kind: self.kind(),
            value: self.value().to_owned(),
        }
    }

    /// `kind:value`, for the hook's record and the menu's flags.
    pub(crate) fn encode(&self) -> String {
        format!("{}:{}", self.kind(), self.value())
    }

    /// The step an [`encode`](Self::encode)d text names; none for anything else (it is read
    /// back from a shell argument, so it is checked again: no control character, a known
    /// kind, a value that is not empty).
    pub(crate) fn decode(text: &str) -> Option<Self> {
        let (kind, value) = text.split_once(':')?;
        let value = value.trim();
        if value.is_empty() || value.chars().any(char::is_control) {
            return None;
        }
        match kind {
            "read" => Some(Self::Read(value.to_owned())),
            "readall" => Some(Self::ReadAll(value.to_owned())),
            "search" => Some(Self::Search(value.to_owned())),
            "run" => Some(Self::Run(value.to_owned())),
            _ => None,
        }
    }
}

/// A folder the model offered to read: it must be a folder under `cwd` (no `..`, not
/// absolute, not a secret place), so a step never sends a run outside what was shown.
fn folder_step(value: &str, cwd: &Path) -> Option<Step> {
    let v = value.trim().trim_matches('`').trim_end_matches('/');
    let v = v.strip_prefix("./").unwrap_or(v);
    let plain = !v.is_empty()
        && v.len() <= 200
        && !v.starts_with('/')
        && !v.starts_with('~')
        && !v.split('/').any(|p| p == ".." || p.is_empty())
        && !v.chars().any(char::is_control);
    let secret = secrets::is_secret_path(v, cwd, None);
    // The folder as the system names it (a symlink is followed): it must still be under `cwd`
    // and not a secret place, so a link named `docs` that points at `~/.ssh` is not offered.
    let inside = plain && !secret && {
        let there =
            crate::access::allowable_at(&cwd.join(v), cwd, None, crate::access::Source::Trust).ok();
        let here = cwd.canonicalize().ok();
        matches!((there, here), (Some(t), Some(h)) if t.starts_with(&h))
    };
    inside.then(|| Step::Read(v.to_owned()))
}

/// Whether the folder `rel` (under `cwd`) has a subfolder worth going through: one `read_dir`,
/// at most [`PEEK`] entries looked at, so it is cheap enough to ask for every step. A hidden
/// folder, a link, a build folder ([`NOISE`]) and a secret place do not count.
fn has_subfolder(rel: &str, cwd: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(cwd.join(rel)) else {
        return false;
    };
    entries.take(PEEK).filter_map(Result::ok).any(|e| {
        // `file_type` does not follow a link: a link to a folder is not a subfolder here.
        let dir = e.file_type().is_ok_and(|t| t.is_dir());
        e.file_name().to_str().is_some_and(|n| {
            dir && !n.starts_with('.')
                && !NOISE.contains(&n)
                && !secrets::is_secret_path(&format!("{rel}/{n}"), cwd, None)
        })
    })
}

/// The folder of a `read all <folder>/` or `read <folder>/ (all)` line, when it is one.
fn all_of(rest: &str) -> Option<&str> {
    let rest = rest.trim();
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("all ") {
        Some(rest[4..].trim())
    } else {
        lower
            .ends_with("(all)")
            .then(|| rest[..rest.len() - 5].trim())
    }
}

/// One line of the block as a step; none when it is not one of the three, or its value does
/// not hold (`command` is the answer's own command: a step that repeats it is dropped, the
/// bar already has Run).
fn parse(line: &str, cwd: &Path, command: Option<&str>) -> Option<Step> {
    let line = line
        .trim()
        .trim_start_matches(['-', '*', '\u{2022}'])
        .trim();
    let (head, rest) = line.split_once(char::is_whitespace)?;
    let rest = rest.trim();
    match head.to_ascii_lowercase().as_str() {
        "read" => match all_of(rest) {
            // The model asked for a recursive read: kept only when the folder has subfolders.
            Some(folder) => match folder_step(folder, cwd)? {
                Step::Read(v) if has_subfolder(&v, cwd) => Some(Step::ReadAll(v)),
                step => Some(step),
            },
            None => folder_step(rest, cwd),
        },
        "search" => {
            let t = rest.trim_matches(['"', '\'', '`']).trim();
            (!t.is_empty() && t.chars().count() <= SEARCH_MAX && !t.chars().any(char::is_control))
                .then(|| Step::Search(t.to_owned()))
        }
        "run" => {
            let c = card::insertable(rest.trim_matches('`'))?;
            (command != Some(c)).then(|| Step::Run(c.to_owned()))
        }
        _ => None,
    }
}

/// Splits the model's answer into what is shown and the steps it suggested: the last
/// ```` ```next ```` block is removed from the text (even when never closed) and read line by
/// line, at most [`MAX`] steps, no repeats. A line that is not a valid step is dropped.
pub(crate) fn split(text: &str, cwd: &Path, command: Option<&str>) -> (String, Vec<Step>) {
    let (kept, block) = askback::take_block(text, "next");
    let mut steps: Vec<Step> = Vec::new();
    for s in block
        .iter()
        .flat_map(|b| b.lines())
        .filter_map(|l| parse(l, cwd, command))
    {
        if !steps.contains(&s) && steps.len() < MAX {
            steps.push(s);
        }
    }
    offer_all(&mut steps, cwd);
    (kept, steps)
}

/// Best effort, no walk: when no step reads a folder with everything under it, the first
/// folder the model offered that has a subfolder gets a [`Step::ReadAll`] right after it.
fn offer_all(steps: &mut Vec<Step>, cwd: &Path) {
    let have = steps
        .iter()
        .filter(|s| matches!(s, Step::ReadAll(_)))
        .count();
    if have >= EXTRA_ALL {
        return;
    }
    let at = steps.iter().position(|s| match s {
        Step::Read(v) => !steps.contains(&Step::ReadAll(v.clone())) && has_subfolder(v, cwd),
        _ => false,
    });
    if let Some(i) = at {
        let all = Step::ReadAll(steps[i].value().to_owned());
        steps.insert(i + 1, all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("api/src")).unwrap();
        std::fs::create_dir_all(d.path().join("docs")).unwrap();
        std::fs::write(d.path().join("a.txt"), "x").unwrap();
        d
    }

    #[test]
    fn the_next_block_becomes_steps_and_leaves_the_text() {
        let d = dir();
        let text = "It is a service.\n\n```next\nread api/\nsearch TODO\nrun cargo test\n```\n\n```sh\nls\n```";
        let (shown, steps) = split(text, d.path(), Some("ls"));
        assert_eq!(shown, "It is a service.\n\n\n```sh\nls\n```");
        assert!(!shown.contains("next"));
        assert_eq!(
            steps,
            vec![
                Step::Read("api".into()),
                Step::ReadAll("api".into()),
                Step::Search("TODO".into()),
                Step::Run("cargo test".into()),
            ]
        );
    }

    #[test]
    fn labels_read_as_actions_not_as_shell_lines() {
        assert_eq!(Step::Read("api".into()).label(), "Read api/");
        assert_eq!(Step::Search("TODO".into()).label(), "Search for TODO");
        assert_eq!(Step::Run("cargo test".into()).label(), "Run cargo test");
        assert_eq!(
            Step::Read("api".into()).question(),
            "?? explain what is in api/"
        );
        assert!(Step::Run("cargo test".into()).question().starts_with("?? "));
    }

    #[test]
    fn at_most_three_and_no_repeats() {
        let d = dir();
        let text = "x\n```next\nsearch a\nsearch a\nsearch b\nsearch c\nsearch d\n```";
        let (_, steps) = split(text, d.path(), None);
        assert_eq!(steps.len(), MAX);
        assert_eq!(steps[1], Step::Search("b".into()));
    }

    #[test]
    fn a_step_that_would_leave_the_folder_or_open_a_secret_is_dropped() {
        let d = dir();
        std::fs::create_dir_all(d.path().join(".ssh")).unwrap();
        let text = "x\n```next\nread ../\nread /etc/\nread ~/\nread a.txt\nread nothing/\nread .ssh/\nread api/src\n```";
        let (_, steps) = split(text, d.path(), None);
        assert_eq!(steps, vec![Step::Read("api/src".into())]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_secret_place_or_outside_the_folder_is_not_offered() {
        let d = dir();
        // A fake home with a `.ssh` in it, linked from inside the working folder.
        let other = tempfile::tempdir().unwrap();
        let ssh = other.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::os::unix::fs::symlink(&ssh, d.path().join("keys")).unwrap();
        // A plain folder outside, linked in: not under the working folder either.
        let out = other.path().join("outside");
        std::fs::create_dir_all(&out).unwrap();
        std::os::unix::fs::symlink(&out, d.path().join("elsewhere")).unwrap();
        // A link to a folder inside stays fine.
        std::os::unix::fs::symlink(d.path().join("api"), d.path().join("svc")).unwrap();
        let text = "x\n```next\nread keys/\nread elsewhere/\nread svc/\n```";
        let (_, steps) = split(text, d.path(), None);
        // `api` has a subfolder, so the link to it also offers the recursive read.
        assert_eq!(
            steps,
            vec![Step::Read("svc".into()), Step::ReadAll("svc".into())]
        );
        // The same through the folder the question names: the real path is judged.
        assert!(
            crate::access::named_folders("explain keys/", d.path(), None)
                .next()
                .is_none()
        );
    }

    #[test]
    fn dot_dot_and_absolute_paths_in_a_step_are_dropped_wherever_they_sit() {
        let d = dir();
        let text = "x\n```next\nread api/../..\nread api//src\nread ./../x\nread /\nread //etc\nread ~root\nread `api/src`\n```";
        let (_, steps) = split(text, d.path(), None);
        assert_eq!(steps, vec![Step::Read("api/src".into())]);
    }

    #[test]
    fn a_command_with_a_control_character_or_the_answers_own_command_is_dropped() {
        let d = dir();
        let text = "x\n```next\nrun ls\u{1b}[2J\nrun cargo test\nrun ls -la\n```";
        let (_, steps) = split(text, d.path(), Some("ls -la"));
        assert_eq!(steps, vec![Step::Run("cargo test".into())]);
    }

    #[test]
    fn a_block_never_closed_is_never_shown() {
        let d = dir();
        let (shown, steps) = split("Hello\n```next\nread docs", d.path(), None);
        assert_eq!(shown, "Hello");
        assert_eq!(steps, vec![Step::Read("docs".into())]);
    }

    #[test]
    fn an_answer_without_the_block_is_unchanged() {
        let d = dir();
        let (shown, steps) = split("  just text \n", d.path(), None);
        assert_eq!(shown, "just text");
        assert_eq!(steps.len(), 0);
    }

    #[test]
    fn a_folder_with_subfolders_also_offers_a_recursive_read() {
        let d = dir();
        // `api/` has `src/`; `docs/` is a leaf.
        let (_, steps) = split("x\n```next\nread api/\nread docs/\n```", d.path(), None);
        assert_eq!(
            steps,
            vec![
                Step::Read("api".into()),
                Step::ReadAll("api".into()),
                Step::Read("docs".into()),
            ]
        );
        assert_eq!(steps[1].label(), "Read api/ (all)");
        assert!(
            steps[1]
                .question()
                .starts_with("?? go through api/ and every folder under it")
        );
        // Only one recursive read is added, and the menu may take all of them.
        let (_, steps) = split("x\n```next\nread api/\nread api/src\n```", d.path(), None);
        assert_eq!(
            steps
                .iter()
                .filter(|s| matches!(s, Step::ReadAll(_)))
                .count(),
            1
        );
        assert!(steps.len() <= OFFERED);
        // The question still names its folder, so the permission check sees it.
        assert!(steps[1].question().contains("api/"));
    }

    #[test]
    fn a_leaf_folder_a_hidden_one_a_link_and_build_output_are_not_a_reason() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        for f in [
            "leaf",
            "hid/.git",
            "tgt/target",
            "tgt/node_modules",
            "sec/.ssh",
            "ln",
        ] {
            std::fs::create_dir_all(p.join(f)).unwrap();
        }
        std::fs::write(p.join("leaf/a.md"), "x").unwrap();
        #[cfg(unix)]
        {
            std::fs::create_dir_all(p.join("real")).unwrap();
            std::os::unix::fs::symlink(p.join("real"), p.join("ln/to")).unwrap();
        }
        for f in ["leaf", "hid", "tgt", "sec", "ln", "missing"] {
            assert!(!has_subfolder(f, p), "{f}");
        }
        std::fs::create_dir_all(p.join("leaf/more")).unwrap();
        assert!(has_subfolder("leaf", p));
    }

    #[test]
    fn the_model_may_ask_for_the_recursive_read_and_a_leaf_stays_a_plain_read() {
        let d = dir();
        for line in ["read all api/", "read api/ (all)", "Read ALL ./api"] {
            let (_, steps) = split(&format!("x\n```next\n{line}\n```"), d.path(), None);
            assert_eq!(steps, vec![Step::ReadAll("api".into())], "{line}");
        }
        // `docs/` has no subfolder: the plain read, never a recursive one.
        let (_, steps) = split("x\n```next\nread all docs/\n```", d.path(), None);
        assert_eq!(steps, vec![Step::Read("docs".into())]);
        // A folder that fails the checks is dropped, recursive or not.
        let (_, steps) = split(
            "x\n```next\nread all ../\nread all /etc/\n```",
            d.path(),
            None,
        );
        assert_eq!(steps.len(), 0);
    }

    #[test]
    fn a_recursive_step_survives_the_hook() {
        let s = Step::ReadAll("atlas/adr".into());
        assert_eq!(s.encode(), "readall:atlas/adr");
        assert_eq!(Step::decode(&s.encode()), Some(s));
    }

    #[test]
    fn a_step_survives_the_hook_and_junk_does_not() {
        for s in [
            Step::Read("api".into()),
            Step::Search("two words: here".into()),
            Step::Run("cargo test -p iohr".into()),
        ] {
            assert_eq!(Step::decode(&s.encode()), Some(s));
        }
        assert_eq!(Step::decode("shout:hi"), None);
        assert_eq!(Step::decode("run:"), None);
        assert_eq!(Step::decode("run:ls\u{7}"), None);
        assert_eq!(Step::decode("nocolon"), None);
    }
}
