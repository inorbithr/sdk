//! `iohr project template list|show|path`: read the templates `init` accepts.
//!
//! The renderers are pure (a [`Paint`] says whether there is colour and a terminal), so the
//! tests read the exact text without a pty. Stdout carries data only; the dim hint under a
//! table is printed only when a person is reading (a terminal, not `--json`), never into a pipe.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::ask;
use crate::error::Error;
use crate::output::{Out, table_text};
use crate::template::{self, Answer, Kind, Origin, Row, Spec, StepName, Template};
use crate::ui::Paint;

/// `list`: the table, or the rows as JSON. `default` is the template `init` uses.
pub(crate) fn list(default: &str, out: Out) -> Result<(), Error> {
    let rows = template::list(&template::user_root()?, default);
    if out.json {
        Out::print_json(&rows);
    } else {
        Out::raw(render_list(&rows, Paint::stdout(false)).as_bytes());
    }
    Ok(())
}

/// The templates table: NAME, STATUS, ORIGIN, DEFAULT (a `*`) and DESCRIPTION, then, for a
/// person at a terminal, the next commands in dim.
pub(crate) fn render_list(rows: &[Row], p: Paint) -> String {
    let body: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                p.name(&r.name),
                p.status(&r.status),
                p.dim(&tilde(&r.origin)),
                if r.default {
                    p.mark("*")
                } else {
                    String::new()
                },
                r.description.clone(),
            ]
        })
        .collect();
    let head = ["NAME", "STATUS", "ORIGIN", "DEFAULT", "DESCRIPTION"].map(|h| p.head(h));
    let head: Vec<&str> = head.iter().map(String::as_str).collect();
    let mut text = table_text(&head, &body);
    if p.tty() {
        let name = rows.first().map_or("NAME", |r| r.name.as_str());
        let _ = write!(
            text,
            "\n{}\n",
            p.dim(&format!(
                "Next: iohr project template show {name}  ·  iohr project template new NAME"
            ))
        );
    }
    text
}

/// `user: /Users/me/.inorbit/templates/x` as `user: ~/.inorbit/templates/x`: shorter, and the
/// same on every machine.
fn tilde(origin: &str) -> String {
    let home = etcetera::home_dir().map(|h| h.display().to_string());
    match home.as_deref().ok().and_then(|h| origin.split_once(h)) {
        Some((before, rest)) => {
            format!("{before}~{}", crate::paths::slashed(Path::new(rest)))
        }
        None => origin.to_owned(),
    }
}

