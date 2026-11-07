//! The panel's state machine: which view is open, what is selected, what each key does.
//!
//! Pure. A key goes in, an [`Effect`] comes out (nothing, a deny to send, stop the panel,
//! refresh the asks); a view goes out as a [`BoxSpec`] for [`super::cells`] to lay out.
//! Data that has not arrived, or could not be read, is shown as such, never as a number.
//!
//! States: `Rest` (the pill; keys go to the shell) and the open views `Menu`, `Approvals`,
//! `Workloads`, `Workload` (one opened), `Auto`, `Run`, `Status`, `Help` (the panel swallows
//! every key). The menu key toggles
//! between `Rest` and `Menu`; `Esc` goes one level back (Approvals, Auto, Run, Status and Help
//! to the menu, the menu to rest); a deny is two keys (`d`, then `y`), `n` or `Esc` cancels it.

mod work;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::cells::{
    ACC, AMBER, BoxSpec, FAINT, GREEN, KEY, Line, MUTE, Span, TXT, bold, one_wide, sp,
};
use super::workloads::{Snapshot, Workload};
use crate::card::Rgb;
use crate::commands::asks::PanelAsk;

/// Where a piece of data stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Source<T> {
    Loading,
    Ready(T),
    Unavailable(String),
}

/// An ask as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Ask {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) kind: String,
    pub(super) age: String,
    /// The name the asking agent gives itself (to find the workload that waits on it).
    pub(super) agent: String,
}

impl Ask {
    /// From the service's ask, with the age measured at `now`.
    pub(super) fn from(a: &PanelAsk, now: OffsetDateTime) -> Self {
        Self {
            id: a.id.clone(),
            title: one_wide(&a.title),
            kind: one_wide(&a.kind),
            age: age(&a.created_at, now),
            agent: a.agent.clone(),
        }
    }
}

/// "4m", "1h", "2d": how long ago `created_at` (RFC 3339) was at `now`; "now" under a
/// minute, "?" when it does not parse.
pub(super) fn age(created_at: &str, now: OffsetDateTime) -> String {
    let Ok(at) = OffsetDateTime::parse(created_at, &Rfc3339) else {
        return "?".into();
    };
    let secs = (now - at).whole_seconds().max(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Rest,
    Menu,
    Approvals,
    Workloads,
    Workload,
    Run,
    Status,
    Auto,
    Help,
}

/// A key the panel understands, already parsed from the terminal's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    Up,
    Down,
    Enter,
    Esc,
    Left,
    Right,
    Char(char),
    /// The panel's key, Option+i.
    Menu,
}

/// What the key asks the proxy to do beyond redrawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Effect {
    None,
    /// Deny this ask (the person confirmed).
    Deny(String),
    /// Read the asks again.
    Refresh,
    /// Stop the panel: the shell goes on without it.
    Stop,
}

/// The menu: label, key, view it opens.
const MENU: [(&str, char, View); 7] = [
    ("Auto mode", 'a', View::Auto),
    ("Approvals", 'p', View::Approvals),
    ("Workloads", 'w', View::Workloads),
    ("Run", 'r', View::Run),
    ("Status", 's', View::Status),
    ("Help", '?', View::Help),
    ("Stop panel", 'x', View::Rest),
];

/// Rows of asks shown at once.
const VISIBLE: usize = 5;

#[derive(Debug, Clone)]
pub(super) struct Model {
    pub(super) view: View,
    menu_sel: usize,
    ask_sel: usize,
    pub(super) asks: Source<Vec<Ask>>,
    /// The ask mode the shell started with (`manual`, `plan`, `auto`, ...).
    pub(super) mode: Source<String>,
    confirm: Option<String>,
    note: Option<String>,
    stub: bool,
    /// What the sessions registry and the coordinator's picture gave.
    pub(super) work: Source<Snapshot>,
    /// The workload tree as rows, the asks folded in.
    work_rows: Vec<Workload>,
    work_sel: usize,
    /// The workload shown in place (`View::Workload`), by id.
    opened: Option<String>,
    act_sel: usize,
}

