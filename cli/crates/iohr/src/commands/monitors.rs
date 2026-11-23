//! `iohr monitors`: the InOrbit monitors in `.inorbit/monitors` (RFC 0008), with the commands an
//! SRE reaches for. `check` reads the folder offline against the schema and the agent's rules;
//! `run` runs monitors from here (http and sse with `expect.shape` checked against the project's
//! own `schemas/`, each `cli` check against the project's binary in a throwaway folder removed
//! afterwards), once or `--watch`ing; `list`, `show` and `history` say what each monitor is, how
//! it is set here and how it ran; `disable` and `enable` switch one off or on on this machine
//! without editing a file; `delete` cuts one entry out of its file; `export` prints one
//! `checks.toml` the InOrbit agent takes today; `install` puts it into the agent's checks file.
//! Silences and the run log live in `.inorbit/state/monitors`, git-ignored (ADR 0009). The work
//! is in [`crate::monitors`]; this file wires the schemas in and prints.

#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::format_push_string,
    clippy::trivially_copy_pass_by_ref,
    clippy::many_single_char_names,
    clippy::items_after_statements,
    clippy::if_not_else,
    clippy::assert_is_empty,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::manual_let_else,
    clippy::uninlined_format_args,
    clippy::cast_precision_loss,
    reason = "ported whole from the Eye monitors crate, where these pedantic style lints were not on; its tests are the contract"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::cli::{MonitorsArgs, MonitorsCmd, MonitorsPick};
use crate::error::Error;
use crate::monitors::cli::Cli;
use crate::monitors::run::{Ran, Verdict, run_all};
use crate::monitors::shape::Shapes;
use crate::monitors::state::{self, Entry, Mode, Past, State};
use crate::monitors::{self, Monitor, Report, edit, explain};
use crate::output::Out;
use crate::schema::{self, Schemas};
use crate::ui::{Mark, Paint};

macro_rules! out_line {
    ($($a:tt)*) => { Out::raw(format!("{}\n", format!($($a)*)).as_bytes()) };
}
macro_rules! out_text {
    ($($a:tt)*) => { Out::raw(format!($($a)*).as_bytes()) };
}
macro_rules! out_note {
    ($($a:tt)*) => { Out::note(&format!($($a)*)) };
}

/// The colours of a report: the shared [`Paint`], with the few tones Eye's report used.
#[derive(Clone, Copy)]
struct Look(Paint);

impl Look {
    fn stdout(json: bool) -> Self {
        Self(Paint::stdout(json))
    }
    fn bold(&self, s: &str) -> String {
        self.0.name(s)
    }
    fn dim(&self, s: &str) -> String {
        self.0.dim(s)
    }
    fn bad(&self, s: &str) -> String {
        self.0.tone(Mark::Fail, s)
    }
    fn good(&self, s: &str) -> String {
        self.0.tone(Mark::Ok, s)
    }
    fn hot(&self, s: &str) -> String {
        self.0.tone(Mark::Warn, s)
    }
}

/// What a line of a report is: passed, failed, or not run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Line {
    Pass,
    Fail,
    Skip,
}

/// A report's first two lines: `Title  ✓ <pass>` or `Title  ✗ N failed`, then `summary` dim.
fn head(look: &Look, title: &str, failed: usize, pass: &str, summary: &str) -> String {
    let verdict = if failed == 0 {
        look.good(&format!("\u{2713} {pass}"))
    } else {
        look.bad(&format!("\u{2717} {failed} failed"))
    };
    format!("{}  {verdict}\n{}\n", look.bold(title), look.dim(summary))
}

