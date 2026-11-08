//! A read-only page inside the panel: one ask's details, a session's transcript. The lines
//! arrive after the view opens (`Loading`), are wrapped to the box once, and scroll with the
//! arrows; a transcript opens at its end.

use super::{Key, Model, Parts, Source, View, back_footer};
use crate::panel::cells::{AMBER, FAINT, KEY, Line, MUTE, TXT, bold, one_wide, sp};

/// Rows shown at once.
const VISIBLE: usize = 9;
/// The width of a wrapped line: the reader's box (72) less its frame and indent.
pub(super) const ROOM: usize = 64;

#[derive(Debug, Clone)]
pub(super) struct Reader {
    title: String,
    back: View,
    lines: Source<Vec<String>>,
    scroll: usize,
    tail: bool,
}

/// `text` as rows of at most `room` characters, broken at spaces where it can.
pub(super) fn wrap(text: &str, room: usize) -> Vec<String> {
    let text = one_wide(text);
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let mut word = word.to_owned();
        while word.chars().count() > room {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            rows.push(word.chars().take(room).collect());
            word = word.chars().skip(room).collect();
        }
        let need = row.chars().count() + usize::from(!row.is_empty()) + word.chars().count();
        if need > room && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        if !row.is_empty() {
            row.push(' ');
        }
        row.push_str(&word);
    }
    rows.push(row);
    rows
}

impl Model {
    /// Opens the reader on `title`, its lines to come; `Esc` goes back to `back`.
    pub(super) fn open_reader(&mut self, title: &str, back: View, tail: bool) {
        self.reader = Some(Reader {
            title: super::work::cut(title, 30),
            back,
            lines: Source::Loading,
            scroll: 0,
            tail,
        });
        self.view = View::Reader;
        self.note = None;
    }

    /// The lines came, or could not.
    pub(in crate::panel) fn reader_done(&mut self, lines: Result<Vec<String>, String>) {
        let Some(r) = &mut self.reader else { return };
        r.lines = match lines {
            Ok(l) => {
                let wrapped: Vec<String> = l.iter().flat_map(|x| wrap(x, ROOM)).collect();
                r.scroll = if r.tail {
                    wrapped.len().saturating_sub(VISIBLE)
                } else {
                    0
                };
                Source::Ready(wrapped)
            }
            Err(why) => Source::Unavailable(why),
        };
    }

    pub(super) fn reader_key(&mut self, key: Key) {
        let Some(r) = &mut self.reader else {
            self.view = View::Menu;
            return;
        };
        let max = match &r.lines {
            Source::Ready(l) => l.len().saturating_sub(VISIBLE),
            _ => 0,
        };
        match key {
            Key::Esc | Key::Left => {
                self.view = r.back;
                self.reader = None;
            }
            Key::Up | Key::Char('k') => r.scroll = r.scroll.saturating_sub(1),
            Key::Down | Key::Char('j') => r.scroll = (r.scroll + 1).min(max),
            _ => {}
        }
    }

    pub(super) fn reader(&self) -> Parts {
        let Some(r) = &self.reader else {
            return (vec![], vec![], vec![], Some(back_footer()));
        };
        let plain = |t: &str, fg| Line {
            spans: vec![sp(t, fg)],
            selected: false,
        };
        let mut body = Vec::new();
        let mut right = Vec::new();
        match &r.lines {
            Source::Loading => body.push(plain("Reading …", MUTE)),
            Source::Unavailable(why) => {
                body.push(plain("Cannot show it.", AMBER));
                body.push(plain(&super::work::cut(why, ROOM), MUTE));
            }
            Source::Ready(l) => {
                body.extend(l.iter().skip(r.scroll).take(VISIBLE).map(|x| plain(x, TXT)));
                if l.len() > VISIBLE {
                    right = vec![sp(
                        &format!("{}/{}", (r.scroll + VISIBLE).min(l.len()), l.len()),
                        FAINT,
                    )];
                }
            }
        }
        let footer = vec![
            sp("↑↓", KEY),
            sp(" scroll   ", MUTE),
            sp("esc", KEY),
            sp(" back", MUTE),
        ];
        (
            vec![sp(" › ", FAINT), bold(&r.title, TXT)],
            right,
            body,
            Some(footer),
        )
    }
}