impl Model {
    pub(super) fn new(stub: bool) -> Self {
        Self {
            view: View::Rest,
            menu_sel: 0,
            ask_sel: 0,
            asks: Source::Loading,
            mode: Source::Loading,
            confirm: None,
            note: None,
            stub,
            work: Source::Loading,
            work_rows: Vec::new(),
            work_sel: 0,
            opened: None,
            act_sel: 0,
        }
    }

    /// Whether the panel is taking the keys.
    pub(super) fn open(&self) -> bool {
        self.view != View::Rest
    }

    /// Sets the asks, keeping the selection on the same ask when it is still there.
    pub(super) fn set_asks(&mut self, asks: Source<Vec<Ask>>) {
        let keep = self.selected_ask().map(|a| a.id.clone());
        self.asks = asks;
        let len = self.list().len();
        self.ask_sel = keep
            .and_then(|id| self.list().iter().position(|a| a.id == id))
            .unwrap_or(self.ask_sel)
            .min(len.saturating_sub(1));
        self.rebuild_work();
        if self
            .confirm
            .as_ref()
            .is_some_and(|id| self.list().iter().all(|a| &a.id != id))
        {
            self.confirm = None;
        }
    }

    /// A deny went through: the ask is gone from the list.
    pub(super) fn denied(&mut self, id: &str) {
        if let Source::Ready(list) = &mut self.asks {
            list.retain(|a| a.id != id);
        }
        self.note = Some("denied".into());
        self.set_asks(self.asks.clone());
    }

    /// A deny failed: say so, in the panel.
    pub(super) fn deny_failed(&mut self, why: &str) {
        self.note = Some(format!("deny failed: {}", one_wide(why)));
    }

    fn list(&self) -> &[Ask] {
        match &self.asks {
            Source::Ready(l) => l,
            _ => &[],
        }
    }

    fn selected_ask(&self) -> Option<&Ask> {
        self.list().get(self.ask_sel)
    }

    /// One key. In `Rest` only the menu key does anything.
    pub(super) fn key(&mut self, key: Key) -> Effect {
        if key == Key::Menu {
            self.note = None;
            self.confirm = None;
            return if self.open() {
                self.view = View::Rest;
                Effect::None
            } else {
                self.view = View::Menu;
                Effect::Refresh
            };
        }
        match self.view {
            View::Rest => Effect::None,
            View::Menu => self.menu_key(key),
            View::Approvals => self.approvals_key(key),
            View::Workloads => self.work_key(key),
            View::Workload => self.work_open_key(key),
            View::Auto | View::Run | View::Status | View::Help => {
                if key == Key::Esc {
                    self.view = View::Menu;
                }
                Effect::None
            }
        }
    }

    fn menu_key(&mut self, key: Key) -> Effect {
        match key {
            Key::Esc => {
                self.view = View::Rest;
                Effect::None
            }
            Key::Up => {
                self.menu_sel = (self.menu_sel + MENU.len() - 1) % MENU.len();
                Effect::None
            }
            Key::Down | Key::Char('j') => {
                self.menu_sel = (self.menu_sel + 1) % MENU.len();
                Effect::None
            }
            Key::Char('k') => self.menu_key(Key::Up),
            Key::Enter => self.choose(self.menu_sel),
            Key::Char(c) => MENU
                .iter()
                .position(|m| m.1 == c.to_ascii_lowercase())
                .map_or(Effect::None, |i| {
                    self.menu_sel = i;
                    self.choose(i)
                }),
            Key::Menu | Key::Left | Key::Right => Effect::None,
        }
    }

    fn choose(&mut self, i: usize) -> Effect {
        let (_, _, view) = MENU[i];
        if i == MENU.len() - 1 {
            return Effect::Stop;
        }
        self.view = view;
        self.note = None;
        if matches!(view, View::Approvals | View::Workloads) {
            Effect::Refresh
        } else {
            Effect::None
        }
    }