/// What `show` knows about one template, for `--json` and for the text alike.
#[derive(Debug, Serialize)]
pub(crate) struct Shown {
    pub id: String,
    pub description: String,
    pub status: String,
    pub origin: ShownOrigin,
    /// The folder, when the template has one on this machine (a built-in has none).
    pub path: Option<PathBuf>,
    pub extends: Option<String>,
    pub default: bool,
    pub steps: Vec<String>,
    pub agent: Option<ShownAgent>,
    pub questions: Vec<ShownQuestion>,
    /// The languages `--lang` takes (none for a template without variants).
    pub languages: Vec<ShownLanguage>,
    /// The paths it renders, placeholders as written (`src/@@NAME@@/main.py`).
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ShownOrigin {
    /// `built-in`, `user`, `path` or `git`.
    pub kind: &'static str,
    /// The folder or the repository, as it was named.
    pub location: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ShownAgent {
    /// Whether the template runs an `agent` step on `init`.
    pub step: bool,
    /// What the `agent` step tells the agent.
    pub step_prompt: Option<String>,
    /// What `init --describe` tells the agent.
    pub describe_prompt: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ShownLanguage {
    pub id: String,
    pub label: String,
    pub status: &'static str,
    /// The programs it needs on `PATH`.
    pub needs: Vec<String>,
    /// Whether every one of them is on this machine's `PATH`.
    pub ready: bool,
    /// The one `--yes` takes.
    pub default: bool,
    /// The paths it lays over the common files, placeholders as written.
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ShownQuestion {
    pub key: String,
    pub prompt: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub choices: Vec<String>,
    pub default: Option<Answer>,
    pub when: Option<String>,
    pub regex: Option<String>,
    pub help: Option<String>,
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::String => "string",
        Kind::Bool => "bool",
        Kind::Choice => "choice",
        Kind::Multi => "multi",
    }
}

fn step_name(s: StepName) -> &'static str {
    match s {
        StepName::Git => "git",
        StepName::Register => "register",
        StepName::Smoke => "smoke",
        StepName::Agent => "agent",
    }
}

/// The folder of a template that has one.
fn folder(origin: &Origin) -> Option<&Path> {
    match origin {
        Origin::User(p) | Origin::Path(p) => Some(p),
        Origin::BuiltIn | Origin::Git(_) => None,
    }
}

pub(crate) fn describe(t: &Template, is_default: bool) -> Shown {
    let steps = t.steps();
    let step_prompt = steps
        .iter()
        .find(|s| s.name == StepName::Agent)
        .and_then(|s| s.prompt.clone());
    let describe_prompt = t.meta.agent_prompt.clone();
    let has_step = steps.iter().any(|s| s.name == StepName::Agent);
    let has_agent = has_step || describe_prompt.is_some();
    Shown {
        id: t.meta.id.clone(),
        description: t.meta.description.clone(),
        status: if t.meta.status == template::Status::Live {
            "live"
        } else {
            "coming"
        }
        .to_owned(),
        origin: match &t.origin {
            Origin::BuiltIn => ShownOrigin {
                kind: "built-in",
                location: None,
            },
            Origin::User(p) => ShownOrigin {
                kind: "user",
                location: Some(p.display().to_string()),
            },
            Origin::Path(p) => ShownOrigin {
                kind: "path",
                location: Some(p.display().to_string()),
            },
            Origin::Git(u) => ShownOrigin {
                kind: "git",
                location: Some(u.clone()),
            },
        },
        path: folder(&t.origin).map(Path::to_path_buf),
        extends: t.meta.extends.clone(),
        default: is_default,
        steps: steps.iter().map(|s| step_name(s.name).to_owned()).collect(),
        agent: has_agent.then_some(ShownAgent {
            step: has_step,
            step_prompt,
            describe_prompt,
        }),
        questions: t
            .meta
            .questions
            .iter()
            .map(|q| ShownQuestion {
                key: q.key.clone(),
                prompt: q.prompt.clone(),
                kind: kind_name(q.kind),
                choices: q.choices.clone(),
                default: q.default.clone(),
                when: q.when.clone(),
                regex: q.regex.clone(),
                help: q.help.clone(),
            })
            .collect(),
        languages: {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let default = t.default_variant().map(|v| v.id.clone());
            t.meta
                .variants
                .iter()
                .map(|v| ShownLanguage {
                    id: v.id.clone(),
                    label: v.label.clone(),
                    status: if v.status == template::Status::Live {
                        "live"
                    } else {
                        "coming"
                    },
                    needs: v.needs.clone(),
                    ready: v
                        .needs
                        .iter()
                        .all(|n| ask::find_on_path(n, &path).is_some()),
                    default: default.as_deref() == Some(v.id.as_str()),
                    files: t
                        .variant_files
                        .iter()
                        .filter(|(id, _, _)| *id == v.id)
                        .map(|(_, p, _)| p.clone())
                        .collect(),
                })
                .collect()
        },
        files: t.files.iter().map(|(p, _)| p.clone()).collect(),
    }
}

/// `show`: one template, in sections, or all of it as one JSON object.
pub(crate) fn show(spec: &str, default: &str, out: Out) -> Result<(), Error> {
    let t = template::load(&template::parse_spec(spec), &template::user_root()?)
        .map_err(Error::Failed)?;
    let shown = describe(&t, t.meta.id == default);
    if out.json {
        Out::print_json(&shown);
    } else {
        Out::raw(render_show(&shown, Paint::stdout(false)).as_bytes());
    }
    Ok(())
}

/// A title, then rows under it indented by two, git log and cargo tree style.
fn section(text: &mut String, p: Paint, title: &str) {
    let _ = write!(text, "\n{}\n", p.head(title));
}

fn render_answer(a: &Answer) -> String {
    match a {
        Answer::Text(t) => format!("\"{t}\""),
        other => other.text(),
    }
}

/// The languages `--lang` takes: id, status, what it is, the default and the toolchain.
fn render_languages(text: &mut String, s: &Shown, p: Paint) {
    if s.languages.is_empty() {
        return;
    }
    section(text, p, "Languages");
    let rows: Vec<Vec<String>> = s
        .languages
        .iter()
        .map(|l| {
            let found = if l.needs.is_empty() || l.ready {
                "ready"
            } else {
                "needs"
            };
            vec![
                p.name(&l.id),
                p.status(l.status),
                l.label.clone(),
                if l.default { "default" } else { "" }.to_owned(),
                if l.needs.is_empty() {
                    found.to_owned()
                } else {
                    format!("{found}: {}", l.needs.join(", "))
                },
            ]
        })
        .collect();
    let head = ["ID", "STATUS", "LANGUAGE", "", "TOOLCHAIN"].map(|h| p.dim(h));
    let head: Vec<&str> = head.iter().map(String::as_str).collect();
    for line in table_text(&head, &rows).lines() {
        let _ = writeln!(text, "  {line}");
    }
}

pub(crate) fn render_show(s: &Shown, p: Paint) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{}  {}", p.name(&s.id), p.status(&s.status));
    let _ = writeln!(text, "{}", s.description);

    // Facts, keys aligned.
    let origin = match (&s.origin.location, s.origin.kind) {
        (Some(l), k) => format!("{k}: {l}"),
        (None, k) => k.to_owned(),
    };
    let mut facts: Vec<(&str, String)> = vec![("Origin", origin)];
    facts.push((
        "Path",
        s.path.as_ref().map_or_else(
            || "none: built into iohr; `iohr project template new NAME --extends {id}` makes an editable copy"
                .replace("{id}", &s.id),
            |p| p.display().to_string(),
        ),
    ));
    if let Some(e) = &s.extends {
        facts.push(("Extends", e.clone()));
    }
    facts.push(("Default", if s.default { "yes" } else { "no" }.to_owned()));
    facts.push(("Steps", s.steps.join(", ")));
    let w = facts.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    text.push('\n');
    for (k, v) in &facts {
        let _ = writeln!(text, "{}  {v}", p.dim(&format!("{k:w$}")));
    }

    section(&mut text, p, "Questions");
    if s.questions.is_empty() {
        let _ = writeln!(text, "  {}", p.dim("none"));
    } else {
        let rows: Vec<Vec<String>> = s
            .questions
            .iter()
            .map(|q| {
                let kind = if q.choices.is_empty() {
                    q.kind.to_owned()
                } else {
                    format!("{} ({})", q.kind, q.choices.join(", "))
                };
                vec![
                    p.name(&q.key),
                    kind,
                    q.default.as_ref().map(render_answer).unwrap_or_default(),
                    q.when.clone().unwrap_or_default(),
                ]
            })
            .collect();
        let head = ["ID", "TYPE", "DEFAULT", "WHEN"].map(|h| p.dim(h));
        let head: Vec<&str> = head.iter().map(String::as_str).collect();
        for line in table_text(&head, &rows).lines() {
            let _ = writeln!(text, "  {line}");
        }
    }

    render_languages(&mut text, s, p);

    section(&mut text, p, "Files");
    if s.files.is_empty() {
        let _ = writeln!(text, "  {}", p.dim("none"));
    }
    for f in &s.files {
        let _ = writeln!(text, "  {f}");
    }

    if let Some(a) = &s.agent {
        section(
            &mut text,
            p,
            if a.step { "Agent step" } else { "Agent prompt" },
        );
        let mut part = |label: &str, body: &Option<String>| {
            if let Some(b) = body {
                let _ = writeln!(text, "  {}", p.dim(label));
                for l in b.lines() {
                    let _ = writeln!(text, "    {l}");
                }
            }
        };
        part("run by the agent step", &a.step_prompt);
        part("given with --describe", &a.describe_prompt);
    }

    if p.tty() {
        // A built-in has no folder to print, so the second hint is the way to get one.
        let then = if s.path.is_some() {
            format!("iohr project template path {}", s.id)
        } else {
            format!("iohr project template new NAME --extends {}", s.id)
        };
        let _ = write!(
            text,
            "\n{}\n",
            p.dim(&format!(
                "Next: iohr project init NAME --template {}  ·  {then}",
                s.id
            ))
        );
    }
    text
}

/// `path`: only the folder, so `cd $(iohr project template path NAME)` works.
pub(crate) fn path(spec: &str, out: Out) -> Result<(), Error> {
    let spec = template::parse_spec(spec);
    if matches!(spec, Spec::Git { .. }) {
        return Err(Error::Failed(
            "a git template has no folder on this machine; `iohr project template show` reads it"
                .into(),
        ));
    }
    // Loaded, so a folder that is not a template, or an id that is none, is refused.
    let t = template::load(&spec, &template::user_root()?).map_err(Error::Failed)?;
    let dir = folder(&t.origin).map(Path::to_path_buf).ok_or_else(|| {
        Error::Failed(format!(
            "`{}` is built into iohr and has no folder; `iohr project template new NAME --extends {}` makes one you can edit",
            t.meta.id, t.meta.id
        ))
    })?;
    if out.json {
        Out::print_json(&serde_json::json!({ "path": dir }));
    } else {
        Out::raw(format!("{}\n", dir.display()).as_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Row> {
        vec![
            Row {
                name: "demo".into(),
                description: "The demo".into(),
                status: "live".into(),
                default: true,
                origin: "built-in".into(),
            },
            Row {
                name: "ui".into(),
                description: "A panel".into(),
                status: "coming".into(),
                default: false,
                origin: "built-in".into(),
            },
        ]
    }

    fn terminal() -> Paint {
        Paint::from_parts(true, false, None, Some("xterm-256color"))
    }

    #[test]
    fn a_pipe_gets_the_table_alone() {
        let t = render_list(&rows(), Paint::plain());
        assert!(!t.contains("Next:") && !t.contains('\u{1b}'), "{t}");
        let first: Vec<&str> = t.lines().next().unwrap().split_whitespace().collect();
        assert_eq!(
            first,
            ["NAME", "STATUS", "ORIGIN", "DEFAULT", "DESCRIPTION"]
        );
    }

    #[test]
    fn a_terminal_gets_colour_markers_and_the_dim_footer() {
        let t = render_list(&rows(), terminal());
        assert!(t.contains('\u{1b}'), "{t}");
        let plain = console::strip_ansi_codes(&t).into_owned();
        assert!(plain.contains("\u{25cf} live"), "{plain}");
        assert!(plain.contains("\u{25cb} coming"), "{plain}");
        assert!(plain.contains('*'), "{plain}");
        assert!(
            plain.trim_end().ends_with(
                "Next: iohr project template show demo  \u{b7}  iohr project template new NAME"
            ),
            "{plain}"
        );
    }

    #[test]
    fn a_terminal_with_no_color_keeps_the_footer_but_no_escape_codes() {
        let p = Paint::from_parts(true, false, Some("1"), Some("xterm"));
        let t = render_list(&rows(), p);
        assert!(
            t.contains("Next:") && !t.contains('\u{1b}') && !t.contains('\u{25cf}'),
            "{t}"
        );
    }

    #[test]
    fn json_means_no_footer_even_on_a_terminal() {
        let p = Paint::from_parts(true, true, None, Some("xterm"));
        assert!(!render_list(&rows(), p).contains("Next:"));
    }

    #[test]
    fn colour_columns_line_up_like_plain_ones() {
        let strip = |s: &str| console::strip_ansi_codes(s).into_owned();
        let coloured = strip(&render_list(&rows(), terminal()));
        let col = |t: &str| t.lines().nth(1).unwrap().find("The demo").unwrap();
        let plain = render_list(&rows(), Paint::plain());
        // The marker is two columns wider in colour; the description still starts in one column
        // for every row.
        let starts: Vec<usize> = coloured
            .lines()
            .skip(1)
            .take(2)
            .map(|l| l.find("The demo").or_else(|| l.find("A panel")).unwrap())
            .collect();
        assert_eq!(starts[0], starts[1], "{coloured}");
        assert!(col(&plain) > 0);
    }
}
