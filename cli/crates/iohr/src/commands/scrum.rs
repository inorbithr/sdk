//! `iohr atlas scrum`: a thin command layer over [`crate::scrum`] (RFC 0234). Every view reads
//! `atlas/scrum/board.json`; every change is checked against the schemas first and written
//! atomically. Only `sync` calls the API, as the session's own caller.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::Env;
use crate::cli::{Global, ScrumArgs, ScrumCommand};
use crate::context::session;
use crate::error::Error;
use crate::output::Out;
use crate::planner::Plans;
use crate::scrum::board::{self, Board, Card};
use crate::scrum::view::{self, Look};
use crate::scrum::watch::{self, Form, Kind};
use crate::scrum::{self, change, sync, today};

fn board_path(given: Option<PathBuf>) -> Result<PathBuf, Error> {
    if let Some(p) = given {
        return Ok(p);
    }
    let here =
        std::env::current_dir().map_err(|e| Error::Failed(format!("no current folder: {e}")))?;
    board::find(&here).ok_or_else(|| {
        Error::Usage(format!(
            "no {} above this folder; pass --board (`iohr atlas init` writes an empty one)",
            scrum::BOARD_PATH
        ))
    })
}

fn readme_of(board: &Path) -> PathBuf {
    board.with_file_name("README.md")
}

/// owner/repo from the git remote `origin` (https or ssh form), else the folder above atlas/.
fn repo_of(board: &Path) -> String {
    let root = board.parent().and_then(Path::parent).and_then(Path::parent);
    let remote = root.and_then(|r| {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(r)
            .args(["remote", "get-url", "origin"])
            .output()
            .ok()?;
        let url = String::from_utf8(o.stdout).ok()?;
        let url = url.trim().trim_end_matches(".git");
        let mut parts = url.rsplit(['/', ':']);
        let (repo, owner) = (parts.next()?, parts.next()?);
        (o.status.success() && !repo.is_empty() && !owner.is_empty())
            .then(|| format!("{owner}/{repo}"))
    });
    remote.unwrap_or_else(|| {
        root.and_then(Path::file_name)
            .map_or_else(|| "repo".into(), |n| n.to_string_lossy().into_owned())
    })
}

fn render<T: Serialize>(
    form: Form,
    v: &T,
    text: impl FnOnce() -> String,
) -> Result<String, scrum::Error> {
    let j = match form {
        Form::Text => return Ok(text()),
        Form::Pretty => serde_json::to_string_pretty(v),
        Form::Line => serde_json::to_string(v),
    };
    j.map(|s| s + "\n")
        .map_err(|e| scrum::Error::Io(e.to_string()))
}

fn put(json: bool, v: &impl Serialize, text: impl FnOnce() -> String) {
    if json {
        Out::print_json(v);
    } else {
        Out::raw(text().as_bytes());
    }
}

/// The kind of screen `--watch` draws for a command; none for a command that is not a view.
const fn kind_of(cmd: &ScrumCommand) -> Option<Kind> {
    match cmd {
        ScrumCommand::Board => Some(Kind::Board),
        ScrumCommand::Plan => Some(Kind::Plan),
        ScrumCommand::List { .. } | ScrumCommand::Next { .. } => Some(Kind::Rows),
        _ => None,
    }
}

fn view_text(
    cmd: &ScrumCommand,
    b: &Board,
    today: &str,
    form: Form,
    look: &Look,
) -> Result<String, scrum::Error> {
    match cmd {
        ScrumCommand::Board => render(form, b, || view::board_view(b, today, look)),
        ScrumCommand::List {
            list,
            section,
            label,
            late,
            blocked,
        } => {
            let f = view::Filter {
                list: list.clone(),
                section: section.clone(),
                label: label.clone(),
                late: *late,
                blocked: *blocked,
            };
            let cards = view::list(b, &f, today);
            render(form, &cards, || view::rows(&cards, today, look))
        }
        ScrumCommand::Next { n } => {
            let cards: Vec<&Card> = view::next(b, *n);
            render(form, &cards, || {
                if cards.is_empty() {
                    "nothing is ready with its dependencies done\n".into()
                } else {
                    view::rows(&cards, today, look)
                }
            })
        }
        ScrumCommand::Plan => {
            let p = view::plan(b, today);
            render(form, &p, || view::plan_text(&p))
        }
        _ => Ok(String::new()),
    }
}

