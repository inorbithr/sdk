//! The Workloads views of the panel's model: the list of the workload tree and one workload
//! opened in place (the design of the owner's approval `apr_01M6VYDKKHBQFWY4A3N78998ZV`).
//!
//! The rows come from [`workloads::tree`], joined to the open asks the model already holds, so a
//! session that an ask names shows as `needs you`. Anything the sources do not give is shown
//! as `not available`, never as a guess. The actions are real: message (a typed line to the
//! session's inbox), terminal (brings its window forward), transcript (read in place), pr (opens
//! it), approval (hands over to the Approvals view) and stop (two keys, then an interrupt).
//! Each is an [`Effect`] the proxy runs; the model only checks that the workload can take it.

use super::{Effect, Key, Model, Parts, Source, View, entry};
use crate::panel::cells::{
    AMBER, FAINT, GREEN, KEY, Line, MUTE, RED, Span, TXT, bold, one_wide, sp,
};
use crate::panel::workloads::{self, Role, Snapshot, State, Workload};

/// Rows of the list shown at once.
const VISIBLE: usize = 8;

/// The open view's actions: key, name.
const ACTIONS: [(char, &str); 6] = [
    ('m', "message"),
    ('t', "terminal"),
    ('l', "transcript"),
    ('p', "pr"),
    ('a', "approval"),
    ('x', "stop"),
];

fn colour(s: State) -> crate::card::Rgb {
    match s {
        State::Working => GREEN,
        State::Needs => AMBER,
        State::Idle => MUTE,
        State::Stuck => RED,
    }
}

fn dot(s: State) -> &'static str {
    match s {
        State::Needs => "◐",
        State::Idle => "○",
        State::Working | State::Stuck => "●",
    }
}