    fn approvals_key(&mut self, key: Key) -> Effect {
        if let Some(id) = self.confirm.clone() {
            self.confirm = None;
            return if matches!(key, Key::Char('y' | 'Y')) {
                Effect::Deny(id)
            } else {
                Effect::None
            };
        }
        let len = self.list().len();
        match key {
            Key::Esc => {
                self.view = View::Menu;
                self.note = None;
            }
            Key::Up | Key::Char('k') => {
                self.ask_sel = self.ask_sel.saturating_sub(1);
                self.note = None;
            }
            Key::Down | Key::Char('j') => {
                self.ask_sel = (self.ask_sel + 1).min(len.saturating_sub(1));
                self.note = None;
            }
            Key::Char(c @ '1'..='9') => {
                let n = usize::from(c as u8 - b'1');
                if n < len {
                    self.ask_sel = n;
                }
            }
            Key::Char('d') if len > 0 => self.confirm = self.selected_ask().map(|a| a.id.clone()),
            Key::Char('q') if len > 0 => self.note = Some("question".into()),
            Key::Char('o') if len > 0 => self.note = Some("more".into()),
            Key::Enter if len > 0 => self.note = Some("phone".into()),
            _ => {}
        }
        Effect::None
    }

    /// The width the current view wants.
    pub(super) fn width(&self) -> usize {
        match self.view {
            View::Rest => 44,
            View::Approvals => 50,
            View::Workloads => 60,
            View::Workload => 72,
            _ => 42,
        }
    }

    /// The current view as a box of `width` columns.
    pub(super) fn spec(&self, width: usize) -> BoxSpec {
        let mut title = vec![sp("◆ ", ACC), bold("iohr", ACC)];
        if self.stub {
            title.push(sp(" · stub", FAINT));
        }
        let (title_more, right, body, footer) = match self.view {
            View::Rest => (vec![], vec![], vec![self.pill()], None),
            View::Menu => self.menu(width),
            View::Approvals => self.approvals(width),
            View::Workloads => self.workloads(width),
            View::Workload => self.workload(width),
            View::Auto => self.auto(),
            View::Run => Self::coming("Run", "run a command with an agent from here"),
            View::Status => Self::coming("Status", "platform and agent status"),
            View::Help => Self::help(),
        };
        title.extend(title_more);
        BoxSpec {
            width,
            title,
            right,
            body,
            footer,
        }
    }

    fn mode_spans(&self) -> Vec<Span> {
        match &self.mode {
            Source::Ready(m) if m == "auto" => {
                vec![sp("auto ", MUTE), sp("●", GREEN), sp(" on", GREEN)]
            }
            Source::Ready(_) => vec![sp("auto ", MUTE), sp("○", MUTE), sp(" off", MUTE)],
            Source::Loading => vec![sp("auto ", MUTE), sp("…", MUTE)],
            Source::Unavailable(_) => vec![sp("auto ", MUTE), sp("unavailable", FAINT)],
        }
    }

    fn waiting_spans(&self) -> Vec<Span> {
        match &self.asks {
            Source::Ready(l) if l.is_empty() => vec![sp("nothing waiting", MUTE)],
            Source::Ready(l) => vec![bold(&l.len().to_string(), AMBER), sp(" waiting", AMBER)],
            Source::Loading => vec![sp("asks …", MUTE)],
            Source::Unavailable(_) => vec![sp("asks unavailable", FAINT)],
        }
    }

    fn pill(&self) -> Line {
        let mut spans = self.mode_spans();
        spans.push(sp("   ", TXT));
        spans.extend(self.waiting_spans());
        spans.extend([sp("   ", TXT), sp("⌥i", KEY), sp(" menu", MUTE)]);
        Line {
            spans,
            selected: false,
        }
    }

    fn menu(&self, width: usize) -> Parts {
        let waiting = match &self.asks {
            Source::Ready(l) => (format!("{} ›", l.len()), AMBER),
            Source::Loading => ("… ›".to_owned(), MUTE),
            Source::Unavailable(_) => ("n/a ›".to_owned(), FAINT),
        };
        let mode = match &self.mode {
            Source::Ready(m) if m == "auto" => ("● on".to_owned(), GREEN),
            Source::Ready(_) => ("○ off".to_owned(), MUTE),
            Source::Loading => ("…".to_owned(), MUTE),
            Source::Unavailable(_) => ("n/a".to_owned(), FAINT),
        };
        let rights = [
            mode,
            waiting,
            self.work_menu_right(),
            ("›".into(), MUTE),
            ("›".into(), MUTE),
            (String::new(), MUTE),
            (String::new(), MUTE),
        ];
        let body = MENU
            .iter()
            .zip(rights)
            .enumerate()
            .map(|(i, ((label, key, _), (right, fg)))| Line {
                spans: item(label, &right, &key.to_string(), width, fg),
                selected: i == self.menu_sel,
            })
            .collect();
        let footer = vec![
            sp("↑↓", KEY),
            sp(" move   ", MUTE),
            sp("⏎", KEY),
            sp(" open   ", MUTE),
            sp("esc", KEY),
            sp(" close", MUTE),
        ];
        (vec![], self.mode_spans(), body, Some(footer))
    }

