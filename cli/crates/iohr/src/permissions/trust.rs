//! Folder trust for `?` (PRD 0197): the home folder is too broad to read without asking, so
//! at a terminal Io asks once, the way Claude Code asks whether to trust a folder. A yes for
//! always is kept in `trusted.toml` beside the user's permission rules; a yes for this time
//! keeps nothing. No terminal, no key or any failure is a no.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file, in the user's iohr folder.
pub(crate) const FILE: &str = "trusted.toml";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Trusted {
    #[serde(default)]
    folders: Vec<String>,
}

fn load(user_dir: &Path) -> Trusted {
    std::fs::read_to_string(user_dir.join(FILE))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn canonical(folder: &Path) -> Option<String> {
    folder.canonicalize().ok().map(|p| p.display().to_string())
}

/// Whether the person said yes for always to reading `folder`.
#[must_use]
pub(crate) fn is_trusted(user_dir: &Path, folder: &Path) -> bool {
    canonical(folder).is_some_and(|f| load(user_dir).folders.contains(&f))
}

/// Keeps `folder` as trusted. Written whole, so a broken file is replaced by a good one.
pub(crate) fn remember(user_dir: &Path, folder: &Path) -> Result<PathBuf, String> {
    let f = canonical(folder).ok_or("the folder cannot be found")?;
    let mut t = load(user_dir);
    if !t.folders.contains(&f) {
        t.folders.push(f);
    }
    std::fs::create_dir_all(user_dir)
        .map_err(|e| format!("cannot create {}: {}", user_dir.display(), e.kind()))?;
    let path = user_dir.join(FILE);
    let body = toml::to_string(&t).map_err(|e| e.to_string())?;
    std::fs::write(&path, body)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e.kind()))?;
    Ok(path)
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Always,
    Once,
    No,
}

/// The answer a key gives, or none for a key that means nothing.
#[must_use]
pub(crate) fn answer_of(key: &console::Key) -> Option<Answer> {
    match key {
        console::Key::Char('y' | 'a') | console::Key::Enter => Some(Answer::Always),
        console::Key::Char('o' | '1') => Some(Answer::Once),
        console::Key::Char('n') | console::Key::Escape | console::Key::CtrlC => Some(Answer::No),
        _ => None,
    }
}

/// Asks at the terminal whether Io may read files in `folder`. Only call it at a terminal.
pub(crate) fn ask(folder: &Path) -> Answer {
    let term = console::Term::stderr();
    let shown: String = folder
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let lines = [
        format!("Io wants to read files in {shown} to answer (secrets stay blocked)."),
        "Trust this folder?  [y] yes, remember  ·  [o] only this time  ·  [n] no".to_owned(),
    ];
    for l in &lines {
        if term.write_line(l).is_err() {
            return Answer::No;
        }
    }
    loop {
        match term.read_key_raw() {
            Ok(k) => {
                if let Some(a) = answer_of(&k) {
                    return a;
                }
            }
            Err(_) => return Answer::No,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_folders_are_trusted_and_others_are_not() {
        let user = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert!(!is_trusted(user.path(), home.path()));
        remember(user.path(), home.path()).unwrap();
        remember(user.path(), home.path()).unwrap();
        assert!(is_trusted(user.path(), home.path()));
        assert!(!is_trusted(user.path(), other.path()));
        assert_eq!(load(user.path()).folders.len(), 1);
    }

    #[test]
    fn a_broken_file_trusts_nothing() {
        let user = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join(FILE), "folders = 3").unwrap();
        assert!(!is_trusted(user.path(), home.path()));
    }

    #[test]
    fn keys_mean_always_once_or_no() {
        use console::Key;
        assert_eq!(answer_of(&Key::Char('y')), Some(Answer::Always));
        assert_eq!(answer_of(&Key::Char('o')), Some(Answer::Once));
        assert_eq!(answer_of(&Key::Char('n')), Some(Answer::No));
        assert_eq!(answer_of(&Key::CtrlC), Some(Answer::No));
        assert_eq!(answer_of(&Key::Char('x')), None);
    }
}
