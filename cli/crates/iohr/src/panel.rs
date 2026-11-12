//! The terminal panel's static design drawings (`--demo --variant rail|accordion|band`), on
//! stub data: nothing here talks to the platform and nothing is wired to a shell. The live
//! panel is in [`proxy`]; the stub tree these drawings show is the one in [`workloads`].
//!
//! The panel is a vertical menu at the top right of the terminal, in the one palette of
//! [`crate::card::Look`] (violet frame, cyan key chips, dim hints, a reversed selected row).
//! Everything is pure: [`lines`] gives the rows for a variant, a view and a state, and
//! [`draw`] wraps them in the sequences a shell hook would write (save the cursor, move,
//! paint, restore it) so the layout is tested without a terminal. Every value shown is a
//! stub and the frame says so (`stub`): a placeholder never looks like a measurement.

use std::fmt::Write as _;

use console::{Style, measure_text_width, truncate_str};

use crate::Env;
use crate::card::Look;
use crate::cli::{Global, PanelArgs, PanelAuto, PanelVariant, PanelView};
use crate::error::Error;
use crate::output::Out;

#[cfg(unix)]
mod act;
#[cfg(unix)]
mod approve;
mod workloads;
// The live panel is a pty proxy (rustix, unix terminals); off unix only the static designs run.
#[cfg(unix)]
mod cells;
#[cfg(unix)]
mod chat;
#[cfg(unix)]
mod input;
#[cfg(unix)]
mod model;
#[cfg(unix)]
mod proxy;
#[cfg(unix)]
mod screen;

/// The panel's width in columns, frame included.
pub(crate) const WIDTH: usize = 34;
/// What the demo says about its data, on every frame.
const STUB: &str = "stub";

/// A waiting approval, as the panel lists it.
struct Pending {
    title: &'static str,
    kind: &'static str,
    age: &'static str,
}

/// The stub approvals of the demo.
const PENDING: [Pending; 3] = [
    Pending {
        title: "Merge sdk #443 to main",
        kind: "pull request",
        age: "4m",
    },
    Pending {
        title: "Roll runner to 0.7.21",
        kind: "roll",
        age: "11m",
    },
    Pending {
        title: "Apply migration 20261106",
        kind: "migration",
        age: "1h",
    },
];

/// The subcommands the Run submenu offers: a key, the words, and what it does.
const RUN: [(char, &str, &str); 5] = [
    ('a', "iohr ask", "ask your agent"),
    ('s', "iohr asks", "the approvals queue"),
    ('m', "iohr memory", "what Io remembers"),
    ('c', "iohr coordinator", "plans and jobs"),
    ('i', "iohr init", "set this folder up"),
];

/// The top-level rows: the key, the label, and the view each opens (`None` acts at once).
const MENU: [(char, &str, Option<PanelView>); 7] = [
    ('a', "Auto mode", Some(PanelView::Auto)),
    ('p', "Approvals", Some(PanelView::Approvals)),
    ('w', "Workloads", Some(PanelView::Workloads)),
    ('r', "Run", Some(PanelView::Run)),
    ('s', "Status", Some(PanelView::Status)),
    ('?', "Help", None),
    ('x', "Stop panel", None),
];

/// One line of the panel before it is framed.
enum Row {
    /// A menu entry: key chip, label, the value at the right, and whether it is selected.
    Item(char, String, String, bool),
    /// The header of a submenu: its name, and what stands at the right.
    Back(String, String),
    /// A line of plain text (dim, or in the palette when `.1` is set).
    Text(String, Tone),
}

/// The colour a text row is in.
#[derive(Clone, Copy)]
enum Tone {
    Plain,
    Dim,
    Amber,
    Green,
}

/// The state the panel is drawn for.
struct Model {
    auto: PanelAuto,
    selected: usize,
}

impl Model {
    fn auto_value(&self) -> (String, Tone) {
        match self.auto {
            PanelAuto::Off => ("○ OFF".to_owned(), Tone::Dim),
            PanelAuto::Asking => ("◌ ASKING".to_owned(), Tone::Amber),
            PanelAuto::On => ("● ON".to_owned(), Tone::Green),
        }
    }

