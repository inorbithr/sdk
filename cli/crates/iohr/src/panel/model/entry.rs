//! One line of text typed into the panel: the question to an ask, the message to a session.
//!
//! While it is open it takes every key (the panel already swallows them); `Enter` sends a
//! non-empty line, `Esc` drops it. What is typed is the person's own words and goes only to the
//! service or the inbox they chose; it is never logged.

use super::{Effect, Key, Model};
use crate::panel::cells::{ACC, KEY, Line, MUTE, Span, TXT, one_wide, sp};

/// The longest line typed, in characters.
const MAX: usize = 400;

/// Who the line is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum For {
    /// A question to this ask.
    Question(String),
    /// A message to this workload (a session).
    Message(String),
}

#[derive(Debug, Clone)]
pub(super) struct Entry {
    to: For,
    text: String,
}

impl Model {
    pub(super) fn begin_entry(&mut self, to: For) {
        self.note = None;
        self.entry = Some(Entry {
            to,
            text: String::new(),
        });
    }

    /// Whether a line is being typed.
    pub(super) fn typing(&self) -> bool {
        self.entry.is_some()
    }

    pub(super) fn entry_key(&mut self, key: Key) -> Effect {
        let Some(e) = &mut self.entry else {
            return Effect::None;
        };
        match key {
            Key::Esc => self.entry = None,
            Key::Backspace => {
                e.text.pop();
            }
            Key::Char(c) if !c.is_control() && e.text.chars().count() < MAX => e.text.push(c),
            Key::Enter if !e.text.trim().is_empty() => {
                let text = e.text.trim().to_owned();
                let to = e.to.clone();
                self.entry = None;
                return match to {
                    For::Question(id) => Effect::Question { id, text },
                    For::Message(id) => Effect::Message { id, text },
                };
            }
            _ => {}
        }
        Effect::None
    }

    /// The typing line, its tail kept in view in `room` columns.
    pub(super) fn entry_line(&self, room: usize) -> Option<Line> {
        let e = self.entry.as_ref()?;
        let label = match e.to {
            For::Question(_) => "ask",
            For::Message(_) => "say",
        };
        let keep = room.saturating_sub(label.len() + 4).max(8);
        let text = one_wide(&e.text);
        let n = text.chars().count();
        let shown: String = text.chars().skip(n.saturating_sub(keep)).collect();
        Some(Line {
            spans: vec![
                sp(label, MUTE),
                sp(" › ", ACC),
                sp(&shown, TXT),
                sp("▏", ACC),
            ],
            selected: false,
        })
    }

    pub(super) fn entry_footer(&self) -> Option<Vec<Span>> {
        self.entry.as_ref().map(|_| {
            vec![
                sp("⏎", KEY),
                sp(" send   ", MUTE),
                sp("esc", KEY),
                sp(" cancel", MUTE),
            ]
        })
    }
}
