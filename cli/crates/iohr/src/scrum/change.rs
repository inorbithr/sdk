//! Changes to a board in memory. Each returns a usage error or a refusal; the caller saves the
//! board with [`super::board::save`], which checks the whole board before it writes.

use super::board::{Board, Card, Check, Docs};
use super::{Error, LISTS};
use serde_json::{Map, Number, Value};

/// Appends one dated line to a card's notes.
fn note_line(c: &mut Card, line: String) {
    match &mut c.notes {
        Some(n) if !n.is_empty() => {
            n.push('\n');
            n.push_str(&line);
        }
        _ => c.notes = Some(line),
    }
}

/// Hours as a JSON number: `6` stays an integer, `1.5` a float.
pub(crate) fn number(h: f64) -> Result<Number, Error> {
    if h.fract() == 0.0 && h.abs() < 1e9 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "whole and under 1e9, checked above"
        )]
        return Ok(Number::from(h as i64));
    }
    Number::from_f64(h).ok_or_else(|| Error::Usage(format!("hours {h} is not a number")))
}

/// What `iohr atlas scrum add` takes.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewCard {
    pub(crate) id: String,
    pub(crate) title: String,
    /// Project fields, `KEY=VALUE`, kept under the card's `x`.
    pub(crate) x: Map<String, Value>,
    pub(crate) size: String,
    pub(crate) hours: f64,
    /// Defaults to today.
    pub(crate) start: Option<String>,
    pub(crate) due: String,
    /// Atlas paths, routed by their folder: `prd/`, `rfc/`, `adr/` or `studies/`.
    pub(crate) docs: Vec<String>,
    pub(crate) depends_on: Vec<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) checklist: Vec<String>,
    pub(crate) acceptance: Vec<String>,
    pub(crate) tests: Vec<String>,
}

/// Adds a card to backlog.
pub(crate) fn add(b: &mut Board, n: NewCard, today: &str) -> Result<(), Error> {
    if b.list_of(&n.id).is_some() {
        return Err(Error::Usage(format!(
            "card {} is already on the board",
            n.id
        )));
    }
    let mut missing = Vec::new();
    for (name, v) in [
        ("--item", &n.checklist),
        ("--acceptance", &n.acceptance),
        ("--test", &n.tests),
    ] {
        if v.is_empty() {
            missing.push(name);
        }
    }
    if !missing.is_empty() {
        return Err(Error::Usage(format!(
            "a card needs at least one {}",
            missing.join(", one ")
        )));
    }
    let mut docs = Docs::default();
    for p in n.docs {
        let into = match p.split('/').next() {
            Some("prd") => &mut docs.prd,
            Some("rfc") => &mut docs.rfc,
            Some("adr") => &mut docs.adr,
            Some("studies") => &mut docs.study,
            _ => {
                return Err(Error::Usage(format!(
                    "docs path {p} must start with prd/, rfc/, adr/ or studies/"
                )));
            }
        };
        into.push(p);
    }
    b.cards.push(Card {
        id: n.id,
        title: n.title,
        list: "backlog".into(),
        labels: n.labels,
        size: n.size,
        hours: number(n.hours)?,
        start: n.start.unwrap_or_else(|| today.to_string()),
        due: n.due,
        depends_on: n.depends_on,
        blocked_by: Vec::new(),
        docs,
        builds_on: Vec::new(),
        checklist: n
            .checklist
            .into_iter()
            .map(|text| Check { text, done: false })
            .collect(),
        acceptance: n.acceptance,
        tests: n.tests,
        planner: None,
        pr: None,
        notes: None,
        x: n.x,
    });
    Ok(())
}

/// What `iohr atlas scrum edit` changes. Labels are `+x` to add (or plain `x`) and `-x` to remove.
#[derive(Debug, Clone, Default)]
pub(crate) struct Edit {
    pub(crate) title: Option<String>,
    pub(crate) due: Option<String>,
    pub(crate) start: Option<String>,
    pub(crate) hours: Option<f64>,
    pub(crate) size: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) blocked_by: Option<String>,
    pub(crate) unblock: bool,
}