    fn auto_sentence(&self) -> &'static str {
        match self.auto {
            PanelAuto::Off => "off",
            PanelAuto::Asking => "waiting on your phone",
            PanelAuto::On => "on, 3h 12m left",
        }
    }
}

fn tone(look: Look, t: Tone) -> Style {
    match t {
        Tone::Plain => look.plain(),
        Tone::Dim => look.dim(),
        Tone::Amber => look.amber(),
        Tone::Green => look.green(),
    }
}

/// The top-level menu; in the nested variants the open entry carries its submenu under it.
fn menu_rows(m: &Model, open: Option<PanelView>) -> Vec<Row> {
    let mut out = Vec::new();
    for (i, (key, label, target)) in MENU.iter().enumerate() {
        let right = match key {
            'a' => m.auto_value().0,
            'p' => format!("{} ▸", PENDING.len()),
            'w' => format!("{} ▸", demo_running()),
            'r' | 's' => "▸".to_owned(),
            _ => String::new(),
        };
        let here = match open {
            Some(_) => false,
            None => i == m.selected,
        };
        out.push(Row::Item(*key, (*label).to_owned(), right, here));
        if target.is_some() && *target == open {
            out.extend(
                rows(open.unwrap_or(PanelView::Menu), m, false)
                    .into_iter()
                    .skip(1),
            );
        }
    }
    out
}

fn approvals_rows(m: &Model) -> Vec<Row> {
    let mut out = vec![Row::Back(
        "Approvals".to_owned(),
        format!("{} waiting", PENDING.len()),
    )];
    for (i, p) in PENDING.iter().enumerate() {
        let sel = i == m.selected.min(PENDING.len() - 1);
        let key = u8::try_from(i + 1).map_or('?', |n| char::from(b'0' + n));
        out.push(Row::Item(key, p.title.to_owned(), String::new(), sel));
        out.push(Row::Text(format!("    {} · {}", p.kind, p.age), Tone::Dim));
        if sel {
            out.push(Row::Text(
                "    d deny  q ask  o more".to_owned(),
                Tone::Plain,
            ));
            out.push(Row::Text(
                "    approve: on your phone".to_owned(),
                Tone::Dim,
            ));
        }
    }
    out
}

/// The demo's workload tree (the stub the floating panel shows), as plain rows.
fn demo_tree() -> Vec<workloads::Workload> {
    workloads::tree(
        &workloads::stub(),
        &[(
            "stub-0".to_owned(),
            "Merge sdk #443 to main".to_owned(),
            "Fix sdk #448 reads".to_owned(),
        )],
    )
}

/// How many sessions and jobs of the demo tree are running.
fn demo_running() -> usize {
    demo_tree()
        .iter()
        .filter(|w| matches!(w.role, workloads::Role::Session | workloads::Role::Job))
        .filter(|w| w.state.running())
        .count()
}

fn workloads_rows() -> Vec<Row> {
    let tree = demo_tree();
    let mut out = vec![Row::Back(
        "Workloads".to_owned(),
        format!("{} running", demo_running()),
    )];
    for (i, w) in tree.iter().enumerate() {
        let dot = match w.state {
            workloads::State::Needs => "◐",
            workloads::State::Idle => "○",
            workloads::State::Working | workloads::State::Stuck => "●",
        };
        let tone = match w.state {
            workloads::State::Needs => Tone::Amber,
            workloads::State::Working => Tone::Green,
            workloads::State::Idle | workloads::State::Stuck => Tone::Dim,
        };
        out.push(Row::Text(
            format!("{}{dot} {}", workloads::glyph(&tree, i), w.label),
            tone,
        ));
    }
    out
}

fn run_rows(m: &Model) -> Vec<Row> {
    let mut out = vec![Row::Back("Run".to_owned(), String::new())];
    for (i, (key, words, what)) in RUN.iter().enumerate() {
        out.push(Row::Item(
            *key,
            (*words).to_owned(),
            String::new(),
            i == m.selected,
        ));
        if i == m.selected {
            out.push(Row::Text(format!("    {what}"), Tone::Dim));
        }
    }
    out.push(Row::Text("⏎ puts it on your prompt".to_owned(), Tone::Dim));
    out
}

