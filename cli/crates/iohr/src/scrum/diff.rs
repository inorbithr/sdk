//! What changed between two versions of a board, card by card, in words a person reads at a
//! glance. `iohr atlas scrum --watch` lists these under the view (RFC 0003, "Watching the board").

use super::board::{Board, Card};
use serde::Serialize;

/// One change to one card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Change {
    pub(crate) card: String,
    pub(crate) what: String,
}

/// Every change from `old` to `new`, in `new`'s card order, removed cards last.
pub(crate) fn changes(old: &Board, new: &Board) -> Vec<Change> {
    let mut out = Vec::new();
    let mut push = |card: &str, what: String| {
        out.push(Change {
            card: card.to_owned(),
            what,
        });
    };
    for c in &new.cards {
        match old.cards.iter().find(|o| o.id == c.id) {
            None => push(&c.id, format!("added to {}: {}", c.list, c.title)),
            Some(o) => card_changes(o, c).into_iter().for_each(|w| push(&c.id, w)),
        }
    }
    for o in old
        .cards
        .iter()
        .filter(|o| new.cards.iter().all(|c| c.id != o.id))
    {
        push(&o.id, "removed from the board".into());
    }
    out
}

#[allow(
    clippy::many_single_char_names,
    reason = "short names for a ported, local diff"
)]
fn card_changes(o: &Card, c: &Card) -> Vec<String> {
    let mut w = Vec::new();
    if o.list != c.list {
        w.push(format!("{} → {}", o.list, c.list));
    }
    let n = c.checklist.len();
    for (i, x) in c.checklist.iter().enumerate() {
        match o.checklist.get(i) {
            Some(y) if y.done != x.done => {
                let done = c.checklist.iter().filter(|x| x.done).count();
                let verb = if x.done { "ticked" } else { "unticked" };
                w.push(format!("{verb} {}: {} ({done}/{n} done)", i + 1, x.text));
            }
            None => w.push(format!("checklist item {} added: {}", i + 1, x.text)),
            _ => {}
        }
    }
    if c.notes != o.notes {
        let line = c
            .notes
            .as_deref()
            .and_then(|n| n.lines().last())
            .unwrap_or_default();
        if !line.is_empty() {
            w.push(format!("note: {line}"));
        }
    }
    if o.blocked_by != c.blocked_by {
        w.push(if c.blocked_by.is_empty() {
            "unblocked".into()
        } else {
            format!("blocked by {}", c.blocked_by.join(", "))
        });
    }
    if let Some(pr) = c.pr.as_ref().filter(|_| o.pr != c.pr) {
        w.push(format!("PR {pr}"));
    }
    if o.title != c.title {
        w.push(format!("renamed: {}", c.title));
    }
    for (name, a, b) in [
        ("start", &o.start, &c.start),
        ("due", &o.due, &c.due),
        ("size", &o.size, &c.size),
    ] {
        if a != b {
            w.push(format!("{name} {a} → {b}"));
        }
    }
    if o.hours != c.hours {
        w.push(format!("hours {} → {}", o.hours, c.hours));
    }
    if w.is_empty() && serde_json::to_value(o).ok() != serde_json::to_value(c).ok() {
        w.push("edited".into());
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrum::board::tests::{board, card};
    use crate::scrum::change;

    const T: &str = "2026-11-14";

    #[test]
    fn a_move_a_tick_a_note_and_an_add_are_named() {
        let old = board(vec![
            card("aa", "ready", &[]),
            card("bb", "ready", &[]),
            card("gone", "backlog", &[]),
        ]);
        let mut new = old.clone();
        change::start(&mut new, "aa", None, T).expect("start");
        change::tick(&mut new, "aa", 2, false).expect("tick");
        change::note(&mut new, "bb", "agent is on it", T).expect("note");
        new.cards.retain(|c| c.id != "gone");
        new.cards.push(card("cc", "backlog", &[]));

        let got: Vec<String> = changes(&old, &new)
            .iter()
            .map(|c| format!("{}: {}", c.card, c.what))
            .collect();
        let note = new.cards[1]
            .notes
            .as_deref()
            .and_then(|n| n.lines().last())
            .expect("note line")
            .to_owned();
        assert_eq!(
            got,
            [
                "aa: ready → doing".to_owned(),
                "aa: ticked 2: two (1/2 done)".into(),
                format!("bb: note: {note}"),
                "cc: added to backlog: Card cc".into(),
                "gone: removed from the board".into(),
            ]
        );
        assert!(note.contains("agent is on it"), "{note}");
        assert!(
            changes(&new, &new).is_empty(),
            "nothing changed, nothing listed"
        );
    }
}