/// `rows` as an aligned block under a blank line: `  ✓ name   detail`. Empty for no rows.
fn block(look: &Look, rows: &[(Line, String, String)]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let pad = rows
        .iter()
        .map(|(_, n, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::from("\n");
    for (mark, name, detail) in rows {
        let gap = " ".repeat(pad - name.chars().count());
        let line = match mark {
            Line::Pass => format!(
                "  {} {name}{gap}  {}",
                look.good("\u{2713}"),
                look.dim(detail)
            ),
            Line::Fail => format!(
                "  {} {}{gap}  {detail}",
                look.bad("\u{2717}"),
                look.bold(name)
            ),
            Line::Skip => look.dim(&format!("  - {name}{gap}  {detail}")),
        };
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// What every command works from.
struct Ctx {
    dir: PathBuf,
    files: Vec<monitors::File>,
    report: Report,
    state: State,
    now: u64,
    json: bool,
}

impl Ctx {
    fn find(&self, name: &str) -> Option<&Monitor> {
        self.report.monitors.iter().find(|m| m.name == name)
    }

    fn modes(&self) -> BTreeMap<String, Mode> {
        let o = self.state.overrides(self.now);
        self.report
            .monitors
            .iter()
            .map(|m| (m.name.clone(), State::mode(m, &o, self.now)))
            .collect()
    }
}

/// Runs the subcommand and returns the exit code: 0 when everything passed, 1 when something
/// failed, 2 for wrong usage.
pub(crate) fn run(a: &MonitorsArgs, out: Out) -> Result<(), Error> {
    let json = out.json;
    let dir = match a.dir.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|d| monitors::find(&d))
    }) {
        Some(d) => d,
        None => return usage("no .inorbit/monitors here or above; pass --dir"),
    };
    let files = match monitors::load(&dir) {
        Ok(f) => f,
        Err(e) => {
            return Err(from_monitors(e));
        }
    };
    let report = monitors::check_files(&files);
    let cmd = a.cmd.clone().unwrap_or(MonitorsCmd::Check);
    if !report.problems.is_empty() || matches!(cmd, MonitorsCmd::Check) {
        return check(&report, json);
    }
    let state = State::new(state::dir_for(&dir));
    let cx = Ctx {
        dir,
        files,
        report,
        state,
        now: state::now(),
        json,
    };
    match cmd {
        MonitorsCmd::Check => check(&cx.report, cx.json),
        MonitorsCmd::Export => match monitors::export(&cx.files) {
            Ok(text) => {
                out_text!("{text}");
                Ok(())
            }
            Err(e) => failed(&e),
        },
        MonitorsCmd::Install {
            file,
            block,
            dry_run,
            files,
        } => install_cmd(&cx, &files, file.as_deref(), block, dry_run),
        MonitorsCmd::Run {
            names,
            only,
            pick,
            watch,
            every,
        } => run_cmd(
            &cx,
            names.into_iter().chain(only).collect(),
            &pick,
            watch,
            every,
        ),
        MonitorsCmd::List { pick, failing, off } => list(&cx, &pick, failing, off),
        MonitorsCmd::Show { name, runs } => show(&cx, &name, runs),
        MonitorsCmd::History { name, last } => history(&cx, name.as_deref(), last),
        MonitorsCmd::Disable { name, for_, reason } => disable(&cx, &name, for_.as_deref(), reason),
        MonitorsCmd::Enable { name } => enable(&cx, &name),
        MonitorsCmd::Delete { name, yes } => delete(&cx, &name, yes),
    }
}

/// The monitors `names` and `pick` choose, in file order; wrong usage as the error.
fn choose<'a>(
    cx: &'a Ctx,
    names: &[String],
    pick: &MonitorsPick,
) -> Result<Vec<&'a Monitor>, String> {
    if let Some(unknown) = names.iter().find(|n| cx.find(n).is_none()) {
        return Err(format!("no monitor named {unknown}"));
    }
    let mut tags = Vec::new();
    for t in &pick.tag {
        tags.push(
            t.split_once('=')
                .ok_or_else(|| format!("--tag {t}: use key=value, such as app=web"))?,
        );
    }
    let chosen: Vec<&Monitor> = cx
        .report
        .monitors
        .iter()
        .filter(|m| names.is_empty() || names.contains(&m.name))
        .filter(|m| {
            pick.surface
                .as_deref()
                .is_none_or(|s| m.surface.as_str() == s)
        })
        .filter(|m| {
            pick.category
                .as_deref()
                .is_none_or(|c| m.category.as_deref() == Some(c))
        })
        .filter(|m| {
            tags.iter()
                .all(|(k, v)| m.tags.get(*k).map(String::as_str) == Some(*v))
        })
        .collect();
    if chosen.is_empty() && (pick.surface.is_some() || pick.category.is_some() || !tags.is_empty())
    {
        let what: Vec<String> = [
            pick.surface.as_ref().map(|s| format!("--surface {s}")),
            pick.category.as_ref().map(|c| format!("--category {c}")),
        ]
        .into_iter()
        .flatten()
        .chain(pick.tag.iter().map(|t| format!("--tag {t}")))
        .collect();
        return Err(format!("no monitor matches {}", what.join(" ")));
    }
    Ok(chosen)
}

/// How iohr runs monitors: the project's schemas for `expect.shape` and its own binary for cli checks.
struct Runner {
    schemas: Result<(Schemas, BTreeMap<String, String>), String>,
    cli: Cli,
}

impl Runner {
    fn new(dir: &Path) -> Runner {
        Runner {
            schemas: schemas_for(dir),
            cli: cli_for(dir),
        }
    }

    fn run(&self, ms: &[Monitor]) -> Result<Vec<Ran>, monitors::Error> {
        let by_path = |shape: &str| match &self.schemas {
            Ok((s, ids)) => {
                let (path, frag) = shape
                    .split_once('#')
                    .map_or((shape, None), |(p, f)| (p, Some(f)));
                let rel = path
                    .strip_prefix("schemas/")
                    .filter(|r| !r.is_empty() && !r.contains(".."));
                let id = rel
                    .and_then(|r| ids.get(r))
                    .ok_or_else(|| format!("{shape} is not a schema under schemas/"))?;
                let reference = frag.map_or_else(|| id.clone(), |f| format!("{id}#{f}"));
                s.validator_ref(&reference).map_err(|e| e.to_string())
            }
            Err(e) => Err(e.clone()),
        };
        let shapes: &dyn Shapes = &by_path;
        run_all(ms, &[], Some(shapes), Some(&self.cli))
    }
}

/// One monitor in a run: its state here and, when it ran, how.
struct Row<'a> {
    m: &'a Monitor,
    mode: Mode,
    ran: Option<Ran>,
}

impl Row<'_> {
    /// `pass`, `fail`, or `skip` for one not run.
    fn verdict(&self) -> &'static str {
        match self.ran.as_ref().map(|r| &r.verdict) {
            Some(Verdict::Pass) => "pass",
            Some(Verdict::Fail(_)) => "fail",
            _ => "skip",
        }
    }

    fn failed(&self) -> bool {
        self.verdict() == "fail"
    }

    /// The why: the runner's for a fail or a skip, else why it did not run here.
    fn why(&self) -> Option<String> {
        match self.ran.as_ref().map(|r| &r.verdict) {
            Some(Verdict::Fail(w) | Verdict::Skip(w)) => Some(w.clone()),
            Some(Verdict::Pass) => None,
            None => Some(off_why(self.m, &self.mode)),
        }
    }

    fn json(&self) -> Value {
        let why = self.why();
        let hint = if self.failed() {
            why.as_deref().and_then(|w| explain::hint(self.m, w))
        } else {
            None
        };
        let (until, reason) = match &self.mode {
            Mode::Silenced { reason, until } => (until.map(iso), reason.clone()),
            _ => (None, None),
        };
        json!({
            "name": self.m.name,
            "verdict": self.verdict(),
            "why": why,
            "ms": self.ran.as_ref().map_or(0, |r| r.ms),
            "status": self.ran.as_ref().and_then(|r| r.status),
            "label": self.m.label,
            "surface": self.m.surface.as_str(),
            "category": self.m.category,
            "target": explain::target(self.m),
            "expected": explain::expected(self.m),
            "hint": hint,
            "state": self.mode.as_str(),
            "until": until,
            "reason": reason,
        })
    }
}