fn auto_rows(m: &Model) -> Vec<Row> {
    let mut out = vec![Row::Back("Auto mode".to_owned(), m.auto_value().0)];
    let lines: &[(&str, Tone)] = match m.auto {
        PanelAuto::Off => &[
            ("Auto mode is off.", Tone::Plain),
            ("Turning it on asks your phone", Tone::Dim),
            ("to approve, for up to 24 h.", Tone::Dim),
            ("Never-list kinds stay manual.", Tone::Dim),
            ("y ask my phone    n cancel", Tone::Plain),
        ],
        PanelAuto::Asking => &[
            ("Waiting on your phone.", Tone::Amber),
            ("Nothing is auto-approved", Tone::Dim),
            ("until you approve there.", Tone::Dim),
            ("c cancel the request", Tone::Plain),
        ],
        PanelAuto::On => &[
            ("Auto mode is on.", Tone::Green),
            ("3h 12m left, 41 approved.", Tone::Dim),
            ("Never-list kinds stay manual.", Tone::Dim),
            ("y turn off now    n keep", Tone::Plain),
        ],
    };
    out.extend(
        lines
            .iter()
            .map(|(t, tone)| Row::Text((*t).to_owned(), *tone)),
    );
    out
}

fn status_rows(m: &Model) -> Vec<Row> {
    let mut out = vec![Row::Back("Status".to_owned(), String::new())];
    for (k, v) in [
        ("profile", "demo"),
        ("agent", "claude"),
        ("runner", "idle"),
        ("approvals", "2 waiting"),
    ] {
        out.push(Row::Text(format!("{k:<10}{v}"), Tone::Plain));
    }
    out.push(Row::Text(
        format!("auto      {}", m.auto_sentence()),
        Tone::Plain,
    ));
    out
}

/// The rows of a view. `nested` is the accordion: the submenu opens under its menu entry.
fn rows(view: PanelView, m: &Model, nested: bool) -> Vec<Row> {
    if nested && !matches!(view, PanelView::Rest | PanelView::Menu) {
        return menu_rows(m, Some(view));
    }
    match view {
        PanelView::Rest => {
            let (value, t) = m.auto_value();
            vec![
                Row::Text(format!("auto  {value}"), t),
                Row::Text(format!("{} approvals waiting", PENDING.len()), Tone::Amber),
                Row::Text(format!("{}  open the menu", open_key_label()), Tone::Dim),
            ]
        }
        PanelView::Menu => menu_rows(m, None),
        PanelView::Approvals => approvals_rows(m),
        PanelView::Workloads => workloads_rows(),
        PanelView::Run => run_rows(m),
        PanelView::Auto => auto_rows(m),
        PanelView::Status => status_rows(m),
    }
}

fn fit(text: &str, width: usize) -> String {
    let t = truncate_str(text, width, "…");
    let pad = width.saturating_sub(measure_text_width(&t));
    format!("{t}{}", " ".repeat(pad))
}

/// The inner text of a row, `width` columns wide, with the look applied.
fn inner(look: Look, row: &Row, width: usize) -> String {
    match row {
        Row::Item(key, label, right, sel) => {
            let chip = format!("[{key}]");
            let room = width.saturating_sub(chip.len() + 4 + measure_text_width(right));
            let label = fit(label, room);
            if *sel {
                let plain = format!("› {chip} {label} {right}");
                let text = fit(&plain, width);
                Style::new()
                    .force_styling(look.fancy)
                    .reverse()
                    .apply_to(text)
                    .to_string()
            } else {
                let right_styled = if right.contains("ON") {
                    look.green().apply_to(right).to_string()
                } else if right.contains("ASKING") || right.contains(char::is_numeric) {
                    look.amber().apply_to(right).to_string()
                } else {
                    look.dim().apply_to(right).to_string()
                };
                format!("  {} {label} {right_styled}", look.cyan().apply_to(chip))
            }
        }
        Row::Back(name, right) => {
            let left = format!("‹ {name}");
            let gap = width.saturating_sub(measure_text_width(&left) + measure_text_width(right));
            format!(
                "{}{}{}",
                look.violet().bold().apply_to(left),
                " ".repeat(gap),
                look.dim().apply_to(right)
            )
        }
        Row::Text(text, t) => tone(look, *t).apply_to(fit(text, width)).to_string(),
    }
}