pub(crate) async fn run(g: &Global, env: &Env, a: ScrumArgs, out: Out) -> Result<(), Error> {
    let path = board_path(a.board)?;
    let cmd = a.cmd.unwrap_or(ScrumCommand::Board);
    if a.watch {
        let Some(kind) = kind_of(&cmd) else {
            return Err(Error::Usage(
                "--watch works with board, list, next and plan".into(),
            ));
        };
        let draw =
            |b: &Board, today: &str, form: Form, look: &Look| view_text(&cmd, b, today, form, look);
        let v = watch::View { kind, draw: &draw };
        return Ok(watch::watch(&path, &v, out.json)?);
    }
    if let ScrumCommand::Sync { dry_run, repo } = &cmd {
        return sync_cmd(g, env, &path, *dry_run, repo.clone(), out).await;
    }
    Ok(go(&path, cmd, out)?)
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per command, each a few lines"
)]
fn go(path: &Path, cmd: ScrumCommand, out: Out) -> Result<(), scrum::Error> {
    use scrum::Error as E;
    let today = today();
    let json = out.json;
    if let ScrumCommand::Check = cmd {
        let raw = std::fs::read(path)
            .map_err(|e| E::Io(format!("cannot read {}: {e}", path.display())))?;
        let value: Value = serde_json::from_slice(&raw)
            .map_err(|e| E::Refused(vec![format!("{} is not JSON: {e}", path.display())]))?;
        let atlas = path
            .parent()
            .and_then(Path::parent)
            .filter(|p| p.file_name().is_some_and(|n| n == "atlas"));
        let problems = scrum::check::check(&value, atlas);
        put(json, &json!({ "problems": problems }), || {
            if problems.is_empty() {
                format!("{}: ok\n", path.display())
            } else {
                String::new()
            }
        });
        return if problems.is_empty() {
            Ok(())
        } else {
            Err(E::Refused(problems))
        };
    }
    if let ScrumCommand::Migrate = cmd {
        return migrate(path, out);
    }
    let loaded = board::load(path)?;
    let b = &loaded.board;
    let mut nb: Board = b.clone();
    let changed = |id: &str, nb: &Board| -> Result<(), E> {
        board::save(&loaded, nb.clone(), &today)?;
        let c = nb.card(id)?;
        put(json, c, || format!("{}: {} ({})\n", c.id, c.list, c.title));
        Ok(())
    };
    match cmd {
        ScrumCommand::Check | ScrumCommand::Migrate | ScrumCommand::Sync { .. } => Ok(()),
        ScrumCommand::Board
        | ScrumCommand::List { .. }
        | ScrumCommand::Next { .. }
        | ScrumCommand::Plan => {
            let form = if json { Form::Pretty } else { Form::Text };
            let text = view_text(&cmd, b, &today, form, &Look::stdout(json))?;
            Out::raw(text.as_bytes());
            Ok(())
        }
        ScrumCommand::Show { card } => {
            let c = b.card(&card)?;
            put(json, c, || view::show(c));
            Ok(())
        }
        ScrumCommand::Add {
            id,
            title,
            section,
            size,
            hours,
            due,
            start,
            docs,
            depends_on,
            labels,
            checklist,
            acceptance,
            tests,
        } => {
            let mut x = Map::new();
            if let Some(s) = section {
                x.insert("section".into(), json!(s));
            }
            let n = change::NewCard {
                id: id.clone(),
                title,
                x,
                size,
                hours,
                start,
                due,
                docs,
                depends_on,
                labels,
                checklist,
                acceptance,
                tests,
            };
            change::add(&mut nb, n, &today)?;
            changed(&id, &nb)
        }
        ScrumCommand::Edit {
            card,
            title,
            due,
            start,
            hours,
            size,
            labels,
            blocked_by,
            unblock,
        } => {
            let e = change::Edit {
                title,
                due,
                start,
                hours,
                size,
                labels,
                blocked_by,
                unblock,
            };
            change::edit(&mut nb, &card, e, &today)?;
            changed(&card, &nb)
        }
        ScrumCommand::Move {
            card,
            list,
            force,
            evidence,
        } => {
            change::move_to(
                &mut nb,
                &card,
                &list,
                force.as_deref(),
                evidence.as_deref(),
                &today,
            )?;
            changed(&card, &nb)
        }
        ScrumCommand::Start { card, force } => {
            change::start(&mut nb, &card, force.as_deref(), &today)?;
            changed(&card, &nb)
        }
        ScrumCommand::Done {
            card,
            pr,
            evidence,
            force,
        } => {
            change::done(
                &mut nb,
                &card,
                pr.as_deref(),
                evidence.as_deref(),
                force.as_deref(),
                &today,
            )?;
            changed(&card, &nb)
        }
        ScrumCommand::Tick { card, n, undo } => {
            change::tick(&mut nb, &card, n, undo)?;
            changed(&card, &nb)
        }
        ScrumCommand::Note { card, text } => {
            change::note(&mut nb, &card, &text.join(" "), &today)?;
            changed(&card, &nb)
        }
        ScrumCommand::Capacity { hours } => {
            if let Some(h) = hours {
                if !(h.is_finite() && h > 0.0) {
                    return Err(E::Refused(vec![format!(
                        "capacity must be more than 0 hours, not {h}"
                    )]));
                }
                nb.day_hours = Some(h);
                board::save(&loaded, nb.clone(), &today)?;
            }
            let h = nb.day_hours.unwrap_or(view::DAY_HOURS);
            put(json, &json!({ "day_hours": h }), || {
                format!("a day holds {} h\n", view::fmt_h(h))
            });
            Ok(())
        }
        ScrumCommand::Render { check } => {
            let readme = readme_of(path);
            let old = std::fs::read_to_string(&readme)
                .map_err(|e| E::Io(format!("cannot read {}: {e}", readme.display())))?;
            let new = view::render_readme(&old, b)?;
            if old == new {
                put(
                    json,
                    &json!({"readme": readme.display().to_string(), "state": "current"}),
                    || format!("{}: current\n", readme.display()),
                );
                Ok(())
            } else if check {
                Err(E::Refused(vec![format!(
                    "{} is stale; run `iohr atlas scrum render`",
                    readme.display()
                )]))
            } else {
                std::fs::write(&readme, new)
                    .map_err(|e| E::Io(format!("{}: {e}", readme.display())))?;
                put(
                    json,
                    &json!({"readme": readme.display().to_string(), "state": "rendered"}),
                    || format!("{}: rendered\n", readme.display()),
                );
                Ok(())
            }
        }
    }
}

