//! `iohr atlas scrum --watch` (RFC 0003, "Watching the board"): redraw a view whenever
//! board.json changes, and in a terminal show what moved.
//!
//! - **Changes.** Each change to a card (moved, ticked, noted, added) is listed with the time it
//!   was seen, newest first ([`super::diff`]).
//! - **Hot cards.** A card that just changed carries a `▸` for two minutes.
//! - **Files.** The files git sees changed in the last 15 minutes, newest first, with the card
//!   whose docs they are. This is how an agent's work shows even before it touches the board.
//! - **Fit.** The board is fitted to the terminal's height by folding done, dropped, backlog and
//!   ready, in that order, keeping their hot cards. The screen is redrawn in place, without
//!   clearing, so it does not flicker.
//!
//! A board that fails to load (say, half written) keeps the last good frame and shows the error.
//! Through a pipe each change prints the view again, one compact JSON line per change with
//! `--json`. Polls the file's size and time twice a second, so it needs no file-events library.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::board::{self, Board, Loaded};
use super::view::Look;
use super::{Error, diff, today};

/// How a view prints: text for people, JSON pretty for one answer, JSON on one line for `--watch`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    Text,
    Pretty,
    Line,
}

/// The view `--watch` redraws: what it draws for a board, and which kind it is (the screen
/// marks changed cards on a board and a list, and folds a board to fit).
pub(crate) struct View<'a> {
    pub(crate) kind: Kind,
    pub(crate) draw: &'a dyn Fn(&Board, &str, Form, &Look) -> Result<String, Error>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Board,
    /// The plan has no card lines to mark.
    Plan,
    Rows,
}

/// How long a changed card keeps its marker.
const HOT: Duration = Duration::from_secs(120);
/// How many changes the feed keeps.
const FEED: usize = 6;
/// How many changed files the strip shows.
const FILES: usize = 5;
/// How old a changed file may be and still show.
const RECENT: Duration = Duration::from_secs(15 * 60);
/// How often git is asked which files changed.
const FILES_EVERY: Duration = Duration::from_secs(2);
/// The lists folded first when the board is taller than the terminal.
const FOLD: [&str; 4] = ["done", "dropped", "backlog", "ready"];

struct Event {
    at: String,
    card: String,
    what: String,
}

#[derive(Debug, Clone, PartialEq)]
struct File {
    at: String,
    path: String,
    card: Option<String>,
}

