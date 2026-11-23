//! `iohr monitors install`: the project's monitors as one block of the InOrbit agent's
//! `checks.toml`. It is the install `agent.install-checks` does for core's blocks
//! ([`crate::runner::host_action::install_checks`]): the same BEGIN and END lines, the same
//! lint before the swap (`iohr agent checks lint --file`), the same `.bak-<UTC time>` backup
//! and the same one-rename swap, so the two never differ. What differs is the source (this
//! project's files, not a core commit) and that nothing restarts the agent or reads its status:
//! the person does, and the agent declares the checks in its next hello.
//!
//! A `cli` check never installs: it runs the project's own binary in a throwaway folder on this
//! machine and the agent has no cli surface. Asked for by file name it is refused; folded in
//! with the rest it is left out and counted, as `export` does.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::{File, export};
use crate::runner::host_action::install_checks::{
    MAX_DECLARED_CHECKS, candidate_path, declared, lint_argv, put_block_noted, swap_in,
};

/// What an install did, or with `dry_run` would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The checks file.
    pub file: PathBuf,
    /// The block's name in it.
    pub block: String,
    /// `[[check]]` and `[[refuse]]` entries the file declares after the install.
    pub declared: usize,
    /// Entries this install put in the block.
    pub entries: usize,
    /// cli checks left out.
    pub cli: usize,
    /// Entries with `enabled = false` left out.
    pub off: usize,
    /// The old file's backup; none for a first install or a dry run.
    pub backup: Option<PathBuf>,
    /// Nothing was written.
    pub dry_run: bool,
}

/// A block name for a project folder: lower case letters, digits and `-`.
#[must_use]
pub fn block_for(root: &Path) -> String {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        "project".to_owned()
    } else {
        name.to_owned()
    }
}

/// Whether a block name is one this writes: 1 to 64 of lower case letters, digits and `-`.
#[must_use]
pub fn is_block_name(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_cli(e: &Value) -> bool {
    e.get("surface").and_then(Value::as_str) == Some("cli")
        || e.get("target").is_some_and(|t| t.get("cli").is_some())
}

/// The entries of `files`, as (cli, off, installable) counts.
fn counts(files: &[File]) -> (usize, usize, usize) {
    let (mut cli, mut off, mut ok) = (0, 0, 0);
    for f in files {
        for kind in ["check", "refuse"] {
            for e in f
                .value
                .get(kind)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if is_cli(e) {
                    cli += 1;
                } else if e.get("enabled").and_then(Value::as_bool) == Some(false) {
                    off += 1;
                } else {
                    ok += 1;
                }
            }
        }
    }
    (cli, off, ok)
}

/// The files `only` names (file names, with or without `.toml`), or all of them when it is
/// empty.
///
/// # Errors
///
/// A name that is no file here, or a file with a cli check: refused, nothing is chosen.
pub fn choose<'a>(files: &'a [File], only: &[String]) -> Result<Vec<&'a File>, String> {
    if only.is_empty() {
        return Ok(files.iter().collect());
    }
    let mut out = Vec::new();
    for name in only {
        let want = name.strip_suffix(".toml").unwrap_or(name);
        let f = files
            .iter()
            .find(|f| f.name().strip_suffix(".toml").unwrap_or(&f.name()) == want)
            .ok_or_else(|| format!("no monitor file named {name}"))?;
        if counts(std::slice::from_ref(f)).0 > 0 {
            return Err(format!(
                "{}: refused, it has cli checks; they run the project's own binary in a throwaway folder here and the agent has no cli surface, so they are never installed",
                f.name()
            ));
        }
        out.push(f);
    }
    Ok(out)
}

/// Runs `iohr agent checks lint --file <candidate>` with this very binary, as the host action
/// does, and says why it failed.
fn lint(candidate: &Path) -> Result<(), String> {
    // The program that holds `agent checks lint`: IOHR_MONITORS_LINT when set (a test or a
    // host with the agent somewhere else), else this binary, else the `iohr` on PATH (a build
    // without the agent in it).
    let mut tries: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("IOHR_MONITORS_LINT") {
        tries.push(PathBuf::from(p));
    } else {
        tries.extend(std::env::current_exe().ok());
        tries.push(PathBuf::from("iohr"));
    }
    let mut said = String::new();
    for exe in &tries {
        let out = match Command::new(exe)
            .args(lint_argv(candidate))
            .env("NO_COLOR", "1")
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                said = format!("cannot run {}: {}", exe.display(), e.kind());
                continue;
            }
        };
        if out.status.success() {
            return Ok(());
        }
        said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // This binary has no agent in it: try the next, not a failure of the file.
        if said.contains("not an iohr command") {
            continue;
        }
        break;
    }
    Err(format!(
        "iohr agent checks lint: failed, the checks file is unchanged\n{}",
        said.trim_end()
    )
    .trim_end()
    .to_owned())
}