    fn approvals(&self, width: usize) -> Parts {
        let more = vec![sp(" › ", FAINT), bold("Approvals", TXT)];
        let right = match &self.asks {
            Source::Ready(l) if !l.is_empty() => vec![sp(&format!("{} waiting", l.len()), AMBER)],
            _ => vec![],
        };
        let plain = |t: &str, fg| Line {
            spans: vec![sp(t, fg)],
            selected: false,
        };
        let mut body = Vec::new();
        match &self.asks {
            Source::Loading => body.push(plain("Reading your asks …", MUTE)),
            Source::Unavailable(why) => {
                body.push(plain("Asks unavailable.", AMBER));
                body.push(plain(&one_wide(why), MUTE));
            }
            Source::Ready(l) if l.is_empty() => {
                body.push(plain("Nothing is waiting for you.", MUTE));
            }
            Source::Ready(l) => {
                let start = self.ask_sel.saturating_sub(VISIBLE - 1);
                for (i, a) in l.iter().enumerate().skip(start).take(VISIBLE) {
                    let selected = i == self.ask_sel;
                    let n = if i < 9 {
                        (i + 1).to_string()
                    } else {
                        " ".into()
                    };
                    body.push(Line {
                        spans: item(&a.title, &a.age, &n, width, MUTE),
                        selected,
                    });
                    if selected {
                        body.extend(self.expanded(a));
                    }
                }
            }
        }
        let footer = if self.confirm.is_some() {
            vec![
                sp("deny this ask?  ", AMBER),
                sp("y", KEY),
                sp(" yes   ", MUTE),
                sp("n", KEY),
                sp(" no", MUTE),
            ]
        } else {
            vec![
                sp("↑↓", KEY),
                sp(" move   ", MUTE),
                sp("⏎", KEY),
                sp(" choose   ", MUTE),
                sp("esc", KEY),
                sp(" back", MUTE),
            ]
        };
        (more, right, body, Some(footer))
    }

    /// The selected ask's detail lines (selected, so the tint spans them).
    fn expanded(&self, a: &Ask) -> Vec<Line> {
        let line = |spans| Line {
            spans,
            selected: true,
        };
        let kind = if a.kind.is_empty() { "ask" } else { &a.kind };
        let mut out = vec![
            line(vec![sp(&format!("  {kind} · approve on your phone"), MUTE)]),
            line(vec![
                sp("  ", TXT),
                sp("d", KEY),
                sp(" deny   ", MUTE),
                sp("q", KEY),
                sp(" ask   ", MUTE),
                sp("o", KEY),
                sp(" more", MUTE),
            ]),
        ];
        match self.note.as_deref() {
            Some("question") => out.extend([
                line(vec![sp("  to ask: iohr asks question", MUTE)]),
                line(vec![sp(&format!("  {}", a.id), KEY)]),
            ]),
            Some("more") => out.extend([
                line(vec![sp("  to read it: iohr asks show", MUTE)]),
                line(vec![sp(&format!("  {}", a.id), KEY)]),
            ]),
            Some("phone") => out.push(line(vec![sp("  approving needs your phone's key", AMBER)])),
            Some(other) => out.push(line(vec![sp(&format!("  {other}"), AMBER)])),
            None => {}
        }
        out
    }