fn pad_to(s: &str, width: usize) -> String {
    let pad = width.saturating_sub(measure_text_width(s));
    format!("{s}{}", " ".repeat(pad))
}

/// The footer line: the keys of the view.
fn foot(view: PanelView) -> &'static str {
    match view {
        PanelView::Rest => "",
        PanelView::Menu => "↑↓ move   ⏎ open   esc close",
        PanelView::Approvals
        | PanelView::Workloads
        | PanelView::Run
        | PanelView::Auto
        | PanelView::Status => "↑↓ move   ⏎ choose   esc back",
    }
}

/// The rows of the panel for a variant and a view, each `WIDTH` columns wide (the band's, the
/// terminal's width), ready to be written one per terminal row at the top right.
fn lines(
    variant: PanelVariant,
    view: PanelView,
    m: &Model,
    look: Look,
    cols: usize,
) -> Vec<String> {
    let violet = look.violet();
    let dim = look.dim();
    let w = WIDTH;
    let nested = variant != PanelVariant::Rail;
    let body = rows(view, m, nested);
    let mut out = Vec::new();
    match variant {
        // The floating band is drawn live by the proxy, not as a static design.
        PanelVariant::Float => {}
        PanelVariant::Rail => {
            let tag = format!(" {STUB} ");
            let title = format!(
                "{}{}",
                violet.apply_to("╭─ "),
                violet.clone().bold().apply_to("iohr ")
            );
            let used = measure_text_width(&title) + measure_text_width(&tag) + 2;
            out.push(format!(
                "{title}{}{}{}",
                violet.apply_to("─".repeat(w.saturating_sub(used))),
                look.amber().apply_to(&tag),
                violet.apply_to("─╮"),
            ));
            let edge = violet.apply_to("│");
            for row in &body {
                out.push(format!(
                    "{edge} {} {edge}",
                    pad_to(&inner(look, row, w - 4), w - 4)
                ));
            }
            let f = foot(view);
            if !f.is_empty() {
                out.push(format!(
                    "{}{}{}",
                    violet.apply_to("├"),
                    violet.apply_to("─".repeat(w - 2)),
                    violet.apply_to("┤")
                ));
                out.push(format!("{edge} {} {edge}", dim.apply_to(fit(f, w - 4))));
            }
            out.push(format!(
                "{}{}{}",
                violet.apply_to("╰"),
                violet.apply_to("─".repeat(w - 2)),
                violet.apply_to("╯")
            ));
        }
        PanelVariant::Accordion => {
            let rule = violet.apply_to("┃");
            out.push(format!(
                "{rule} {}  {}",
                violet.bold().apply_to("iohr"),
                look.amber().apply_to(STUB)
            ));
            for row in &body {
                out.push(format!(
                    "{rule} {}",
                    pad_to(&inner(look, row, w - 2), w - 2)
                ));
            }
            let f = foot(view);
            if !f.is_empty() {
                out.push(format!("{rule} {}", dim.apply_to(fit(f, w - 2))));
            }
        }
        PanelVariant::Band => {
            // A strip across the top. Closed it is one line; open it holds the list.
            let (value, t) = m.auto_value();
            let auto = format!("auto {}", tone(look, t).apply_to(value));
            let strip = format!(
                "{} {} {}   {auto}   {}  {}",
                violet.bold().apply_to("iohr"),
                look.amber().apply_to(STUB),
                dim.apply_to("│"),
                look.amber().apply_to(format!("{} waiting", PENDING.len())),
                dim.apply_to(format!("{} menu", open_key_label())),
            );
            let right = |s: &str| {
                let gap = cols.saturating_sub(measure_text_width(s));
                format!("{}{s}", " ".repeat(gap))
            };
            out.push(right(&strip));
            if view != PanelView::Rest {
                for row in &body {
                    out.push(right(&format!(
                        "{} ",
                        pad_to(&inner(look, row, w - 2), w - 2)
                    )));
                }
                let f = foot(view);
                if !f.is_empty() {
                    out.push(right(&format!("{} ", dim.apply_to(pad_to(f, w - 2)))));
                }
            }
            out.push(dim.apply_to("─".repeat(cols)).to_string());
        }
    }
    out
}