pub(crate) fn watch(path: &Path, view: &View, json: bool) -> Result<(), Error> {
    use std::io::{IsTerminal, Write};
    let tty = std::io::stdout().is_terminal() && !json;
    let form = if json { Form::Line } else { Form::Text };
    let stamp = || std::fs::metadata(path).ok().map(|m| (m.modified().ok(), m.len()));
    let root = if tty { git_root(path) } else { None };
    let (mut seen, mut size, mut day) = (None, (0, 0), String::new());
    let mut printed = None::<String>;
    let mut board: Option<Loaded> = None;
    let mut error: Option<String> = None;
    let mut feed: VecDeque<Event> = VecDeque::new();
    let mut hot: HashMap<String, Instant> = HashMap::new();
    let (mut files, mut asked) = (Vec::<File>::new(), None::<Instant>);
    let mut stdout = std::io::stdout();
    loop {
        let (now, today, look) = (stamp(), today(), if tty { Look::stdout(false) } else { Look::plain() });
        let rows = if tty { term_rows() } else { 0 };
        let mut redraw = printed.is_none() || (look.width, rows) != size || today != day;
        if now != seen {
            seen = now;
            redraw = true;
            match board::load(path) {
                Ok(l) => {
                    if let Some(old) = &board {
                        let at = clock();
                        for c in diff::changes(&old.board, &l.board) {
                            hot.insert(c.card.clone(), Instant::now());
                            feed.push_front(Event { at: at.clone(), card: c.card, what: c.what });
                        }
                        feed.truncate(FEED);
                    }
                    board = Some(l);
                    error = None;
                }
                Err(e) => error = Some(e.to_string()),
            }
        }
        let before = hot.len();
        hot.retain(|_, t| t.elapsed() < HOT);
        redraw |= hot.len() != before;
        if let (Some(root), Some(b), true) = (&root, &board, asked.is_none_or(|t| t.elapsed() >= FILES_EVERY)) {
            asked = Some(Instant::now());
            let got = changed_files(root, path, b);
            redraw |= got != files;
            files = got;
        }
        if redraw {
            (size, day) = ((look.width, rows), today.clone());
            // In a terminal a load error keeps the last good board on screen and goes in the footer.
            let frame = match &board {
                Some(b) if tty || error.is_none() => (view.draw)(&b.board, &today, form, &look),
                _ => Err(Error::Io(error.clone().unwrap_or_else(|| format!("cannot read {}", path.display())))),
            };
            let text = match frame {
                Ok(f) if tty => screen(Screen { view: &f, kind: view.kind, feed: &feed, hot: &hot, files: &files, error: error.as_deref(), path, rows, look: &look }),
                Ok(f) => f,
                Err(e) if tty => screen(Screen { view: "", kind: Kind::Rows, feed: &feed, hot: &hot, files: &files, error: Some(&e.to_string()), path, rows, look: &look }),
                Err(e) => {
                    eprintln!("{e}");
                    printed.clone().unwrap_or_default()
                }
            };
            // A terminal is redrawn on every change; a pipe gets the view again only when it differs.
            if tty || printed.as_ref() != Some(&text) {
                let gap = if !tty && !json && printed.is_some() { "\n" } else { "" };
                let _ = stdout.write_all(format!("{gap}{text}").as_bytes()).and_then(|()| stdout.flush());
                printed = Some(text);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The terminal's rows, to fit the board to the screen.
fn term_rows() -> usize {
    usize::from(console::Term::stdout().size().0)
}

fn clock() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

struct Screen<'a> {
    view: &'a str,
    kind: Kind,
    feed: &'a VecDeque<Event>,
    hot: &'a HashMap<String, Instant>,
    files: &'a [File],
    error: Option<&'a str>,
    path: &'a Path,
    rows: usize,
    look: &'a Look,
}

/// One terminal frame: the view (marked and fitted), the changes, the files, a footer. Drawn
/// from the top left, each line cleared to its end and the rest of the screen cleared after, so
/// nothing flickers.
fn screen(s: Screen) -> String {
    let look = s.look;
    let cut = |t: &str| t.chars().take(look.width.saturating_sub(1)).collect::<String>();
    let mut tail = Vec::new();
    if !s.feed.is_empty() {
        tail.push(String::new());
        tail.push(look.dim("changes"));
        for e in s.feed {
            tail.push(format!("  {}  {}  {}", look.dim(&e.at), look.hot(&e.card), cut(&e.what)));
        }
    }
    if !s.files.is_empty() {
        tail.push(String::new());
        tail.push(look.dim("files changed"));
        for f in s.files {
            let card = f.card.as_deref().map(|c| format!("  {}", look.hot(c))).unwrap_or_default();
            tail.push(format!("  {}  {}{card}", look.dim(&f.at), cut(&f.path)));
        }
    }
    let foot = match s.error {
        Some(e) => look.dim(&format!("{e}; showing the last good board · {}", clock())),
        None => look.dim(&format!("watching {} · drawn {} · Ctrl-C stops", s.path.display(), clock())),
    };
    tail.push(String::new());
    tail.push(foot);
    let hot: Vec<&str> = s.hot.keys().map(String::as_str).collect();
    let board = s.kind == Kind::Board;
    let marked = mark(s.view, &hot, board, look, s.kind != Kind::Plan);
    let room = s.rows.saturating_sub(tail.len()).max(4);
    let body = if board { fit_board(&marked, &hot, room, look) } else { fit(&marked, room, look) };
    let mut out = String::from("\x1b[H");
    for l in body.lines().map(str::to_owned).chain(tail) {
        out.push_str(&l);
        out.push_str("\x1b[K\n");
    }
    out.truncate(out.len() - 1);
    out.push_str("\x1b[J");
    out
}

/// Text without ANSI escapes, to read what a painted line says.
fn plain(s: &str) -> String {
    console::strip_ansi_codes(s).into_owned()
}

/// Whether a line of a view shows a changed card: its first word on a board line, one of the
/// first three on a list row (list, section, id), after any `▸` already put there.
fn is_hot(line: &str, hot: &[&str], board: bool) -> bool {
    let p = plain(line);
    let words = p.trim_start().trim_start_matches('▸').split_whitespace().take(if board { 1 } else { 3 });
    words.into_iter().any(|w| hot.contains(&w))
}

/// Puts `▸ ` before each changed card. Board lines start with two spaces, which the marker
/// takes; list rows get a two-character margin. The plan view has no card lines (`cards` false).
fn mark(text: &str, hot: &[&str], board: bool, look: &Look, cards: bool) -> String {
    let mut out = String::new();
    for l in text.lines() {
        let on = cards && is_hot(l, hot, board);
        let line = match (board, on) {
            (true, true) if l.starts_with("  ") => format!("{}{}", look.hot("▸ "), &l[2..]),
            (true, _) => l.to_owned(),
            (false, true) => format!("{}{l}", look.hot("▸ ")),
            (false, false) if cards => format!("  {l}"),
            (false, false) => l.to_owned(),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Cuts a view to `room` lines, saying how many were left out.
fn fit(text: &str, room: usize, look: &Look) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= room {
        return text.to_owned();
    }
    let keep = room.saturating_sub(1);
    let mut out: String = lines[..keep].iter().map(|l| format!("{l}\n")).collect();
    out.push_str(&look.dim(&format!("  … {} more lines", lines.len() - keep)));
    out.push('\n');
    out
}

/// Fits the board to `room` lines by folding lists in [`FOLD`] order. A folded list keeps its
/// heading, its changed cards and a line saying how many cards it hides.
fn fit_board(text: &str, hot: &[&str], room: usize, look: &Look) -> String {
    let mut blocks: Vec<Vec<String>> = text.split("\n\n").map(|b| b.lines().map(str::to_owned).collect()).collect();
    let height = |bs: &[Vec<String>]| bs.iter().map(Vec::len).sum::<usize>() + bs.len().saturating_sub(1);
    for list in FOLD {
        if height(&blocks) <= room {
            break;
        }
        let Some(b) = blocks.iter_mut().skip(1).find(|b| list_of(b) == list) else { continue };
        let head = b.iter().take_while(|l| !l.starts_with("  ")).count();
        let (keep, hide): (Vec<String>, Vec<String>) = b[head..].iter().cloned().partition(|l| is_hot(l, hot, true));
        let mut folded = b[..head].to_vec();
        folded.extend(keep);
        if !hide.is_empty() {
            folded.push(look.dim(&format!("  … {} more", hide.len())));
        }
        *b = folded;
    }
    let joined = blocks.iter().map(|b| b.join("\n")).collect::<Vec<_>>().join("\n\n") + "\n";
    fit(&joined, room, look)
}

/// The list a board block is about, from its heading ("● ready  3 cards" or "ready (3 cards, …)").
fn list_of(block: &[String]) -> String {
    let head = block.first().map(|l| plain(l)).unwrap_or_default();
    head.trim_start_matches('●').split_whitespace().next().unwrap_or_default().to_owned()
}

/// The repository holding the board, when git knows one.
fn git_root(board: &Path) -> Option<PathBuf> {
    let dir = board.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(["rev-parse", "--show-toplevel"]).output().ok()?;
    let s = String::from_utf8(o.stdout).ok()?;
    (o.status.success() && !s.trim().is_empty()).then(|| PathBuf::from(s.trim()))
}

/// Paths from `git status --porcelain=v1 -z`: "XY path\0", a rename or copy followed by the old
/// path as one more field.
fn porcelain(raw: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut fields = raw.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(f) = fields.next() {
        if f.len() < 4 {
            continue;
        }
        out.push(String::from_utf8_lossy(&f[3..]).into_owned());
        if f[..2].iter().any(|c| matches!(c, b'R' | b'C')) {
            fields.next();
        }
    }
    out
}

/// The files git sees changed (or new) under `root` in the last [`RECENT`], newest first, each
/// with the card whose docs it is. board.json itself is left out: the view already shows it.
fn changed_files(root: &Path, board_path: &Path, b: &Loaded) -> Vec<File> {
    let Ok(o) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
    else {
        return Vec::new();
    };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    let board = canon(board_path);
    let atlas = b.atlas.as_deref().map(canon);
    let docs: HashMap<PathBuf, &str> = atlas
        .iter()
        .flat_map(|a| b.board.cards.iter().flat_map(move |c| c.docs.all().map(move |p| (a.join(p), c.id.as_str()))))
        .collect();
    recent(root, porcelain(&o.stdout), &board, &docs, SystemTime::now())
}

fn recent(root: &Path, paths: Vec<String>, skip: &Path, docs: &HashMap<PathBuf, &str>, now: SystemTime) -> Vec<File> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let mut got: Vec<(SystemTime, File)> = paths
        .into_iter()
        .filter_map(|p| {
            let full = root.join(&p);
            let m = std::fs::metadata(&full).ok().filter(std::fs::Metadata::is_file)?;
            let at = m.modified().ok()?;
            let fresh = now.duration_since(at).unwrap_or_default() <= RECENT;
            (fresh && full != skip).then(|| {
                let card = docs.get(&full).map(|c| (*c).to_owned());
                let shown = chrono::DateTime::<chrono::Local>::from(at).format("%H:%M:%S").to_string();
                (at, File { at: shown, path: p, card })
            })
        })
        .collect();
    got.sort_by_key(|f| std::cmp::Reverse(f.0));
    got.into_iter().take(FILES).map(|(_, f)| f).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn color() -> Look {
        Look::new(crate::ui::Paint::from_parts(true, false, None, Some("xterm-256color")), 100)
    }

    #[test]
    fn porcelain_reads_changed_new_and_renamed_paths() {
        let raw = b" M crates/iohr/src/scrum.rs\0?? atlas/rfc/new.md\0R  new.rs\0old.rs\0 D gone.rs\0";
        assert_eq!(porcelain(raw), ["crates/iohr/src/scrum.rs", "atlas/rfc/new.md", "new.rs", "gone.rs"]);
    }

    #[test]
    fn a_changed_card_is_marked_and_kept_when_its_list_folds() {
        let board = "Board (updated 2026-11-14)\n\nready (2 cards, 3 h)\n  aa S 1 h\n  bb S 2 h\n\ndoing (1 card, 1 h)\n  cc M 1 h\n\ndone (3 cards, 3 h)\n  dd S 1 h\n  ee S 1 h\n  ff S 1 h\n";
        let marked = mark(board, &["ee"], true, &Look::PLAIN, true);
        assert!(marked.contains("▸ ee S 1 h"), "{marked}");
        assert!(marked.contains("\n  dd S 1 h"), "others keep their indent: {marked}");

        // 14 lines; room for 12 folds done (keeping ee) and leaves ready alone.
        let fitted = fit_board(&marked, &["ee"], 12, &Look::PLAIN);
        assert!(fitted.contains("done (3 cards, 3 h)\n▸ ee S 1 h\n  … 2 more"), "{fitted}");
        assert!(fitted.contains("  aa S 1 h\n  bb S 2 h"), "ready is not folded while it fits: {fitted}");
        assert!(fitted.lines().count() <= 12, "{fitted}");
    }

    #[test]
    fn painted_lines_are_read_through_their_colours() {
        let line = format!("  {} {}", color().hot("aa"), color().dim("S"));
        assert!(is_hot(&line, &["aa"], true));
        assert_eq!(list_of(&[format!("{} {}  3 cards", color().dim("●"), color().hot("review"))]), "review");
        let row = mark("doing   core  aa S 1 h\nready   core  bb S\n", &["aa"], false, &color(), true);
        assert!(plain(&row).starts_with("▸ doing"), "{row}");
        assert!(plain(&row).contains("\n  ready"), "{row}");
    }

    #[test]
    fn recent_files_are_newest_first_with_their_card_and_without_the_board() {
        let tmp = tempfile::Builder::new().prefix("iohr-watch-files-").tempdir().expect("dir");
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join("atlas/rfc")).expect("dir");
        let root = std::fs::canonicalize(&dir).expect("canon");
        let old = SystemTime::now() - Duration::from_secs(3600);
        for (p, age) in [("a.rs", 30), ("atlas/rfc/0003.md", 10), ("board.json", 5), ("stale.rs", 0)] {
            let f = std::fs::File::create(root.join(p)).expect("file");
            let t = if p == "stale.rs" { old } else { SystemTime::now() - Duration::from_secs(age) };
            f.set_modified(t).expect("mtime");
        }
        let docs = HashMap::from([(root.join("atlas/rfc/0003.md"), "scrum")]);
        let paths = ["a.rs", "atlas/rfc/0003.md", "board.json", "stale.rs", "deleted.rs"].map(String::from).to_vec();
        let got = recent(&root, paths, &root.join("board.json"), &docs, SystemTime::now());
        let names: Vec<(&str, Option<&str>)> = got.iter().map(|f| (f.path.as_str(), f.card.as_deref())).collect();
        assert_eq!(names, [("atlas/rfc/0003.md", Some("scrum")), ("a.rs", None)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