/// Changes a card's fields. A moved due date keeps the old one in notes.
pub(crate) fn edit(b: &mut Board, id: &str, e: Edit, today: &str) -> Result<(), Error> {
    if e.blocked_by.is_some() && e.unblock {
        return Err(Error::Usage(
            "--blocked-by and --unblock cannot be used together".into(),
        ));
    }
    let c = b.card_mut(id)?;
    if let Some(t) = e.title {
        c.title = t;
    }
    if let Some(d) = e.due
        && d != c.due
    {
        let old = std::mem::replace(&mut c.due, d);
        let line = format!("{today}: due moved from {old} to {}", c.due);
        note_line(c, line);
    }
    if let Some(s) = e.start {
        c.start = s;
    }
    if let Some(h) = e.hours {
        c.hours = number(h)?;
    }
    if let Some(s) = e.size {
        c.size = s;
    }
    for l in e.labels {
        if let Some(gone) = l.strip_prefix('-') {
            c.labels.retain(|x| x != gone);
        } else {
            let l = l.strip_prefix('+').unwrap_or(&l).to_string();
            if !c.labels.contains(&l) {
                c.labels.push(l);
            }
        }
    }
    if let Some(x) = e.blocked_by
        && !c.blocked_by.contains(&x)
    {
        c.blocked_by.push(x);
    }
    if e.unblock {
        c.blocked_by.clear();
    }
    Ok(())
}

/// Moves a card to a list, keeping the guards:
///
/// - to `doing`: every `depends_on` is done and nothing is in `blocked_by`;
/// - to `done`: every checklist item is ticked, and the card has a `pr` or `evidence` is given.
///
/// `force` (a reason) overrides the guards and writes the reason and what it skipped to notes.
/// `evidence` is written to notes as "DATE: evidence: TEXT".
pub(crate) fn move_to(
    b: &mut Board,
    id: &str,
    list: &str,
    force: Option<&str>,
    evidence: Option<&str>,
    today: &str,
) -> Result<(), Error> {
    if !LISTS.contains(&list) {
        return Err(Error::Usage(format!(
            "no list {list}; the lists are {}",
            LISTS.join(", ")
        )));
    }
    if force.is_some_and(|r| r.trim().is_empty()) {
        return Err(Error::Usage("--force needs a reason".into()));
    }
    if evidence.is_some_and(|r| r.trim().is_empty()) {
        return Err(Error::Usage("--evidence needs a text".into()));
    }
    let c = b.card(id)?;
    if c.list == list {
        return Err(Error::Usage(format!("{id} is already in {list}")));
    }
    let mut why = Vec::new();
    if list == "doing" {
        for d in &c.depends_on {
            match b.list_of(d) {
                Some("done") => {}
                Some(l) => why.push(format!("{id} depends on {d}, which is in {l}")),
                None => why.push(format!("{id} depends on {d}, which is not on the board")),
            }
        }
        if !c.blocked_by.is_empty() {
            why.push(format!("{id} is blocked by: {}", c.blocked_by.join("; ")));
        }
    }
    if list == "done" {
        for (i, k) in c.checklist.iter().enumerate() {
            if !k.done {
                why.push(format!(
                    "checklist item {} is not ticked: {}",
                    i + 1,
                    k.text
                ));
            }
        }
        if c.pr.is_none() && evidence.is_none() {
            why.push("done needs --pr owner/repo#N or --evidence TEXT".into());
        }
    }
    if !why.is_empty() && force.is_none() {
        why.push("use --force REASON to move it anyway".into());
        return Err(Error::Refused(why));
    }
    let c = b.card_mut(id)?;
    c.list = list.to_string();
    if let Some(r) = force {
        let line = if why.is_empty() {
            format!("{today}: forced to {list}: {r}")
        } else {
            format!(
                "{today}: forced to {list}: {r} (skipped: {})",
                why.join("; ")
            )
        };
        note_line(c, line);
    }
    if let Some(e) = evidence {
        note_line(c, format!("{today}: evidence: {e}"));
    }
    Ok(())
}