/// What a hook writes to reserve the room: right-hand columns where the terminal has left
/// and right margins (DECLRMM, DECSLRM), the top rows under a scroll region (DECSTBM) for the
/// band. `release` gives all of it back, so `iohr stop` leaves the terminal as it was.
fn reserve(variant: PanelVariant, cols: usize, rows: usize, height: usize) -> String {
    match variant {
        PanelVariant::Rail | PanelVariant::Accordion => {
            format!("\x1b[?69h\x1b[1;{}s", cols.saturating_sub(WIDTH + 1))
        }
        PanelVariant::Band => format!("\x1b[{};{rows}r", height + 1),
        PanelVariant::Float => String::new(),
    }
}

fn release() -> &'static str {
    "\x1b[?69l\x1b[r"
}

/// The sequences that paint `rows`: save the cursor, write each row at its place, restore the
/// cursor, so the prompt the person is typing at is left where it was.
fn draw(variant: PanelVariant, rows: &[String], cols: usize) -> String {
    let col = match variant {
        PanelVariant::Rail | PanelVariant::Accordion => cols.saturating_sub(WIDTH) + 1,
        PanelVariant::Band => 1,
        PanelVariant::Float => return String::new(),
    };
    let mut out = String::from("\x1b7");
    for (i, row) in rows.iter().enumerate() {
        let _ = write!(out, "\x1b[{};{col}H{row}", i + 1);
    }
    out.push_str("\x1b8");
    out
}

/// `iohr panel`: the floating band over a shell (or the command after `--`); `--demo` runs it
/// on stub asks (`--variant float`) or draws one of the static designs.
pub(crate) async fn run(g: &Global, env: Env, args: PanelArgs) -> Result<(), Error> {
    #[cfg(unix)]
    if std::env::var_os(proxy::LEADER).is_some() {
        return proxy::session_leader(&args.command);
    }
    if args.demo && args.variant != PanelVariant::Float {
        demo(g, &args);
        return Ok(());
    }
    live(g, env, args).await
}

/// The floating panel over a shell, on a pseudo-terminal.
#[cfg(unix)]
async fn live(g: &Global, env: Env, args: PanelArgs) -> Result<(), Error> {
    let initial = args.view.map_or(model::View::Rest, |v| match v {
        PanelView::Rest => model::View::Rest,
        PanelView::Menu => model::View::Menu,
        PanelView::Approvals => model::View::Approvals,
        PanelView::Workloads => model::View::Workloads,
        PanelView::Run => model::View::Run,
        PanelView::Auto => model::View::Auto,
        PanelView::Status => model::View::Status,
    });
    proxy::run(
        g,
        env,
        proxy::Config {
            stub: args.demo,
            initial,
            command: args.command,
            host_gone: None,
            project: None,
        },
    )
    .await
    .map(|_| ())
}

/// Bare `iohr`: the person's shell under the panel, in the session the caller joined for
/// `project`. Answers `true` when the session host stopped while the shell was open (`iohr
/// stop`); running `iohr` again then joins a host again, from here, for as long as the panel is up.
#[cfg(unix)]
pub(crate) async fn session_shell(
    g: &Global,
    env: Env,
    host_gone: Option<tokio::sync::oneshot::Receiver<()>>,
    project: std::path::PathBuf,
) -> Result<bool, Error> {
    proxy::run(
        g,
        env,
        proxy::Config {
            stub: false,
            initial: model::View::Rest,
            command: Vec::new(),
            host_gone,
            project: Some(project),
        },
    )
    .await
}

/// Whether bare `iohr` enters the session: a terminal, colour allowed, no `--json`, and not
/// already inside a panel. Anything else keeps the usage report.
#[cfg(unix)]
pub(crate) fn bare_enters(g: &Global) -> bool {
    proxy::fancy(g) && !proxy::inside()
}

#[cfg(not(unix))]
pub(crate) fn bare_enters(_: &Global) -> bool {
    false
}