/// Why a monitor switched off here was not run, and how to run it.
fn off_why(m: &Monitor, mode: &Mode) -> String {
    match mode {
        Mode::Off => format!(
            "off: enabled = false in {}; `iohr monitors enable {}` runs it here",
            m.file, m.name
        ),
        Mode::Silenced { reason, until } => {
            let mut s = String::from("silenced");
            if let Some(u) = until {
                s.push_str(&format!(" until {}", when(*u)));
            }
            if let Some(r) = reason {
                s.push_str(&format!(": {r}"));
            }
            s
        }
        Mode::On => "on".into(),
    }
}

/// Runs the `on` monitors among `chosen` once and logs them; the rest come back unrun.
fn round<'a>(
    cx: &Ctx,
    runner: &Runner,
    chosen: &[&'a Monitor],
    now: u64,
) -> Result<Vec<Row<'a>>, monitors::Error> {
    let o = cx.state.overrides(now);
    let mut rows: Vec<Row<'a>> = chosen
        .iter()
        .map(|m| Row {
            m,
            mode: State::mode(m, &o, now),
            ran: None,
        })
        .collect();
    let on: Vec<Monitor> = rows
        .iter()
        .filter(|r| r.mode == Mode::On)
        .map(|r| r.m.clone())
        .collect();
    let ran = runner.run(&on)?;
    if let Err(e) = cx.state.record(&ran, now) {
        out_note!(
            "the run log in {} was not written: {e}",
            cx.state.dir.display()
        );
    }
    let mut by_name: BTreeMap<String, Ran> = ran.into_iter().map(|r| (r.name.clone(), r)).collect();
    for r in &mut rows {
        r.ran = by_name.remove(&r.m.name);
    }
    Ok(rows)
}

fn run_cmd(
    cx: &Ctx,
    names: Vec<String>,
    pick: &MonitorsPick,
    watch: bool,
    every: Option<String>,
) -> Result<(), Error> {
    let chosen = match choose(cx, &names, pick) {
        Ok(c) => c,
        Err(e) => return usage(&e),
    };
    let gap = match every.as_deref().map(monitors::secs).transpose() {
        Ok(Some(0)) | Err(_) => {
            return usage(&format!(
                "--every {}: use a number with s, m, h or d, such as 30s or 5m",
                every.unwrap_or_default()
            ));
        }
        Ok(gap) => (watch || gap.is_some()).then(|| gap.unwrap_or(60)),
    };
    let runner = Runner::new(&cx.dir);
    let rows = match round(cx, &runner, &chosen, cx.now) {
        Ok(r) => r,
        Err(e) => {
            return Err(from_monitors(e));
        }
    };
    let failed = rows.iter().any(Row::failed);
    if cx.json {
        out_line!(
            "{}",
            json!({ "ok": !failed, "ran": rows.iter().map(Row::json).collect::<Vec<_>>() })
        );
    } else {
        out_text!("{}", view(&rows, &Look::stdout(cx.json)));
    }
    let Some(gap) = gap else {
        return if failed {
            Err(Error::Findings("a monitor failed".into()))
        } else {
            Ok(())
        };
    };
    // --watch: the first report above in full, then only the monitors whose verdict or state
    // changed, until Ctrl-C.
    let look = Look::stdout(cx.json);
    let key = |r: &Row| (r.verdict(), r.mode.as_str(), r.why());
    let mut last: BTreeMap<String, (&'static str, &'static str, Option<String>)> =
        rows.iter().map(|r| (r.m.name.clone(), key(r))).collect();
    if !cx.json {
        out_line!(
            "{}",
            look.dim(&format!(
                "\nwatching {} monitors every {}; Ctrl-C stops",
                chosen.len(),
                human(gap)
            ))
        );
    }
    loop {
        std::thread::sleep(Duration::from_secs(gap));
        let now = state::now();
        let rows = match round(cx, &runner, &chosen, now) {
            Ok(r) => r,
            Err(e) => {
                return Err(from_monitors(e));
            }
        };
        let changed: Vec<&Row> = rows
            .iter()
            .filter(|r| {
                last.get(&r.m.name)
                    .is_none_or(|k| k.0 != r.verdict() || k.1 != r.mode.as_str())
            })
            .collect();
        if cx.json && !changed.is_empty() {
            out_line!(
                "{}",
                json!({ "ok": !rows.iter().any(Row::failed), "ran": changed.iter().map(|r| r.json()).collect::<Vec<_>>() })
            );
        } else if !changed.is_empty() {
            for r in &changed {
                let was = last
                    .get(&r.m.name)
                    .map_or("new", |k| if k.1 == "on" { k.0 } else { k.1 });
                let now_is = if r.mode == Mode::On {
                    r.verdict()
                } else {
                    r.mode.as_str()
                };
                let line = format!("{}  {} {was} -> {now_is}", clock(now), r.m.name);
                let line = match r.verdict() {
                    "fail" => format!("{}  {}", look.bad(&line), r.why().unwrap_or_default()),
                    "pass" => look.good(&line),
                    _ => format!("{}  {}", look.dim(&line), r.why().unwrap_or_default()),
                };
                out_line!("{line}");
                if let Some(h) = r
                    .why()
                    .filter(|_| r.failed())
                    .and_then(|w| explain::hint(r.m, &w))
                {
                    out_line!("{}", look.dim(&format!("      hint   {h}")));
                }
            }
        }
        last = rows.iter().map(|r| (r.m.name.clone(), key(r))).collect();
    }
}

