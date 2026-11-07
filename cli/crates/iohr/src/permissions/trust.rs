//! Folder trust for `?` (PRD 0197): the home folder is too broad to read without asking, so
//! at a terminal Io asks once, on the same Io card as a command's ask, the way Claude Code
//! asks whether to trust a folder. A yes for always is kept in `trusted.toml` beside the
//! user's permission rules; a yes for this time keeps nothing. No terminal, no key or any failure is a no.

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

/// The folders the person said yes for always, as kept (canonical when they were written).
/// A folder that is gone or a file that is broken gives none; callers check each again.
#[must_use]
pub(crate) fn folders(user_dir: &Path) -> Vec<PathBuf> {
    load(user_dir)
        .folders
        .into_iter()
        .map(PathBuf::from)
        .collect()
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

/// The answer a key gives, or none for a key that means nothing. The keys of the card and of
/// a command's ask: `y` once, `a` always, `n` (and Enter, Esc, `q`, Ctrl-C) no.
#[must_use]
pub(crate) fn answer_of(key: &console::Key) -> Option<Answer> {
    use console::Key;
    match key {
        Key::Char('y' | 'Y') => Some(Answer::Once),
        Key::Char('a' | 'A') => Some(Answer::Always),
        Key::Char('n' | 'N' | 'q' | 'Q' | '\u{3}') | Key::Enter | Key::Escape | Key::CtrlC => {
            Some(Answer::No)
        }
        _ => None,
    }
}

/// What Io asks to do, as the card says it.
const WANTS: &str = "read files here to answer (secrets stay blocked)";

/// `folder` with no control character in it, for the terminal.
fn shown(folder: &Path) -> String {
    folder
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// How the card names the folder it asks about: "your home folder", else the folder's own
/// name (the card's other line holds the full path).
#[must_use]
pub(crate) fn place_of(folder: &Path, home: Option<&Path>) -> String {
    if crate::headless::is_home(folder, home) {
        return "your home folder".to_owned();
    }
    folder
        .file_name()
        .map_or_else(|| shown(folder), |n| shown(Path::new(n)))
}

/// Asks at the terminal whether Io may read files in `folder`, on the same Io card as a
/// command's ask (one prompt, one design); a terminal too narrow or plain for the card gets
/// one line with the same keys. Only call it at a terminal.
pub(crate) fn ask(folder: &Path) -> Answer {
    #[cfg(unix)]
    {
        use super::ask_card;
        use super::broker::{self, Choice, Wanted};
        let look = crate::card::Look::detect(crate::ui::Ui::detect(false).fancy());
        let cols = console::Term::stderr()
            .size_checked()
            .map_or(0, |(_, c)| usize::from(c));
        if ask_card::drawn_at(cols, look) {
            let card = ask_card::Ask {
                wanted: Wanted {
                    verb: WANTS.to_owned(),
                    thing: Some(shown(folder)),
                    more: 0,
                },
                folder: place_of(
                    folder,
                    std::env::var_os("HOME").map(PathBuf::from).as_deref(),
                ),
                can_always: true,
            };
            return match broker::draw_and_read_rows(&card, &ask_card::trust_choices(), cols, look) {
                Choice::Once => Answer::Once,
                Choice::Always => Answer::Always,
                Choice::No => Answer::No,
            };
        }
    }
    let term = console::Term::stderr();
    let line = format!(
        "Io (AI) wants to {WANTS} in {} \u{b7} y once \u{b7} a always trust this folder \u{b7} n no",
        shown(folder)
    );
    if term.write_line(&line).is_err() {
        return Answer::No;
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
    fn the_kept_folders_are_listed_in_the_order_they_were_kept() {
        let user = tempfile::tempdir().unwrap();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_eq!(folders(user.path()).len(), 0);
        remember(user.path(), a.path()).unwrap();
        remember(user.path(), b.path()).unwrap();
        remember(user.path(), a.path()).unwrap();
        assert_eq!(
            folders(user.path()),
            [
                a.path().canonicalize().unwrap(),
                b.path().canonicalize().unwrap()
            ]
        );
    }

    #[test]
    fn the_card_names_the_folder_it_asks_about() {
        let home = tempfile::tempdir().unwrap();
        let sub = home.path().join("Downloads");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(place_of(home.path(), Some(home.path())), "your home folder");
        assert_eq!(place_of(&sub, Some(home.path())), "Downloads");
    }

    #[test]
    fn a_broken_file_trusts_nothing() {
        let user = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join(FILE), "folders = 3").unwrap();
        assert!(!is_trusted(user.path(), home.path()));
    }

    #[test]
    fn keys_are_the_command_asks_keys() {
        use console::Key;
        assert_eq!(answer_of(&Key::Char('y')), Some(Answer::Once));
        assert_eq!(answer_of(&Key::Char('a')), Some(Answer::Always));
        assert_eq!(answer_of(&Key::Char('n')), Some(Answer::No));
        // A stray Enter never trusts a folder.
        assert_eq!(answer_of(&Key::Enter), Some(Answer::No));
        assert_eq!(answer_of(&Key::CtrlC), Some(Answer::No));
        assert_eq!(answer_of(&Key::Char('x')), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_trust_ask_is_the_io_card_with_its_own_rows() {
        use crate::permissions::ask_card;
        use crate::permissions::broker::Wanted;
        let card = ask_card::Ask {
            wanted: Wanted {
                verb: WANTS.to_owned(),
                thing: Some("/Users/ana".to_owned()),
                more: 0,
            },
            folder: "your home folder".to_owned(),
            can_always: true,
        };
        let look = crate::card::Look {
            fancy: true,
            truecolor: false,
        };
        let rows = ask_card::trust_choices();
        let l: Vec<String> = ask_card::lines_with(&card, &rows, 2, 90, look)
            .iter()
            .map(|x| console::strip_ansi_codes(x).into_owned())
            .collect();
        let all = l.join("\n");
        assert!(l[0].contains("AI request"), "{all}");
        assert!(all.contains("Io (AI) wants to read files here"), "{all}");
        assert!(all.contains("/Users/ana"), "{all}");
        assert!(all.contains("Always trust this folder"), "{all}");
        assert!(!all.contains("Always in this project"), "{all}");
        assert_eq!(
            ask_card::choice_for(&rows[1].value),
            crate::permissions::broker::Choice::Always
        );
    }
}