/// Whether the panel's chat box is on the screen and shows an answer's progress (RFC 0212): the
/// ask then leaves its own spinner out.
#[cfg(unix)]
pub(crate) fn chat_shown() -> bool {
    proxy::chat_shown()
}

#[cfg(not(unix))]
pub(crate) fn chat_shown() -> bool {
    false
}

/// One progress report to the chat box (`begin`, `step;2;-;reading 2 files`, `end`, `back;1`);
/// whether it was sent.
#[cfg(unix)]
pub(crate) fn chat_report(body: &str) -> bool {
    proxy::chat_report(body)
}

#[cfg(not(unix))]
pub(crate) fn chat_report(_: &str) -> bool {
    false
}

/// Says `line` on stderr where the panel will not cover it. The panel floats its box over the
/// top right corner of the screen (ADR 0225), and a fresh terminal prints this note in those very
/// rows, so the box would paint over the cause it is there to give. A terminal first scrolls the
/// corner away: the note starts below the box and its shadow. Off a terminal it is the plain note.
#[cfg(unix)]
pub(crate) fn note_below_the_box(line: &str) {
    use std::io::{IsTerminal as _, Write as _};
    let mut err = std::io::stderr().lock();
    if err.is_terminal() {
        let _ = write!(err, "{}", "\n".repeat(CORNER_ROWS));
    }
    let _ = writeln!(err, "{line}");
}

/// The rows the idle box and its shadow cover from the top of the screen: three for the box,
/// one for the shadow under it.
#[cfg(unix)]
const CORNER_ROWS: usize = 4;

/// The key the static designs name: the live panel's own choice for this terminal.
#[cfg(unix)]
fn open_key_label() -> String {
    input::OpenKey::here(false).label().to_owned()
}

#[cfg(not(unix))]
fn open_key_label() -> String {
    "⌥i".to_owned()
}

/// What bare `iohr` says when it runs inside a panel (the session is already here), else `None`.
#[cfg(unix)]
pub(crate) fn bare_inside_message(g: &Global) -> Option<String> {
    proxy::bare_inside_message(g)
}

#[cfg(not(unix))]
pub(crate) fn bare_inside_message(_: &Global) -> Option<String> {
    None
}

/// Closes the panel around this process when `iohr stop` runs inside one; whether it did.
#[cfg(unix)]
pub(crate) fn end_from_inside() -> bool {
    proxy::end_from_inside()
}

/// Off unix there is no pseudo-terminal to proxy: the live panel is not built, and the static
/// designs (`--demo --variant rail|accordion|band`) still draw. `async` only to match the unix
/// `live` the caller awaits.
#[cfg(not(unix))]
#[allow(clippy::unused_async)]
async fn live(_g: &Global, _env: Env, _args: PanelArgs) -> Result<(), Error> {
    Err(Error::Failed(
        "the floating panel needs a unix terminal; `--demo --variant rail|accordion|band` draws the static designs".into(),
    ))
}