/// The text report: the verdict and counts; each failure with its label, what it sent, what it
/// wanted, what came back and a hint; then one line per pass grouped by surface; then the
/// monitors not run, with why.
fn view(rows: &[Row], look: &Look) -> String {
    let fails: Vec<&Row> = rows.iter().filter(|r| r.failed()).collect();
    let mut oks: Vec<&Row> = rows.iter().filter(|r| r.verdict() == "pass").collect();
    oks.sort_by_key(|r| r.m.surface.as_str());
    let skips: Vec<&Row> = rows.iter().filter(|r| r.verdict() == "skip").collect();
    let ms: u64 = rows
        .iter()
        .filter_map(|r| r.ran.as_ref())
        .map(|r| r.ms)
        .sum();
    let mut summary = format!("{} passed, {} failed", oks.len(), fails.len());
    for (n, what) in [
        (skips.iter().filter(|r| r.mode == Mode::Off).count(), "off"),
        (
            skips
                .iter()
                .filter(|r| matches!(r.mode, Mode::Silenced { .. }))
                .count(),
            "silenced",
        ),
        (
            skips.iter().filter(|r| r.mode == Mode::On).count(),
            "skipped",
        ),
    ] {
        if n > 0 {
            summary.push_str(&format!(", {n} {what}"));
        }
    }
    summary.push_str(&format!(", {:.1} s", ms as f64 / 1000.0));
    let mut out = head(look, "Monitors", fails.len(), "all pass", &summary);
    for r in &fails {
        out.push_str(&block(
            look,
            &[(
                Line::Fail,
                r.m.name.clone(),
                r.m.label.clone().unwrap_or_default(),
            )],
        ));
        let why = r.why().unwrap_or_default();
        let mut lines = vec![
            (r.m.surface.as_str(), explain::target(r.m)),
            ("wanted", explain::expected(r.m)),
            ("got", why.clone()),
        ];
        if let Some(h) = explain::hint(r.m, &why) {
            lines.push(("hint", h));
        }
        for (k, v) in lines {
            out.push_str(&format!("      {} {v}\n", look.dim(&format!("{k:<6}"))));
        }
    }
    let pad = oks
        .iter()
        .map(|r| r.m.surface.as_str().len())
        .max()
        .unwrap_or(0);
    let ok_rows: Vec<(Line, String, String)> = oks
        .iter()
        .map(|r| {
            let ms = r.ran.as_ref().map_or(0, |x| x.ms);
            (
                Line::Pass,
                r.m.name.clone(),
                format!(
                    "{ms:>5} ms  {:<pad$}  {}",
                    r.m.surface.as_str(),
                    r.m.label.as_deref().unwrap_or("")
                ),
            )
        })
        .collect();
    out.push_str(&block(look, &ok_rows));
    let skip_rows: Vec<(Line, String, String)> = skips
        .iter()
        .map(|r| (Line::Skip, r.m.name.clone(), r.why().unwrap_or_default()))
        .collect();
    out.push_str(&block(look, &skip_rows));
    out
}

/// A monitor's state here, in words: `on`, `off`, or `silenced until <time> (<reason>)`.
fn state_text(mode: &Mode) -> String {
    match mode {
        Mode::On => "on".into(),
        Mode::Off => "off".into(),
        Mode::Silenced { reason, until } => {
            let mut s = String::from("silenced");
            if let Some(u) = until {
                s.push_str(&format!(" until {}", when(*u)));
            }
            if let Some(r) = reason {
                s.push_str(&format!(" ({r})"));
            }
            s
        }
    }
}

/// A logged run as JSON, its time as an ISO date.
fn entry_json(e: &Entry) -> Value {
    json!({ "at": iso(e.at), "verdict": e.verdict, "why": e.why, "ms": e.ms, "status": e.status })
}

/// One monitor's line in `list` and `show`, as JSON.
fn monitor_json(m: &Monitor, mode: &Mode, past: Option<&Past>) -> Value {
    let (until, reason) = match mode {
        Mode::Silenced { reason, until } => (until.map(iso), reason.clone()),
        _ => (None, None),
    };
    json!({
        "name": m.name,
        "label": m.label,
        "surface": m.surface.as_str(),
        "category": m.category,
        "tags": m.tags,
        "target": explain::target(m),
        "every": every_text(m),
        "fail_after": m.fail_after,
        "state": mode.as_str(),
        "until": until,
        "reason": reason,
        "last": past.and_then(Past::last).map(entry_json),
    })
}

/// The last result in a few words: `pass 3m ago`, `FAIL 12s ago`, or `never run`.
fn last_text(now: u64, past: Option<&Past>) -> String {
    match past.and_then(Past::last) {
        Some(e) => format!(
            "{} {}",
            if e.verdict == "fail" {
                "FAIL"
            } else {
                e.verdict.as_str()
            },
            ago(now, e.at)
        ),
        None => "never run".into(),
    }
}

