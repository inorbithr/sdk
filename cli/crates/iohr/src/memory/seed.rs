//! What a new project starts with in `.inorbit/memory/`: a README and the built-in
//! personalities' recall lines as memories, written locally with no network. Used by
//! `iohr project init` and `iohr atlas init`.

use std::path::Path;

use super::store::{Entry, Kind, PROJECT_DIR, Scope};
use crate::personalities;

/// The README's path, relative to the project root.
#[must_use]
pub fn readme_path() -> String {
    format!("{PROJECT_DIR}/README.md")
}

/// The seed memories of a project of `lang`: the recall lines of the personalities it gets
/// (`security` and its language's engineer), each as a fact that names its source.
#[must_use]
pub fn entries(lang: Option<&str>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for p in personalities::selection(lang) {
        for l in p.lines.iter().filter(|l| l.place == "recall") {
            let why = format!(
                "seed from the {} personality (catalogue v{}); source {}",
                p.title,
                p.version,
                l.cites.first().map_or("", String::as_str)
            );
            let e = Entry::new(&l.text, Kind::Fact, &why, Vec::new(), false);
            if !out.iter().any(|o| o.id == e.id) {
                out.push(e);
            }
        }
    }
    out
}

/// Every file a new project gets under `.inorbit/memory/`, as relative path and text.
#[must_use]
pub fn files(lang: Option<&str>, slug: &str) -> Vec<(String, String)> {
    let mut out = vec![(readme_path(), readme(slug))];
    out.extend(entries(lang).into_iter().map(|e| {
        (
            format!("{PROJECT_DIR}/{}.md", e.id),
            e.render(Scope::Project),
        )
    }));
    out
}

/// Whether the project at `root` already keeps memories there: any entry file besides the
/// README. A person who forgot a seed keeps it forgotten.
#[must_use]
pub fn has_entries(root: &Path) -> bool {
    std::fs::read_dir(root.join(PROJECT_DIR)).is_ok_and(|rd| {
        rd.filter_map(Result::ok).any(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.ends_with(".md") && n != "README.md"
        })
    })
}

/// `.inorbit/memory/README.md`: how the project's agents remember and recall, locally first.
#[must_use]
pub fn readme(slug: &str) -> String {
    format!(
        "\
# Project memory

What this project's people and agents have learned, kept as one Markdown file per memory in
this folder and committed with the code. It lives on this machine and in git. Nothing here
goes to InOrbit unless you turn sync on or run `iohr memory sync`.

| Do | Command |
| --- | --- |
| Look up what is known before you start | `iohr memory recall \"<topic>\"` |
| Keep a lesson or a fact | `iohr memory remember \"<fact>\" --why \"<reason>\" --ref <path>` |
| See what is kept | `iohr memory list` |
| Drop one | `iohr memory forget <id>` |
| Find rotten or missing memories | `iohr memory check` |
| See what the code changed since | `iohr memory drift` |
| Compare and sync with your InOrbit account (optional) | `iohr memory sync --dry-run` |

A file starts with front matter: `id`, `kind` (fact, preference, episode), `scope`, `created`,
`updated`, `why`, `pinned` and `refs` (the paths and `cmd:` commands the memory is about); the
text follows. Telling `remember` the same text again confirms it: `updated` moves to now.
`remote_id`, `remote_version` and `remote_sha` appear after a sync and stay on this machine.

Scopes: `project` (this folder, the default), `user` (your own notes on this machine, in every
project) and `account` (the InOrbit memory service only, needs sync on). Set the default with
`scope` under `[memory]` in the iohr config; `--scope` overrides it per command. When synced,
this project's entries carry the source `project:{slug}`.

Keep memories short and true, and give each a reason. Never put secrets, personal data or
anything owner-only in memory: `remember` refuses what looks like a secret, and this folder is
committed.
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rust_project_gets_the_readme_and_the_recall_lines_of_two_personalities() {
        let f = files(Some("rust"), "x");
        assert_eq!(f[0].0, ".inorbit/memory/README.md");
        assert!(f.len() > 5, "{}", f.len());
        assert!(f.iter().all(|(p, _)| p.starts_with(".inorbit/memory/")));
        let e = entries(Some("rust"));
        assert!(e.iter().all(|e| e.why.contains("source https://")));
        assert!(e.iter().all(|e| e.refs.is_empty()));
    }

    #[test]
    fn no_language_still_seeds_the_security_personality() {
        assert_ne!(entries(None).len(), 0);
    }

    #[test]
    fn the_seeds_read_back_as_the_same_memories() {
        for (path, text) in files(Some("go"), "x").into_iter().skip(1) {
            let stem = path
                .rsplit('/')
                .next()
                .and_then(|n| n.strip_suffix(".md"))
                .unwrap_or_default();
            let e = Entry::parse(&text, stem).expect("a seed parses");
            assert_eq!(e.id, stem);
            assert_eq!(e.render(Scope::Project), text);
        }
    }
}