/// The static designs, drawn once on stub data (`--demo`).
fn demo(g: &Global, args: &PanelArgs) {
    let fancy = crate::ui::fancy_for(
        true,
        g.json,
        std::env::var("NO_COLOR").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    );
    let look = Look {
        fancy,
        truecolor: matches!(
            std::env::var("COLORTERM").unwrap_or_default().as_str(),
            "truecolor" | "24bit"
        ),
    };
    let m = Model {
        auto: args.auto,
        selected: args.selected,
    };
    let (cols, term_rows) = (usize::from(args.cols), usize::from(args.rows));
    let view = args.view.unwrap_or(PanelView::Menu);
    let rows = lines(args.variant, view, &m, look, cols);
    let mut text = String::new();
    if args.lines {
        for row in &rows {
            let _ = writeln!(text, "{row}");
        }
    } else {
        if args.setup {
            text.push_str(&reserve(args.variant, cols, term_rows, rows.len()));
        }
        text.push_str(&draw(args.variant, &rows, cols));
        if args.setup {
            text.push_str(release());
        }
    }
    Out::raw(text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOK: Look = Look {
        fancy: false,
        truecolor: false,
    };

    fn model(auto: PanelAuto, selected: usize) -> Model {
        Model { auto, selected }
    }

    const VIEWS: [PanelView; 7] = [
        PanelView::Rest,
        PanelView::Menu,
        PanelView::Approvals,
        PanelView::Workloads,
        PanelView::Run,
        PanelView::Auto,
        PanelView::Status,
    ];

    #[test]
    fn every_column_row_is_the_panel_width_and_says_stub() {
        for variant in [PanelVariant::Rail, PanelVariant::Accordion] {
            for view in VIEWS {
                for auto in [PanelAuto::Off, PanelAuto::Asking, PanelAuto::On] {
                    let rows = lines(variant, view, &model(auto, 1), LOOK, 100);
                    assert!(rows[0].contains(STUB), "{variant:?} {view:?}");
                    for r in &rows {
                        assert!(
                            measure_text_width(r) <= WIDTH,
                            "{variant:?} {view:?} too wide: {r:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_rail_is_framed_to_the_same_width_on_every_row() {
        let rows = lines(
            PanelVariant::Rail,
            PanelView::Menu,
            &model(PanelAuto::On, 0),
            LOOK,
            100,
        );
        assert!(
            rows.iter().all(|r| measure_text_width(r) == WIDTH),
            "{rows:#?}"
        );
    }

    #[test]
    fn the_band_fills_the_terminal_and_closes_with_a_rule() {
        let rows = lines(
            PanelVariant::Band,
            PanelView::Menu,
            &model(PanelAuto::Off, 0),
            LOOK,
            90,
        );
        assert!(rows.iter().all(|r| measure_text_width(r) == 90));
        assert!(rows.last().is_some_and(|r| r.starts_with('─')));
    }

    #[test]
    fn auto_mode_shows_its_three_states() {
        let on = lines(
            PanelVariant::Rail,
            PanelView::Menu,
            &model(PanelAuto::On, 5),
            LOOK,
            100,
        )
        .join("\n");
        let off = lines(
            PanelVariant::Rail,
            PanelView::Menu,
            &model(PanelAuto::Off, 5),
            LOOK,
            100,
        )
        .join("\n");
        let asking = lines(
            PanelVariant::Rail,
            PanelView::Menu,
            &model(PanelAuto::Asking, 5),
            LOOK,
            100,
        )
        .join("\n");
        assert!(on.contains("● ON") && off.contains("○ OFF") && asking.contains("◌ ASKING"));
    }

    #[test]
    fn approvals_offer_deny_and_question_but_approve_stays_on_the_phone() {
        let text = lines(
            PanelVariant::Rail,
            PanelView::Approvals,
            &model(PanelAuto::On, 0),
            LOOK,
            100,
        )
        .join("\n");
        assert!(text.contains("d deny") && text.contains("q ask"));
        assert!(text.contains("approve: on your phone"));
    }

    #[test]
    fn the_accordion_opens_a_submenu_under_its_row() {
        let rows = lines(
            PanelVariant::Accordion,
            PanelView::Run,
            &model(PanelAuto::On, 2),
            LOOK,
            100,
        );
        let at = |needle: &str| rows.iter().position(|r| r.contains(needle));
        assert!(at("Run") < at("iohr ask") && at("iohr ask") < at("Status"));
    }

    #[test]
    fn drawing_saves_and_restores_the_cursor_and_stop_releases_the_room() {
        let rows = vec!["x".to_owned()];
        let d = draw(PanelVariant::Rail, &rows, 100);
        assert!(d.starts_with("\x1b7") && d.ends_with("\x1b8"));
        assert!(d.contains("\x1b[1;67Hx"));
        assert!(reserve(PanelVariant::Rail, 100, 30, 1).contains("\x1b[?69h"));
        assert!(reserve(PanelVariant::Band, 100, 30, 3).contains("\x1b[4;30r"));
        assert_eq!(release(), "\x1b[?69l\x1b[r");
    }

    #[test]
    fn a_plain_look_has_no_escape_codes() {
        for view in VIEWS {
            let rows = lines(
                PanelVariant::Rail,
                view,
                &model(PanelAuto::On, 0),
                LOOK,
                100,
            );
            assert!(rows.iter().all(|r| !r.contains('\x1b')));
        }
    }
}