/// `s` cut to `max` characters with an ellipsis, its text made one cell wide first.
pub(super) fn cut(s: &str, max: usize) -> String {
    let s = one_wide(s);
    if s.chars().count() <= max {
        return s;
    }
    s.chars()
        .take(max.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

impl Model {
    /// Sets what the sources found, keeping the selection on the same workload.
    pub(in crate::panel) fn set_work(&mut self, work: Source<Snapshot>) {
        self.work = work;
        self.rebuild_work();
    }

    /// Folds the asks into the tree again (after either changed), keeping the selection.
    pub(super) fn rebuild_work(&mut self) {
        let keep = self.work_rows.get(self.work_sel).map(|w| w.id.clone());
        self.work_rows = match &self.work {
            Source::Ready(snap) => {
                let asks: Vec<_> = self
                    .list()
                    .iter()
                    .map(|a| (a.id.clone(), a.title.clone(), a.agent.clone()))
                    .collect();
                workloads::tree(snap, &asks)
            }
            _ => Vec::new(),
        };
        self.work_sel = keep
            .and_then(|id| self.work_rows.iter().position(|w| w.id == id))
            .unwrap_or(self.work_sel)
            .min(self.work_rows.len().saturating_sub(1));
        if self.view == View::Workload && self.opened().is_none() {
            self.view = View::Workloads;
        }
    }

    fn opened(&self) -> Option<&Workload> {
        let id = self.opened.as_deref()?;
        self.work_rows.iter().find(|w| w.id == id)
    }

    /// (running, needing you, stuck) over the sessions and jobs: the coordinator and the
    /// service rows are groups, not work, and are not counted.
    fn work_counts(&self) -> (usize, usize, usize) {
        let leaves = || {
            self.work_rows
                .iter()
                .filter(|w| matches!(w.role, Role::Session | Role::Job))
        };
        (
            leaves().filter(|w| w.state.running()).count(),
            leaves().filter(|w| w.state == State::Needs).count(),
            leaves().filter(|w| w.state == State::Stuck).count(),
        )
    }

    /// The menu's right-hand cell for Workloads: the running count, never a made-up one.
    pub(super) fn work_menu_right(&self) -> (String, crate::card::Rgb) {
        match &self.work {
            Source::Ready(_) => {
                let (running, ..) = self.work_counts();
                (
                    format!("{running} ›"),
                    if running > 0 { GREEN } else { MUTE },
                )
            }
            Source::Loading => ("… ›".to_owned(), MUTE),
            Source::Unavailable(_) => ("n/a ›".to_owned(), FAINT),
        }
    }

    pub(super) fn work_key(&mut self, key: Key) -> Effect {
        let len = self.work_rows.len();
        match key {
            Key::Esc => {
                self.view = View::Menu;
                self.note = None;
            }
            Key::Up | Key::Char('k') => {
                self.work_sel = self.work_sel.saturating_sub(1);
                self.note = None;
            }
            Key::Down | Key::Char('j') => {
                self.work_sel = (self.work_sel + 1).min(len.saturating_sub(1));
                self.note = None;
            }
            Key::Enter => {
                if let Some(w) = self.work_rows.get(self.work_sel) {
                    self.opened = Some(w.id.clone());
                    self.act_sel = 0;
                    self.note = None;
                    self.view = View::Workload;
                }
            }
            Key::Char('m') if len > 0 => {
                self.opened = self.work_rows.get(self.work_sel).map(|w| w.id.clone());
                return self.do_action('m');
            }
            Key::Char('t') if len > 0 => {
                self.opened = self.work_rows.get(self.work_sel).map(|w| w.id.clone());
                return self.do_action('t');
            }
            _ => {}
        }
        Effect::None
    }

    pub(super) fn work_open_key(&mut self, key: Key) -> Effect {
        if let Some(id) = self.stop_confirm.take() {
            self.note = None;
            return if matches!(key, Key::Char('y' | 'Y')) {
                Effect::StopWork(id)
            } else {
                Effect::None
            };
        }
        match key {
            Key::Esc => {
                self.view = View::Workloads;
                self.note = None;
                Effect::None
            }
            Key::Left => {
                self.act_sel = (self.act_sel + ACTIONS.len() - 1) % ACTIONS.len();
                self.note = None;
                Effect::None
            }
            Key::Right => {
                self.act_sel = (self.act_sel + 1) % ACTIONS.len();
                self.note = None;
                Effect::None
            }
            Key::Enter => self.do_action(ACTIONS[self.act_sel].0),
            Key::Char(c) => {
                let c = c.to_ascii_lowercase();
                match ACTIONS.iter().position(|a| a.0 == c) {
                    Some(i) => {
                        self.act_sel = i;
                        self.do_action(c)
                    }
                    None => Effect::None,
                }
            }
            _ => Effect::None,
        }
    }

    /// The opened workload's id when it is a session on this Mac; otherwise says so.
    fn opened_session(&mut self, what: &str) -> Option<String> {
        let id = self.opened().map(|w| w.id.clone())?;
        if id.starts_with("s:") {
            Some(id)
        } else {
            self.note = Some(format!("{what}: only a session on this Mac can"));
            None
        }
    }

    /// One action of the open view: approval opens the Approvals view on the ask this workload
    /// waits on; the others are effects the proxy runs, after the model has checked the
    /// workload can take them.
    fn do_action(&mut self, c: char) -> Effect {
        self.note = None;
        match c {
            'a' => {
                let ask = self.opened().and_then(|w| w.ask.clone());
                let at = ask.and_then(|id| self.list().iter().position(|a| a.id == id));
                if let Some(i) = at {
                    self.ask_sel = i;
                    self.view = View::Approvals;
                    return Effect::Refresh;
                }
                self.note = Some("no ask is waiting on this".into());
                Effect::None
            }
            'm' => {
                if let Some(id) = self.opened_session("message") {
                    self.begin_entry(entry::For::Message(id));
                }
                Effect::None
            }
            't' => self
                .opened_session("terminal")
                .map_or(Effect::None, Effect::Terminal),
            'l' => {
                let Some(id) = self.opened_session("transcript") else {
                    return Effect::None;
                };
                let title = self
                    .opened()
                    .map_or(String::new(), |w| format!("{} transcript", w.label));
                self.open_reader(&title, View::Workload, true);
                Effect::Transcript(id)
            }
            'p' => match self.opened().map(|w| (w.id.clone(), w.pr.clone())) {
                Some((id, pr)) => Effect::Pr { id, pr },
                None => Effect::None,
            },
            'x' => {
                self.stop_confirm = self.opened_session("stop");
                Effect::None
            }
            _ => Effect::None,
        }
    }

    fn note_line(&self) -> Option<Line> {
        self.note.as_ref().map(|n| Line {
            spans: vec![sp(&cut(n, 60), AMBER)],
            selected: false,
        })
    }

    pub(super) fn workloads(&self, width: usize) -> Parts {
        let more = vec![sp(" › ", FAINT), bold("Workloads", TXT)];
        let (running, needs, stuck) = self.work_counts();
        let mut right = Vec::new();
        if matches!(self.work, Source::Ready(_)) {
            if running == 0 {
                right.push(sp("none running", MUTE));
            } else {
                right.push(sp(&format!("{running} running"), GREEN));
            }
            if needs > 0 {
                right.extend([sp(" · ", FAINT), sp(&format!("{needs} need you"), AMBER)]);
            }
            if stuck > 0 {
                right.extend([sp(" · ", FAINT), sp(&format!("{stuck} stuck"), RED)]);
            }
        }
        let plain = |t: &str, fg| Line {
            spans: vec![sp(t, fg)],
            selected: false,
        };
        let mut body = Vec::new();
        match &self.work {
            Source::Loading => body.push(plain("Reading your workloads …", MUTE)),
            Source::Unavailable(why) => {
                body.push(plain("Workloads unavailable.", AMBER));
                body.push(plain(&cut(why, width - 4), MUTE));
            }
            Source::Ready(_) => {
                let start = self.work_sel.saturating_sub(VISIBLE - 1);
                for (i, w) in self.work_rows.iter().enumerate().skip(start).take(VISIBLE) {
                    body.push(Line {
                        spans: list_row(width, workloads::glyph(&self.work_rows, i), w),
                        selected: i == self.work_sel,
                    });
                }
            }
        }
        body.extend(self.entry_line(width.saturating_sub(6)));
        body.extend(self.note_line());
        let footer = self.entry_footer().unwrap_or_else(|| {
            vec![
                sp("↑↓", KEY),
                sp(" move  ", MUTE),
                sp("⏎", KEY),
                sp(" open  ", MUTE),
                sp("m", KEY),
                sp(" message  ", MUTE),
                sp("t", KEY),
                sp(" terminal  ", MUTE),
                sp("esc", KEY),
                sp(" back", MUTE),
            ]
        });
        (more, right, body, Some(footer))
    }

    pub(super) fn workload(&self, width: usize) -> Parts {
        let Some(w) = self.opened() else {
            return self.workloads(width);
        };
        let more = vec![
            sp(" › ", FAINT),
            sp("Workloads", MUTE),
            sp(" › ", FAINT),
            bold(&cut(&w.label, 30), TXT),
        ];
        let right = vec![
            sp(&format!("{} ", dot(w.state)), colour(w.state)),
            sp(w.state.word(), colour(w.state)),
        ];
        let room = width.saturating_sub(6 + 8);
        let field = |k: &str, v: Option<&str>, fg, missing: &str| {
            let (text, fg) = match v.filter(|v| !v.is_empty()) {
                Some(v) => (cut(v, room), fg),
                None => (missing.to_owned(), FAINT),
            };
            Line {
                spans: vec![sp(&format!("{k:<8}"), MUTE), sp(&text, fg)],
                selected: false,
            }
        };
        let na = "not available";
        let (pr, pr_missing) = (
            w.pr.as_deref(),
            if w.role == Role::Job { "none" } else { na },
        );
        let mut body = vec![
            field("Why", w.why.as_deref(), TXT, na),
            field("Latest", w.latest.as_deref(), TXT, na),
            field("Since", Some(&w.since()), TXT, na),
            field("Uses", w.uses.as_deref(), TXT, na),
            field("Waits", w.waits.as_deref(), AMBER, "nothing"),
            field("PR", pr, TXT, pr_missing),
            Line {
                spans: vec![],
                selected: false,
            },
        ];
        let mut acts: Vec<Span> = Vec::new();
        for (i, (k, n)) in ACTIONS.iter().enumerate() {
            if i == self.act_sel {
                acts.extend([bold(&k.to_string(), KEY), bold(&format!(" {n}  "), TXT)]);
            } else {
                acts.extend([sp(&k.to_string(), KEY), sp(&format!(" {n}  "), MUTE)]);
            }
        }
        body.push(Line {
            spans: acts,
            selected: true,
        });
        body.extend(self.entry_line(width.saturating_sub(6)));
        body.extend(self.note_line());
        let footer = if let Some(f) = self.entry_footer() {
            f
        } else if self.stop_confirm.is_some() {
            vec![
                sp("stop this session?  ", AMBER),
                sp("y", KEY),
                sp(" yes   ", MUTE),
                sp("n", KEY),
                sp(" no", MUTE),
            ]
        } else {
            vec![
                sp("←→", KEY),
                sp(" pick   ", MUTE),
                sp("⏎", KEY),
                sp(" do it   ", MUTE),
                sp("esc", KEY),
                sp(" back to list", MUTE),
            ]
        };
        (more, right, body, Some(footer))
    }
}

/// A list row: the tree glyph, the state dot, the label, its model or where it runs, and the
/// time cell at the right edge; the label gives way when the row is too long.
fn list_row(width: usize, glyph: &str, w: &Workload) -> Vec<Span> {
    let right = w.right();
    let sub = one_wide(&w.sub);
    let fixed = glyph.chars().count() + 2 + right.chars().count() + 1;
    let room = width.saturating_sub(6 + fixed);
    let sub_len = if sub.is_empty() {
        0
    } else {
        sub.chars().count() + 2
    };
    let label = cut(
        &w.label,
        room.saturating_sub(sub_len).max(8).min(room.max(1)),
    );
    let sub = if label.chars().count() + sub_len > room {
        String::new()
    } else {
        sub
    };
    let used = glyph.chars().count()
        + 2
        + label.chars().count()
        + if sub.is_empty() {
            0
        } else {
            sub.chars().count() + 2
        }
        + right.chars().count();
    let pad = width.saturating_sub(6 + used).max(1);
    let rfg = match w.state {
        State::Needs => AMBER,
        State::Stuck => RED,
        State::Working if w.role == Role::Root => GREEN,
        _ => MUTE,
    };
    let mut spans = vec![
        sp(glyph, FAINT),
        sp(&format!("{} ", dot(w.state)), colour(w.state)),
        sp(&label, TXT),
    ];
    if !sub.is_empty() {
        spans.push(sp(&format!("  {sub}"), MUTE));
    }
    spans.extend([sp(&" ".repeat(pad), TXT), sp(&right, rfg)]);
    spans
}