/// Puts the monitors of `files` into `file` as the block `block`.
///
/// # Errors
///
/// A refusal or failure, in words; the checks file is unchanged then.
pub fn install(
    files: &[&File],
    block: &str,
    file: &Path,
    dry_run: bool,
) -> Result<Installed, String> {
    if !is_block_name(block) {
        return Err(format!(
            "{block}: a block name is 1 to 64 lower case letters, digits and -"
        ));
    }
    let owned: Vec<File> = files.iter().map(|f| (*f).clone()).collect();
    let (cli, off, entries) = counts(&owned);
    if entries == 0 {
        return Err(
            "nothing to install: every monitor here is a cli check or has enabled = false"
                .to_owned(),
        );
    }
    let source = export(&owned)?;
    let current = match std::fs::read_to_string(file) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("could not read the checks file: {}", e.kind())),
    };
    let next = put_block_noted(
        current.as_deref().unwrap_or(""),
        block,
        &format!("from project {block} {}", super::DIR),
        super::DIR,
        &source,
    )?;
    let total = declared(&next);
    if total > MAX_DECLARED_CHECKS {
        return Err(format!(
            "refused: the file would declare {total} checks, over the platform's {MAX_DECLARED_CHECKS}"
        ));
    }
    let done = |backup| Installed {
        file: file.to_path_buf(),
        block: block.to_owned(),
        declared: total,
        entries,
        cli,
        off,
        backup,
        dry_run,
    };
    // Built beside the file and linted there; a dry run lints a copy in the temp folder.
    let candidate = if dry_run {
        std::env::temp_dir().join(format!(".checks.toml.lint-{}", std::process::id()))
    } else {
        if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {}", dir.display(), e.kind()))?;
        }
        candidate_path(file)
    };
    std::fs::write(&candidate, &next)
        .map_err(|e| format!("could not write the checks file: {}", e.kind()))?;
    if let Err(why) = lint(&candidate) {
        let _ = std::fs::remove_file(&candidate);
        return Err(why);
    }
    if dry_run {
        let _ = std::fs::remove_file(&candidate);
        return Ok(done(None));
    }
    if current.is_none() {
        let moved = std::fs::rename(&candidate, file)
            .map_err(|e| format!("could not write the checks file: {}", e.kind()));
        let _ = std::fs::remove_file(&candidate);
        moved?;
        return Ok(done(None));
    }
    Ok(done(Some(swap_in(file, &candidate)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitors::parse;

    fn file(name: &str, text: &str) -> File {
        File {
            path: name.into(),
            value: parse(text).unwrap_or_else(|e| panic!("{name}: {e}")),
        }
    }

    const WEB: &str =
        "[[check]]\nname = \"web\"\ntarget = \"http://127.0.0.1:7171/h\"\nevery = \"60s\"\n";
    const CLI: &str =
        "[[check]]\nname = \"app\"\ntarget = { cli = \"app\" }\nargv = [\"--version\"]\n";

    #[test]
    fn the_block_is_named_by_the_project_folder() {
        assert_eq!(block_for(Path::new("/x/My Shop_2")), "my-shop-2");
        assert_eq!(block_for(Path::new("/")), "project");
        assert!(is_block_name("my-shop-2") && !is_block_name("My Shop"));
    }

    #[test]
    fn a_named_cli_file_is_refused_and_an_unknown_one_is_not_found() {
        let files = [file("web.toml", WEB), file("cli.toml", CLI)];
        let why = choose(&files, &["cli".into()]).unwrap_err();
        assert!(why.contains("never installed"), "{why}");
        assert!(
            choose(&files, &["nope".into()])
                .unwrap_err()
                .contains("no monitor file")
        );
        assert_eq!(choose(&files, &["web.toml".into()]).unwrap().len(), 1);
        // Left unnamed, every file is taken and the install leaves the cli checks out.
        assert_eq!(choose(&files, &[]).unwrap().len(), 2);
    }

    #[test]
    fn counts_split_cli_off_and_installable() {
        let off = format!(
            "{WEB}\n[[check]]\nname = \"off\"\ntarget = \"127.0.0.1:2\"\nevery = \"5m\"\nenabled = false\n"
        );
        assert_eq!(
            counts(&[file("a.toml", &off), file("cli.toml", CLI)]),
            (1, 1, 1)
        );
    }
}