    fn auto(&self) -> Parts {
        let plain = |t: &str, fg| Line {
            spans: vec![sp(t, fg)],
            selected: false,
        };
        let mode = match &self.mode {
            Source::Ready(m) => format!("Ask mode at start: {m}"),
            Source::Loading => "Ask mode at start: …".into(),
            Source::Unavailable(why) => format!("Ask mode unavailable: {why}"),
        };
        let body = vec![
            plain(&mode, TXT),
            plain("`auto` shows as on; any other mode as off.", MUTE),
            plain("Auto-approve from the phone: coming next.", FAINT),
        ];
        (
            vec![sp(" › ", FAINT), bold("Auto mode", TXT)],
            vec![],
            body,
            Some(back_footer()),
        )
    }

    fn coming(name: &str, what: &str) -> Parts {
        let plain = |t: &str, fg| Line {
            spans: vec![sp(t, fg)],
            selected: false,
        };
        let body = vec![
            plain(&format!("{name}: coming next."), FAINT),
            plain(what, MUTE),
            plain("Not built yet.", MUTE),
        ];
        (
            vec![sp(" › ", FAINT), bold(name, TXT)],
            vec![],
            body,
            Some(back_footer()),
        )
    }

    fn help() -> Parts {
        let row = |k: &str, t: &str| Line {
            spans: vec![sp(k, KEY), sp(&format!("  {t}"), MUTE)],
            selected: false,
        };
        let body = vec![
            row("⌥i", "open or close this panel"),
            row("↑↓ ⏎", "move and open"),
            row("esc", "back, then close"),
            row("d y", "deny an ask, then confirm"),
            row("", "your command line is kept as is"),
        ];
        (
            vec![sp(" › ", FAINT), bold("Help", TXT)],
            vec![],
            body,
            Some(back_footer()),
        )
    }
}

type Parts = (Vec<Span>, Vec<Span>, Vec<Line>, Option<Vec<Span>>);

fn back_footer() -> Vec<Span> {
    vec![sp("esc", KEY), sp(" back", MUTE)]
}