fn list(cx: &Ctx, pick: &MonitorsPick, failing: bool, off: bool) -> Result<(), Error> {
    let chosen = match choose(cx, &[], pick) {
        Ok(c) => c,
        Err(e) => return usage(&e),
    };
    let modes = cx.modes();
    let pasts = cx.state.pasts();
    let shown: Vec<&Monitor> = chosen
        .into_iter()
        .filter(|m| {
            !failing
                || pasts
                    .get(&m.name)
                    .and_then(Past::last)
                    .is_some_and(|e| e.verdict == "fail")
        })
        .filter(|m| !off || modes.get(&m.name).is_some_and(|s| *s != Mode::On))
        .collect();
    if cx.json {
        let rows: Vec<Value> = shown
            .iter()
            .map(|m| monitor_json(m, &modes[&m.name], pasts.get(&m.name)))
            .collect();
        out_line!("{}", json!({ "ok": true, "monitors": rows }));
        return Ok(());
    }
    let look = Look::stdout(cx.json);
    let failing_n = shown
        .iter()
        .filter(|m| {
            pasts
                .get(&m.name)
                .and_then(Past::last)
                .is_some_and(|e| e.verdict == "fail")
        })
        .count();
    let off_n = shown.iter().filter(|m| modes[&m.name] != Mode::On).count();
    out_line!(
        "{}  {}",
        look.bold("Monitors"),
        look.dim(&format!(
            "{} shown, {failing_n} failing last run, {off_n} off or silenced",
            shown.len()
        ))
    );
    let head = [
        "NAME", "STATE", "LAST", "SURFACE", "CATEGORY", "EVERY", "AFTER", "LABEL", "TARGET",
    ]
    .map(String::from)
    .to_vec();
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|m| {
            vec![
                m.name.clone(),
                state_text(&modes[&m.name]),
                last_text(cx.now, pasts.get(&m.name)),
                m.surface.as_str().into(),
                m.category.clone().unwrap_or_else(|| "-".into()),
                every_text(m),
                m.fail_after.to_string(),
                m.label.clone().unwrap_or_else(|| "-".into()),
                explain::target(m),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..head.len())
        .map(|i| {
            std::iter::once(&head)
                .chain(&rows)
                .map(|r| r[i].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |r: &[String]| {
        r.iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    out_line!("\n{}", look.dim(&line(&head)));
    for r in &rows {
        let l = line(r);
        out_line!(
            "{}",
            if r[2].starts_with("FAIL") {
                look.bad(&l)
            } else if r[1] != "on" {
                look.dim(&l)
            } else {
                l
            }
        );
    }
    Ok(())
}

/// `m`'s entry in its file as JSON, as written.
fn definition(cx: &Ctx, m: &Monitor) -> Value {
    let kind = if m.refuse { "refuse" } else { "check" };
    cx.files
        .iter()
        .filter(|f| f.name() == m.file)
        .filter_map(|f| f.value.get(kind).and_then(Value::as_array))
        .flatten()
        .find(|e| e.get("name").and_then(Value::as_str) == Some(&m.name))
        .cloned()
        .unwrap_or(Value::Null)
}

fn show(cx: &Ctx, name: &str, n: usize) -> Result<(), Error> {
    let Some(m) = cx.find(name) else {
        return usage(&format!("no monitor named {name}"));
    };
    let path = cx.dir.join(&m.file);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let span = edit::locate(&text, name);
    let source = span.as_ref().map(|s| {
        text.lines()
            .skip(s.start)
            .take(s.end - s.start)
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    });
    let line = span.as_ref().map(|s| s.header + 1);
    let mode = cx.modes().remove(name).unwrap_or(Mode::On);
    let past = cx.state.pasts().remove(name).unwrap_or_default();
    let recent: Vec<&Entry> = past.runs.iter().rev().take(n).collect();
    if cx.json {
        let mut v = monitor_json(m, &mode, Some(&past));
        let Some(o) = v.as_object_mut() else {
            return failed("monitor_json gave no object");
        };
        o.insert("file".into(), json!(m.file));
        o.insert("line".into(), json!(line));
        o.insert("checks".into(), json!(explain::checks(m)));
        o.insert("expected".into(), json!(explain::expected(m)));
        o.insert("enabled".into(), json!(m.enabled));
        o.insert("definition".into(), definition(cx, m));
        let history = json!({
            "runs": past.runs.len(),
            "flaps": past.flaps,
            "last_pass": past.last_pass.map(iso),
            "last_fail": past.last_fail.map(iso),
            "recent": recent.iter().map(|e| entry_json(e)).collect::<Vec<_>>(),
        });
        out_line!(
            "{}",
            json!({ "ok": true, "monitor": v, "history": history })
        );
        return Ok(());
    }
    let look = Look::stdout(cx.json);
    let mut out = format!(
        "{}  {}\n\n",
        look.bold(&m.name),
        m.label.as_deref().unwrap_or("")
    );
    let tags = m
        .tags
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    let at = |when: Option<u64>| {
        when.map_or_else(
            || "never".into(),
            |t| format!("{} ({})", self::when(t), ago(cx.now, t)),
        )
    };
    let mut facts = vec![
        ("checks", explain::checks(m)),
        (
            "file",
            format!(
                "{}{}",
                path.display(),
                line.map(|l| format!(":{l}")).unwrap_or_default()
            ),
        ),
        (
            "surface",
            format!(
                "{}{}",
                m.surface.as_str(),
                m.category
                    .as_ref()
                    .map(|c| format!(", category {c}"))
                    .unwrap_or_default()
            ),
        ),
        (
            "every",
            format!(
                "{}, pages after {} fail{} in a row",
                every_text(m),
                m.fail_after,
                if m.fail_after == 1 { "" } else { "s" }
            ),
        ),
        (
            "state",
            format!(
                "{}{}",
                state_text(&mode),
                if m.enabled {
                    ""
                } else {
                    " (enabled = false in its file)"
                }
            ),
        ),
        ("last", last_text(cx.now, Some(&past))),
        (
            "history",
            format!(
                "{} runs logged, {} flaps, last pass {}, last fail {}",
                past.runs.len(),
                past.flaps,
                at(past.last_pass),
                at(past.last_fail)
            ),
        ),
    ];
    if !tags.is_empty() {
        facts.insert(3, ("tags", tags));
    }
    for (k, v) in facts {
        out.push_str(&format!("  {} {v}\n", look.dim(&format!("{k:<8}"))));
    }
    if let Some(src) = source {
        out.push_str(&format!("\n  {}\n", look.dim("definition")));
        for l in src.lines() {
            out.push_str(&format!("    {l}\n"));
        }
    }
    out.push_str(&format!(
        "\n  {}\n",
        look.dim(&format!("last {}, newest first", runs_n(recent.len())))
    ));
    out.push_str(&runs_text(&recent, &look));
    out_text!("{out}");
    Ok(())
}

/// Logged runs, one line each: when, the verdict, the time and the why.
fn runs_text(runs: &[&Entry], look: &Look) -> String {
    if runs.is_empty() {
        return format!(
            "    {}\n",
            look.dim("none yet; `iohr monitors run` logs each run")
        );
    }
    runs.iter()
        .map(|e| {
            let head = format!("    {}  {:<4} {:>6} ms", when(e.at), e.verdict, e.ms);
            match e.verdict.as_str() {
                "fail" => format!("{}  {}\n", look.bad(&head), e.why.as_deref().unwrap_or("")),
                "pass" => format!("{head}\n"),
                _ => format!(
                    "{}\n",
                    look.dim(&format!("{head}  {}", e.why.as_deref().unwrap_or("")))
                ),
            }
        })
        .collect()
}

fn history(cx: &Ctx, name: Option<&str>, n: usize) -> Result<(), Error> {
    let pasts = cx.state.pasts();
    if let Some(name) = name
        && cx.find(name).is_none()
        && !pasts.contains_key(name)
    {
        return usage(&format!("no monitor named {name}"));
    }
    let shown: Vec<(&String, &Past)> = pasts
        .iter()
        .filter(|(k, _)| name.is_none_or(|n| n == k.as_str()))
        .collect();
    let count = |p: &Past, v: &str| p.runs.iter().filter(|e| e.verdict == v).count();
    let log = cx.state.dir.join("runs.jsonl");
    if cx.json {
        let rows: Vec<Value> = shown
            .iter()
            .map(|(k, p)| {
                json!({
                    "name": k,
                    "runs": p.runs.len(),
                    "passed": count(p, "pass"),
                    "failed": count(p, "fail"),
                    "flaps": p.flaps,
                    "last_pass": p.last_pass.map(iso),
                    "last_fail": p.last_fail.map(iso),
                    "recent": p.runs.iter().rev().take(if name.is_some() { n } else { 1 }).map(entry_json).collect::<Vec<_>>(),
                })
            })
            .collect();
        out_line!(
            "{}",
            json!({ "ok": true, "log": log.display().to_string(), "monitors": rows })
        );
        return Ok(());
    }
    let look = Look::stdout(cx.json);
    let total: usize = shown.iter().map(|(_, p)| p.runs.len()).sum();
    out_line!(
        "{}  {}",
        look.bold("Monitor history"),
        look.dim(&format!("{} in {}", runs_n(total), log.display()))
    );
    if let Some((k, p)) = shown.first().filter(|_| name.is_some()) {
        let recent: Vec<&Entry> = p.runs.iter().rev().take(n).collect();
        out_line!(
            "\n  {}  {}, {} passed, {} failed, {} flaps",
            look.bold(k),
            runs_n(p.runs.len()),
            count(p, "pass"),
            count(p, "fail"),
            p.flaps
        );
        out_text!("{}", runs_text(&recent, &look));
        return Ok(());
    }
    if shown.is_empty() {
        out_line!(
            "\n  {}",
            look.dim("no runs logged yet; `iohr monitors run` logs each run")
        );
        return Ok(());
    }
    let at = |t: Option<u64>| t.map_or_else(|| "never".into(), |t| ago(cx.now, t));
    let head = [
        "NAME",
        "RUNS",
        "PASS",
        "FAIL",
        "FLAPS",
        "LAST PASS",
        "LAST FAIL",
        "LAST",
    ]
    .map(String::from)
    .to_vec();
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|(k, p)| {
            vec![
                (*k).clone(),
                p.runs.len().to_string(),
                count(p, "pass").to_string(),
                count(p, "fail").to_string(),
                p.flaps.to_string(),
                at(p.last_pass),
                at(p.last_fail),
                p.last().map(|e| e.verdict.clone()).unwrap_or_default(),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..head.len())
        .map(|i| {
            std::iter::once(&head)
                .chain(&rows)
                .map(|r| r[i].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |r: &[String]| {
        r.iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    out_line!("\n{}", look.dim(&line(&head)));
    for r in &rows {
        let l = line(r);
        out_line!(
            "{}",
            if r[7] == "fail" {
                look.bad(&l)
            } else if r[4] != "0" {
                look.hot(&l)
            } else {
                l
            }
        );
    }
    Ok(())
}

fn disable(cx: &Ctx, name: &str, for_: Option<&str>, reason: Option<String>) -> Result<(), Error> {
    let Some(m) = cx.find(name) else {
        return usage(&format!("no monitor named {name}"));
    };
    let until = match for_.map(monitors::secs).transpose() {
        Ok(Some(0)) | Err(_) => {
            return usage(&format!(
                "--for {}: use a number with s, m, h or d, such as 30m or 2h",
                for_.unwrap_or_default()
            ));
        }
        Ok(s) => s.map(|s| cx.now + s),
    };
    // A monitor its file has off goes back to that with a bare `disable`; a silence needs --for
    // or --reason.
    if !m.enabled && until.is_none() && reason.is_none() {
        if let Err(e) = cx.state.forget(name) {
            return failed(&format!("{}: {e}", cx.state.dir.display()));
        }
        return said(
            cx,
            m,
            &Mode::Off,
            &format!(
                "{name} is off again, as {} says (enabled = false); it is not run and never fails a run.",
                m.file
            ),
        );
    }
    if let Err(e) = cx.state.disable(m, until, reason.clone()) {
        return failed(&format!("{}: {e}", cx.state.dir.display()));
    }
    let mode = Mode::Silenced { reason, until };
    said(
        cx,
        m,
        &mode,
        &format!(
            "{name} is {}; it is not run and never fails a run. `iohr monitors enable {name}` ends it.",
            state_text(&mode)
        ),
    )
}

fn enable(cx: &Ctx, name: &str) -> Result<(), Error> {
    let Some(m) = cx.find(name) else {
        return usage(&format!("no monitor named {name}"));
    };
    if let Err(e) = cx.state.enable(m) {
        return failed(&format!("{}: {e}", cx.state.dir.display()));
    }
    let text = if m.enabled {
        format!("{name} is on again; the next `iohr monitors run` runs it.")
    } else {
        format!(
            "{name} now runs on this Mac; the next `iohr monitors run` runs it. Its file keeps enabled = false, so export still leaves it out; a bare `disable` puts it back."
        )
    };
    said(cx, m, &Mode::On, &text)
}

/// What `disable` or `enable` did: the line in text, the state in JSON.
fn said(cx: &Ctx, m: &Monitor, mode: &Mode, line: &str) -> Result<(), Error> {
    if cx.json {
        let (until, reason) = match mode {
            Mode::Silenced { reason, until } => (until.map(iso), reason.clone()),
            _ => (None, None),
        };
        out_line!(
            "{}",
            json!({ "ok": true, "name": m.name, "state": mode.as_str(), "until": until, "reason": reason })
        );
    } else {
        out_line!("{line}");
    }
    Ok(())
}

fn delete(cx: &Ctx, name: &str, yes: bool) -> Result<(), Error> {
    let Some(m) = cx.find(name) else {
        return usage(&format!("no monitor named {name}"));
    };
    let path = cx.dir.join(&m.file);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            return failed(&format!("{}: {e}", path.display()));
        }
    };
    let span = edit::locate(&text, name);
    let (out, removed) = match edit::cut(&text, name) {
        Ok(c) => c,
        Err(e) => {
            return failed(&format!("{}: {e}", path.display()));
        }
    };
    let lines = span.map(|s| (s.start + 1, s.end));
    if yes {
        let tmp = path.with_extension(format!("toml.{}", std::process::id()));
        if let Err(e) = std::fs::write(&tmp, &out).and_then(|()| std::fs::rename(&tmp, &path)) {
            let _ = std::fs::remove_file(&tmp);
            return failed(&format!("{}: {e}", path.display()));
        }
        let _ = cx.state.forget(name);
    }
    if cx.json {
        out_line!(
            "{}",
            json!({ "ok": true, "name": name, "file": path.display().to_string(), "lines": lines.map(|(a, b)| [a, b]), "removed": removed, "done": yes })
        );
        return Ok(());
    }
    let look = Look::stdout(cx.json);
    let at = lines
        .map(|(a, b)| format!(" lines {a}-{b}"))
        .unwrap_or_default();
    if yes {
        out_line!(
            "removed {name} from {}{at}; the other entries and comments are as they were",
            path.display()
        );
    } else {
        out_line!("would remove {name} from {}{at}:\n", path.display());
        for l in removed.lines() {
            out_line!("  {}", look.dim(l));
        }
        out_line!("\nnothing changed; `iohr monitors delete {name} --yes` removes it");
    }
    Ok(())
}

fn check(r: &Report, as_json: bool) -> Result<(), Error> {
    let bad = r.problems.len();
    if as_json {
        let names: Vec<&str> = r.monitors.iter().map(|m| m.name.as_str()).collect();
        out_line!(
            "{}",
            json!({ "ok": bad == 0, "monitors": names, "problems": r.problems, "warnings": r.warnings })
        );
    } else {
        for p in &r.problems {
            out_line!("bad  {p}");
        }
        for w in &r.warnings {
            out_line!("warn {w}");
        }
        if bad == 0 {
            out_line!("{} monitors, every file valid", r.monitors.len());
        }
    }
    if bad > 0 {
        return Err(Error::Findings(format!(
            "{bad} problem{} in the monitor files",
            if bad == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

/// `2026-11-14 17:20 UTC`.
fn when(at: u64) -> String {
    chrono::DateTime::from_timestamp(i64::try_from(at).unwrap_or(0), 0).map_or_else(
        || at.to_string(),
        |t| t.format("%Y-%m-%d %H:%M UTC").to_string(),
    )
}

/// `17:20:05 UTC`, for a watch line.
fn clock(at: u64) -> String {
    chrono::DateTime::from_timestamp(i64::try_from(at).unwrap_or(0), 0)
        .map_or_else(|| at.to_string(), |t| t.format("%H:%M:%S UTC").to_string())
}

/// `2026-11-14T17:20:05Z`, for JSON.
fn iso(at: u64) -> String {
    chrono::DateTime::from_timestamp(i64::try_from(at).unwrap_or(0), 0).map_or_else(
        || at.to_string(),
        |t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
}

/// `12s ago`, `5m ago`, `3h ago`, `2d ago`.
fn ago(now: u64, at: u64) -> String {
    format!("{} ago", human(now.saturating_sub(at)))
}

/// Seconds as the largest whole unit: `45s`, `5m`, `2h`, `1d`; `90s` stays `90s`.
fn runs_n(n: usize) -> String {
    format!("{n} run{}", if n == 1 { "" } else { "s" })
}

/// How often `m` runs: a cli monitor (every 0) runs after each build, on `iohr monitors run`.
fn every_text(m: &Monitor) -> String {
    if m.every_secs == 0 {
        "build".into()
    } else {
        human(m.every_secs)
    }
}

fn human(s: u64) -> String {
    match s {
        0..60 => format!("{s}s"),
        _ if s.is_multiple_of(86_400) => format!("{}d", s / 86_400),
        _ if s.is_multiple_of(3_600) => format!("{}h", s / 3_600),
        _ if s.is_multiple_of(60) => format!("{}m", s / 60),
        _ if s < 3_600 => format!("{}m", s / 60),
        _ if s < 86_400 => format!("{}h", s / 3_600),
        _ => format!("{}d", s / 86_400),
    }
}

fn usage(why: &str) -> Result<(), Error> {
    Err(Error::Usage(why.to_owned()))
}

fn failed(why: &str) -> Result<(), Error> {
    Err(Error::Failed(why.to_owned()))
}

fn from_monitors(e: monitors::Error) -> Error {
    match e {
        monitors::Error::Usage(m) => Error::Usage(m),
        other => Error::Failed(other.to_string()),
    }
}

/// The project's schemas for `expect.shape`: its `schemas/` folder over the embedded ones.
fn schemas_for(dir: &Path) -> Result<(Schemas, BTreeMap<String, String>), String> {
    let root = dir
        .ancestors()
        .nth(2)
        .map(|r| r.join("schemas"))
        .filter(|s| s.is_dir());
    match root {
        Some(s) => schema::load_tree(&s),
        None => Ok((Schemas::embedded()?, BTreeMap::new())),
    }
}

/// How `iohr monitors run` runs a project's own cli checks: this binary, throwaway folders under
/// the system temp folder (this binary unless `IOHR_MONITORS_BIN` names the project's own), and copies of the monitors folder (`{monitors}`) and, when the
/// project has one, of its scrum board (`{board}`), so no check writes to the project.
fn cli_for(dir: &Path) -> Cli {
    let bin = std::env::var_os("IOHR_MONITORS_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("iohr"));
    let root = std::env::var_os("IOHR_MONITORS_TMP").map_or_else(
        || std::env::temp_dir().join("iohr-monitors-cli"),
        PathBuf::from,
    );
    let mut copies = vec![("monitors".to_string(), dir.to_path_buf())];
    if let Some(board) = dir
        .ancestors()
        .nth(2)
        .map(|r| r.join("atlas/scrum"))
        .filter(|b| b.join("board.json").is_file())
    {
        copies.push(("board".to_string(), board));
    }
    Cli { bin, root, copies }
}

/// `install`: the project's monitors as one block of the agent's checks file.
fn install_cmd(
    cx: &Ctx,
    only: &[String],
    file: Option<&Path>,
    block: Option<String>,
    dry_run: bool,
) -> Result<(), Error> {
    use crate::monitors::install as inst;
    let chosen = match inst::choose(&cx.files, only) {
        Ok(c) => c,
        Err(e) => return usage(&e),
    };
    let root = cx.dir.ancestors().nth(2).unwrap_or(&cx.dir);
    let block = block.unwrap_or_else(|| inst::block_for(root));
    let file = match file.map(Path::to_path_buf).or_else(|| {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/iohr-agent/checks.toml"))
    }) {
        Some(f) => f,
        None => return usage("no HOME; pass --file"),
    };
    let done = match inst::install(&chosen, &block, &file, dry_run) {
        Ok(d) => d,
        Err(e) => return failed(&e),
    };
    if cx.json {
        out_line!(
            "{}",
            json!({
                "ok": true, "dry_run": done.dry_run, "file": done.file.display().to_string(), "block": done.block,
                "entries": done.entries, "declared": done.declared, "cli_left_out": done.cli, "off_left_out": done.off,
                "backup": done.backup.as_ref().map(|b| b.display().to_string()),
            })
        );
        return Ok(());
    }
    let verb = if done.dry_run { "would put" } else { "put" };
    out_line!(
        "{verb} {} checks in block {} of {} ({} declared there)",
        done.entries,
        done.block,
        done.file.display(),
        done.declared
    );
    if done.cli > 0 {
        out_line!(
            "left out: {} cli checks (they never leave this machine)",
            done.cli
        );
    }
    if done.off > 0 {
        out_line!("left out: {} with enabled = false", done.off);
    }
    if let Some(b) = &done.backup {
        out_line!("backup: {}", b.display());
    }
    if !done.dry_run {
        out_line!("restart the agent to load them; it declares them in its next hello");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::human;

    #[test]
    fn human_keeps_whole_units() {
        assert_eq!(human(45), "45s");
        assert_eq!(human(300), "5m");
        assert_eq!(human(7200), "2h");
        assert_eq!(human(86_400), "1d");
        assert_eq!(human(90), "1m");
    }
}