/// Moves a card to `doing`.
pub(crate) fn start(
    b: &mut Board,
    id: &str,
    force: Option<&str>,
    today: &str,
) -> Result<(), Error> {
    move_to(b, id, "doing", force, None, today)
}

/// Sets the card's `pr` when given, then moves it to `done`.
pub(crate) fn done(
    b: &mut Board,
    id: &str,
    pr: Option<&str>,
    evidence: Option<&str>,
    force: Option<&str>,
    today: &str,
) -> Result<(), Error> {
    if let Some(p) = pr {
        b.card_mut(id)?.pr = Some(p.to_string());
    }
    move_to(b, id, "done", force, evidence, today)
}

/// Ticks checklist item `n` (1-based), or unticks it with `undo`.
pub(crate) fn tick(b: &mut Board, id: &str, n: usize, undo: bool) -> Result<(), Error> {
    let c = b.card_mut(id)?;
    let len = c.checklist.len();
    let k = n
        .checked_sub(1)
        .and_then(|i| c.checklist.get_mut(i))
        .ok_or_else(|| Error::Usage(format!("{id} has checklist items 1 to {len}, not {n}")))?;
    k.done = !undo;
    Ok(())
}

/// Adds a dated line to a card's notes.
pub(crate) fn note(b: &mut Board, id: &str, text: &str, today: &str) -> Result<(), Error> {
    if text.trim().is_empty() {
        return Err(Error::Usage("a note needs a text".into()));
    }
    note_line(b.card_mut(id)?, format!("{today}: {}", text.trim()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrum::board::tests::{board, card};

    const T: &str = "2026-11-14";

    #[test]
    fn moves_keep_their_guards() {
        let mut b = board(vec![card("aa", "ready", &[]), card("bb", "ready", &["aa"])]);
        b.cards[1].blocked_by = vec!["apr_x answer".into()];

        let err = start(&mut b, "bb", None, T).expect_err("guarded");
        assert_eq!(
            err.to_string(),
            "bb depends on aa, which is in ready\nbb is blocked by: apr_x answer\nuse --force REASON to move it anyway"
        );
        assert_eq!(err.code(), 1);
        assert_eq!(
            move_to(&mut b, "bb", "someday", None, None, T)
                .expect_err("list")
                .code(),
            2
        );
        assert_eq!(
            start(&mut b, "bb", Some(" "), T)
                .expect_err("empty reason")
                .code(),
            2
        );

        start(&mut b, "bb", Some("the owner said go"), T).expect("forced");
        assert_eq!(b.cards[1].list, "doing");
        let n = b.cards[1].notes.as_deref().unwrap_or_default();
        assert!(
            n.starts_with(
                "2026-11-14: forced to doing: the owner said go (skipped: bb depends on aa"
            ),
            "{n}"
        );

        start(&mut b, "aa", None, T).expect("no guard");
        let err = done(&mut b, "aa", None, None, None, T).expect_err("not ticked, no evidence");
        assert!(
            err.to_string()
                .starts_with("checklist item 1 is not ticked: one\nchecklist item 2"),
            "{err}"
        );
        assert!(
            err.to_string()
                .contains("done needs --pr owner/repo#N or --evidence TEXT")
        );

        tick(&mut b, "aa", 1, false).expect("tick");
        tick(&mut b, "aa", 2, false).expect("tick");
        assert_eq!(tick(&mut b, "aa", 3, false).expect_err("range").code(), 2);
        assert_eq!(tick(&mut b, "aa", 0, false).expect_err("1-based").code(), 2);
        let err = done(&mut b, "aa", None, None, None, T).expect_err("no evidence");
        assert_eq!(
            err.to_string(),
            "done needs --pr owner/repo#N or --evidence TEXT\nuse --force REASON to move it anyway"
        );

        done(
            &mut b,
            "aa",
            None,
            Some("cargo test -p iohr: 9 passed"),
            None,
            T,
        )
        .expect("evidence");
        assert_eq!(
            b.cards[0].notes.as_deref(),
            Some("2026-11-14: evidence: cargo test -p iohr: 9 passed")
        );

        let mut b = board(vec![card("cc", "review", &[])]);
        tick(&mut b, "cc", 1, false).expect("tick");
        tick(&mut b, "cc", 2, false).expect("tick");
        tick(&mut b, "cc", 2, true).expect("untick");
        assert!(!b.cards[0].checklist[1].done);
        tick(&mut b, "cc", 2, false).expect("tick");
        done(&mut b, "cc", Some("inorbithr/eye#12"), None, None, T).expect("pr");
        assert_eq!(b.cards[0].pr.as_deref(), Some("inorbithr/eye#12"));
        assert!(b.cards[0].notes.is_none());
    }

    #[test]
    #[allow(clippy::too_many_lines, reason = "one scenario read top to bottom")]
    fn add_and_edit_keep_history() {
        let mut b = board(vec![card("aa", "ready", &[])]);
        let n = NewCard {
            id: "bb".into(),
            title: "Card bb".into(),
            x: Map::from_iter([("section".to_string(), Value::from("fit"))]),
            size: "M".into(),
            hours: 4.0,
            due: "2026-11-16".into(),
            docs: vec![
                "rfc/0003-the-scrum-board-in-atlas.md".into(),
                "studies/0002-x.md".into(),
            ],
            depends_on: vec!["aa".into()],
            checklist: vec!["one".into()],
            acceptance: vec!["works".into()],
            tests: vec!["cargo test".into()],
            ..NewCard::default()
        };
        assert_eq!(
            add(
                &mut b,
                NewCard {
                    tests: vec![],
                    ..n.clone()
                },
                T
            )
            .expect_err("tests")
            .code(),
            2
        );
        assert_eq!(
            add(
                &mut b,
                NewCard {
                    docs: vec!["notes/x.md".into()],
                    ..n.clone()
                },
                T
            )
            .expect_err("docs")
            .code(),
            2
        );
        add(&mut b, n.clone(), T).expect("add");
        assert_eq!(add(&mut b, n, T).expect_err("twice").code(), 2);
        let c = &b.cards[1];
        assert_eq!(
            (c.list.as_str(), c.start.as_str(), c.hours.to_string()),
            ("backlog", T, "4".to_string())
        );
        assert_eq!(c.docs.study, vec!["studies/0002-x.md"]);

        let e = Edit {
            due: Some("2026-11-17".into()),
            hours: Some(4.5),
            labels: vec!["+ui".into(), "api".into()],
            blocked_by: Some("apr_1 answer".into()),
            ..Edit::default()
        };
        edit(&mut b, "bb", e, T).expect("edit");
        edit(
            &mut b,
            "bb",
            Edit {
                labels: vec!["-ui".into()],
                ..Edit::default()
            },
            T,
        )
        .expect("edit");
        let c = &b.cards[1];
        assert_eq!(
            c.notes.as_deref(),
            Some("2026-11-14: due moved from 2026-11-16 to 2026-11-17")
        );
        assert_eq!(
            (c.hours.to_string(), c.labels.clone()),
            ("4.5".to_string(), vec!["api".to_string()])
        );
        assert_eq!(c.blocked_by, vec!["apr_1 answer"]);
        let both = Edit {
            blocked_by: Some("x y z".into()),
            unblock: true,
            ..Edit::default()
        };
        assert_eq!(edit(&mut b, "bb", both, T).expect_err("both").code(), 2);
        edit(
            &mut b,
            "bb",
            Edit {
                unblock: true,
                ..Edit::default()
            },
            T,
        )
        .expect("unblock");
        assert_eq!(b.cards[1].blocked_by, Vec::<String>::new());
        note(&mut b, "bb", "asked the Supervisor", "2026-11-15").expect("note");
        assert!(
            b.cards[1]
                .notes
                .as_deref()
                .unwrap_or_default()
                .ends_with("\n2026-11-15: asked the Supervisor")
        );
        assert_eq!(note(&mut b, "zz", "x", T).expect_err("no card").code(), 2);
    }
}