/// A row: label left, a value, then the key, in `width` columns (the design's `item`); a
/// label too long for the row is cut with an ellipsis.
fn item(label: &str, right: &str, key: &str, width: usize, rfg: Rgb) -> Vec<Span> {
    let rlen = right.chars().count();
    let label_max = width.saturating_sub(6 + rlen + 3 + 1).max(1);
    let label: String = if label.chars().count() > label_max {
        label
            .chars()
            .take(label_max - 1)
            .chain(std::iter::once('…'))
            .collect()
    } else {
        label.to_owned()
    };
    let pad = width
        .saturating_sub(6 + label.chars().count() + rlen + 3)
        .max(1);
    vec![
        sp(&label, TXT),
        sp(&" ".repeat(pad), TXT),
        sp(right, rfg),
        sp("  ", TXT),
        sp(key, KEY),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(id: &str, title: &str) -> Ask {
        Ask {
            id: id.into(),
            title: title.into(),
            kind: "pull request".into(),
            age: "4m".into(),
            agent: String::new(),
        }
    }

    fn model(n: usize) -> Model {
        let mut m = Model::new(false);
        m.set_asks(Source::Ready(
            (0..n)
                .map(|i| ask(&format!("apr_{i}"), &format!("Ask {i}")))
                .collect(),
        ));
        m.mode = Source::Ready("auto".into());
        m
    }

    fn rows(m: &Model) -> Vec<String> {
        let f = super::super::cells::lay_out(&m.spec(m.width()), 0, 0);
        f.cells
            .iter()
            .map(|r| r.iter().map(|c| c.ch).collect())
            .collect()
    }

    #[test]
    fn age_is_minutes_hours_days() {
        let now = OffsetDateTime::parse("2026-11-07T12:00:00Z", &Rfc3339).expect("time");
        assert_eq!(age("2026-11-07T11:56:00Z", now), "4m");
        assert_eq!(age("2026-11-07T11:59:30Z", now), "now");
        assert_eq!(age("2026-11-07T09:00:00Z", now), "3h");
        assert_eq!(age("2026-11-05T12:00:00Z", now), "2d");
        assert_eq!(age("yesterday", now), "?");
    }

    #[test]
    fn the_pill_says_what_it_knows_and_labels_what_it_does_not() {
        let mut m = model(3);
        assert!(
            rows(&m)[1].contains("auto ● on   3 waiting   ⌥i menu"),
            "{:?}",
            rows(&m)
        );
        m.set_asks(Source::Unavailable("signed out".into()));
        assert!(rows(&m)[1].contains("asks unavailable"));
        m.set_asks(Source::Loading);
        assert!(rows(&m)[1].contains("asks …"));
        m.mode = Source::Ready("manual".into());
        assert!(rows(&m)[1].contains("auto ○ off"));
        m.mode = Source::Unavailable("x".into());
        assert!(rows(&m)[1].contains("auto unavailable"));
        assert_eq!(
            Model::new(true)
                .spec(44)
                .title
                .last()
                .map(|s| s.text.as_str()),
            Some(" · stub")
        );
    }

    #[test]
    fn the_menu_key_opens_and_closes_and_rest_ignores_every_other_key() {
        let mut m = model(1);
        assert_eq!(m.key(Key::Char('p')), Effect::None);
        assert_eq!(m.view, View::Rest);
        assert_eq!(m.key(Key::Menu), Effect::Refresh);
        assert_eq!(m.view, View::Menu);
        assert!(m.open());
        assert_eq!(m.key(Key::Menu), Effect::None);
        assert_eq!(m.view, View::Rest);
        m.key(Key::Menu);
        m.key(Key::Esc);
        assert_eq!(m.view, View::Rest, "esc closes the menu");
    }

    #[test]
    fn the_menu_moves_wraps_and_opens_by_key_or_enter() {
        let mut m = model(2);
        m.key(Key::Menu);
        m.key(Key::Up);
        assert_eq!(m.menu_sel, 6, "wraps to Stop panel");
        m.key(Key::Down);
        assert_eq!(m.menu_sel, 0);
        let text = rows(&m);
        assert!(text[1].contains("Auto mode") && text[1].contains("● on"));
        assert!(text[2].contains("Approvals") && text[2].contains("2 ›"));
        assert_eq!(m.key(Key::Char('p')), Effect::Refresh);
        assert_eq!(m.view, View::Approvals);
        m.key(Key::Esc);
        assert_eq!(m.view, View::Menu);
        m.key(Key::Char('r'));
        assert_eq!(m.view, View::Run);
        assert!(rows(&m).join("\n").contains("coming next"));
        m.key(Key::Esc);
        assert_eq!(m.key(Key::Char('x')), Effect::Stop);
    }

    #[test]
    fn approvals_expand_the_selected_ask_and_deny_asks_first() {
        let mut m = model(3);
        m.key(Key::Menu);
        m.key(Key::Char('p'));
        let text = rows(&m).join("\n");
        assert!(text.contains("Approvals") && text.contains("3 waiting"));
        assert!(text.contains("pull request · approve on your phone"));
        assert!(text.contains("d deny   q ask   o more"));
        assert!(!text.contains("Ask 1 ") || text.matches("approve on your phone").count() == 1);
        m.key(Key::Down);
        assert_eq!(m.ask_sel, 1);
        assert_eq!(m.key(Key::Char('d')), Effect::None, "d only asks");
        assert!(rows(&m).join("\n").contains("deny this ask?"));
        assert_eq!(m.key(Key::Char('n')), Effect::None);
        assert!(!rows(&m).join("\n").contains("deny this ask?"));
        m.key(Key::Char('d'));
        assert_eq!(m.key(Key::Char('y')), Effect::Deny("apr_1".into()));
        m.denied("apr_1");
        assert_eq!(m.list().len(), 2);
        assert!(rows(&m).join("\n").contains("denied"));
    }

    #[test]
    fn approvals_show_unavailable_empty_and_the_commands_for_question_and_more() {
        let mut m = model(1);
        m.key(Key::Menu);
        m.key(Key::Char('p'));
        m.key(Key::Char('q'));
        let t = rows(&m).join("\n");
        assert!(
            t.contains("iohr asks question") && t.contains("apr_0"),
            "{t}"
        );
        m.key(Key::Char('o'));
        assert!(rows(&m).join("\n").contains("iohr asks show"));
        m.key(Key::Enter);
        assert!(rows(&m).join("\n").contains("needs your phone"));
        m.set_asks(Source::Ready(vec![]));
        assert!(rows(&m).join("\n").contains("Nothing is waiting"));
        assert_eq!(m.key(Key::Char('d')), Effect::None, "nothing to deny");
        m.set_asks(Source::Unavailable("signed out".into()));
        let t = rows(&m).join("\n");
        assert!(t.contains("Asks unavailable.") && t.contains("signed out"));
    }

    #[test]
    fn the_list_scrolls_with_the_selection_and_keeps_it_when_the_asks_refresh() {
        let mut m = model(8);
        m.key(Key::Menu);
        m.key(Key::Char('p'));
        for _ in 0..7 {
            m.key(Key::Down);
        }
        assert_eq!(m.ask_sel, 7);
        assert!(rows(&m).join("\n").contains("Ask 7"));
        // The list shrinks under the selection: it clamps; an unchanged list keeps it.
        m.set_asks(Source::Ready(
            (0..8).map(|i| ask(&format!("apr_{i}"), "x")).collect(),
        ));
        assert_eq!(m.ask_sel, 7);
        m.set_asks(Source::Ready(vec![ask("apr_0", "x")]));
        assert_eq!(m.ask_sel, 0);
        m.key(Key::Char('d'));
        m.set_asks(Source::Ready(vec![]));
        assert_eq!(
            m.confirm, None,
            "a pending deny is dropped when its ask is gone"
        );
    }

    /// A model with the design's workloads and one ask that names the "Fix sdk #448 reads" session.
    fn work_model() -> Model {
        let mut m = model(1);
        m.set_work(Source::Ready(super::super::workloads::stub()));
        let mut a = ask("apr_0", "Merge sdk #448 to main");
        a.agent = "Fix sdk #448 reads".into();
        m.set_asks(Source::Ready(vec![a]));
        m
    }

    #[test]
    fn the_menu_counts_what_is_running_and_the_list_is_the_workload_tree() {
        let mut m = work_model();
        m.key(Key::Menu);
        let text = rows(&m);
        assert!(
            text[3].contains("Workloads") && text[3].contains("4 ›") && text[3].contains("4 ›  w"),
            "{text:?}"
        );
        assert_eq!(m.key(Key::Char('w')), Effect::Refresh);
        assert_eq!(m.view, View::Workloads);
        let t = rows(&m);
        assert!(t[0].contains("iohr › Workloads"), "{t:?}");
        assert!(t[0].contains("4 running · 1 need you"), "{t:?}");
        let by = |s: &str| {
            t.iter()
                .find(|r| r.contains(s))
                .cloned()
                .unwrap_or_default()
        };
        assert!(by("Coordinator  this Mac").contains("needs you"), "{t:?}");
        assert!(by("├ ◐ Fix sdk #448 reads  Sonnet").contains("needs you"));
        assert!(by("├ ● Floating band builder").contains("14 min"));
        assert!(by("└ ○ Atlas sync  job").contains("idle 18 min"));
        assert!(by("Coordinator service  queue").contains("3 jobs"));
        assert!(by("esc back").contains("↑↓ move  ⏎ open  m message  t terminal"));
        assert_eq!(m.key(Key::Char('m')), Effect::None);
        assert!(rows(&m).join("\n").contains("message: coming next"));
        m.key(Key::Esc);
        assert_eq!(m.view, View::Menu);
    }

    #[test]
    fn opening_one_shows_its_fields_picks_actions_and_hands_approval_to_approvals() {
        let mut m = work_model();
        m.key(Key::Menu);
        m.key(Key::Char('w'));
        for _ in 0..3 {
            m.key(Key::Down);
        }
        assert_eq!(m.key(Key::Enter), Effect::None);
        assert_eq!(m.view, View::Workload);
        assert_eq!(m.width(), 72);
        let t = rows(&m);
        let all = t.join("\n");
        assert!(t[0].contains("Workloads › Fix sdk #448 reads"), "{all}");
        assert!(t[0].contains("◐ needs you"), "{all}");
        for want in [
            "Why     clear the security hold on sdk #448",
            "Latest  rewriting grant_access in ask.rs",
            "Since   needs you  ·  Sonnet  ·  under Coordinator",
            "Uses    412 MB  ·  38% CPU",
            "Waits   Merge sdk #448 to main  ·  approve on your phone",
            "PR      inorbithr/sdk #448  draft  ·  3 checks failing",
            "m message  t terminal  l transcript  p pr  a approval  x stop",
            "←→ pick   ⏎ do it   esc back to list",
        ] {
            assert!(all.contains(want), "missing {want:?} in\n{all}");
        }
        m.key(Key::Right);
        assert_eq!(m.act_sel, 1);
        m.key(Key::Left);
        m.key(Key::Left);
        assert_eq!(m.act_sel, 5, "wraps to stop");
        assert_eq!(m.key(Key::Enter), Effect::None);
        assert!(rows(&m).join("\n").contains("stop: coming next"));
        assert_eq!(m.key(Key::Char('l')), Effect::None);
        assert!(rows(&m).join("\n").contains("transcript: coming next"));
        assert_eq!(m.key(Key::Char('a')), Effect::Refresh);
        assert_eq!(m.view, View::Approvals, "approval is the Approvals view");
        assert_eq!(m.ask_sel, 0);
        m.key(Key::Esc);
        m.key(Key::Char('w'));
        m.key(Key::Enter);
        m.key(Key::Esc);
        assert_eq!(m.view, View::Workloads, "esc goes back to the list");
    }

    #[test]
    fn what_a_source_does_not_give_is_not_available_and_a_workload_without_an_ask_has_no_approval()
    {
        let mut m = work_model();
        m.key(Key::Menu);
        m.key(Key::Char('w'));
        m.key(Key::Down);
        m.key(Key::Enter);
        let all = rows(&m).join("\n");
        assert!(all.contains("Uses    388 MB"), "{all}");
        m.key(Key::Char('a'));
        assert!(rows(&m).join("\n").contains("no ask is waiting on this"));
        assert_eq!(m.view, View::Workload);
        m.key(Key::Esc);
        m.key(Key::Down);
        m.key(Key::Down);
        m.key(Key::Down);
        m.key(Key::Enter);
        assert!(rows(&m).join("\n").contains("Uses    not available"));
        assert!(rows(&m).join("\n").contains("PR      not available"));
    }

    #[test]
    fn workloads_loading_unavailable_and_gone_are_said_not_counted() {
        let mut m = model(0);
        m.key(Key::Menu);
        assert!(rows(&m)[3].contains("… ›"));
        m.key(Key::Char('w'));
        assert!(rows(&m).join("\n").contains("Reading your workloads"));
        m.set_work(Source::Unavailable("disk gone".into()));
        let t = rows(&m).join("\n");
        assert!(t.contains("Workloads unavailable.") && t.contains("disk gone"));
        m.key(Key::Esc);
        assert!(rows(&m)[3].contains("n/a ›"));
        let mut snap = super::super::workloads::stub();
        snap.sessions.clear();
        m.set_work(Source::Ready(snap));
        m.key(Key::Char('w'));
        m.key(Key::Enter);
        assert_eq!(m.view, View::Workload);
        m.set_work(Source::Ready(super::super::workloads::Snapshot {
            sessions: vec![],
            queue: Err("signed out".into()),
        }));
        m.key(Key::Esc);
        let t = rows(&m).join("\n");
        assert!(
            t.contains("unavailable") && t.contains("none running"),
            "{t}"
        );
    }

    #[test]
    fn every_view_fits_its_width() {
        for view in [
            View::Rest,
            View::Menu,
            View::Approvals,
            View::Workloads,
            View::Workload,
            View::Run,
            View::Status,
            View::Auto,
            View::Help,
        ] {
            let mut m = model(3);
            m.view = view;
            m.note = Some("question".into());
            let w = m.width();
            let f = super::super::cells::lay_out(&m.spec(w), 0, 0);
            assert!(f.cells.iter().all(|r| r.len() == w), "{view:?}");
        }
        let mut long = model(1);
        long.set_asks(Source::Ready(vec![ask(
            "apr_0",
            &"very long title ".repeat(10),
        )]));
        long.view = View::Approvals;
        let f = super::super::cells::lay_out(&long.spec(50), 0, 0);
        assert!(f.cells.iter().all(|r| r.len() == 50));
        let text: String = f.cells[1].iter().map(|c| c.ch).collect();
        assert!(text.contains('…') && text.contains("4m  1"), "{text}");
    }
}