/// `migrate`: a board written before the schemas keeps `section` and `ideas` on the card; the
/// schema keeps a project's own fields under `x`. The keys move, the file is written (the old
/// one is in git), and the board is checked: what the schemas refuse is listed and the exit
/// code is 1, so the board is fixed by hand and checked again. A board that is already
/// migrated is left as it is.
fn migrate(path: &Path, out: Out) -> Result<(), scrum::Error> {
    use scrum::Error as E;
    let raw =
        std::fs::read(path).map_err(|e| E::Io(format!("cannot read {}: {e}", path.display())))?;
    let mut value: Value = serde_json::from_slice(&raw)
        .map_err(|e| E::Refused(vec![format!("{} is not JSON: {e}", path.display())]))?;
    let mut moved = 0usize;
    if let Some(cards) = value.get_mut("cards").and_then(Value::as_array_mut) {
        for c in cards.iter_mut().filter_map(Value::as_object_mut) {
            for key in ["section", "ideas"] {
                if let Some(v) = c.remove(key) {
                    let x = c.entry("x").or_insert_with(|| Value::Object(Map::new()));
                    if let Some(x) = x.as_object_mut() {
                        x.insert(key.into(), v);
                    }
                    moved += 1;
                }
            }
        }
    }
    if moved > 0 {
        let mut text =
            serde_json::to_string_pretty(&value).map_err(|e| E::Io(format!("board: {e}")))?;
        text.push('\n');
        let tmp = path.with_file_name(".board.json.tmp");
        let io = |e: std::io::Error| E::Io(format!("{}: {e}", path.display()));
        std::fs::write(&tmp, text).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)?;
    }
    let atlas = path
        .parent()
        .and_then(Path::parent)
        .filter(|p| p.file_name().is_some_and(|n| n == "atlas"));
    let problems = scrum::check::check(&value, atlas);
    put(
        out.json,
        &json!({"moved": moved, "problems": problems}),
        || format!("moved {moved} fields under x\n"),
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(E::Refused(problems))
    }
}

async fn sync_cmd(
    g: &Global,
    env: &Env,
    path: &Path,
    dry_run: bool,
    repo: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let today = today();
    let json = out.json;
    let loaded = board::load(path)?;
    let repo = repo.unwrap_or_else(|| repo_of(path));
    if dry_run {
        let cmds = sync::sync_dry(&loaded.board, &repo);
        put(json, &json!({ "commands": cmds }), || {
            cmds.join("\n") + "\n"
        });
        return Ok(());
    }
    let s = session(g, env).await?;
    let mut nb = loaded.board.clone();
    let r = sync::sync(&mut nb, &repo, &Plans::new(&s.api, None), &s.label).await?;
    if !r.written.is_empty() {
        board::save(&loaded, nb, &today)?;
    }
    put(json, &r, || {
        let mut t = String::new();
        let _ = writeln!(
            t,
            "plan {}{}",
            r.plan,
            if r.created { " (created)" } else { "" }
        );
        for s in &r.sent {
            let _ = writeln!(t, "sent: {s}");
        }
        for s in &r.failed {
            let _ = writeln!(t, "refused: {s}");
        }
        if r.sent.is_empty() && r.failed.is_empty() {
            t.push_str("nothing to send\n");
        }
        if !r.written.is_empty() {
            let _ = writeln!(t, "planner ids written to {}", r.written.join(", "));
        }
        t
    });
    if r.failed.is_empty() {
        Ok(())
    } else {
        Err(Error::Failed(r.failed.join("\n")))
    }
}
