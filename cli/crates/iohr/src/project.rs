//! `iohr project init`: a project you build yourself (PRD 0088), set up with one command.
//!
//! The local half is real: the scaffold from a template (the `demo` template: a private macOS
//! service that heartbeats and serves a tiny panel, with PRD 0084's manifest), a git
//! repository with one commit, the project's own state under `~/.inorbit/projects/<name>/`
//! (never inside the repository), a Keychain slot reserved by name, and a smoke test that runs
//! the project once and reads its heartbeat back. The cloud half (the project in InOrbit, its
//! private listing, its scopes and tokens, memory, monitors and notifications) waits on APIs
//! that do not exist yet: each step is reported as `not-yet` with the API it needs, never
//! skipped in silence. Every step is re-runnable; nothing is overwritten.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::ext::lint::lint;
use crate::ext::manifest::{SOURCE_FILE, check_name as check_ext_name};

/// One file of a template: where it goes (placeholders allowed) and its text.
struct TemplateFile {
    path: &'static str,
    text: &'static str,
}

macro_rules! tf {
    ($path:expr, $src:expr) => {
        TemplateFile {
            path: $path,
            text: include_str!(concat!("../templates/project/", $src)),
        }
    };
}

const DEMO: &[TemplateFile] = &[
    tf!("extension.toml", "demo/extension.toml"),
    tf!("README.md", "demo/README.md"),
    tf!("inorbit-project.toml", "demo/inorbit-project.toml"),
    tf!(".gitignore", "demo/gitignore"),
    tf!("src/@@MODULE@@/__init__.py", "demo/src/module/__init__.py"),
    tf!("src/@@MODULE@@/__main__.py", "demo/src/module/__main__.py"),
    tf!("src/@@MODULE@@/channel.py", "demo/src/module/channel.py"),
    tf!("panel/index.html", "demo/panel/index.html"),
    tf!("tests/test_selftest.py", "demo/tests/test_selftest.py"),
];

/// The templates `--template` accepts. Only `demo` is live; the others say so and refuse.
pub const TEMPLATES: &[(&str, &str, bool)] = &[
    (
        "demo",
        "A private macOS service that heartbeats and serves a tiny local panel (Python, no dependencies)",
        true,
    ),
    (
        "macos-app",
        "A signed macOS app of your own; coming after the demo",
        false,
    ),
    (
        "ui-panel",
        "A panel inside InOrbit Desktop; coming after the demo",
        false,
    ),
];

/// The name rule: the folder, the Python module and the extension's name all come from it, so
/// it is the extension name rule (lowercase, digits, dashes, 2 to 32, never `tbd`) and never a
/// command of `iohr` itself.
///
/// # Errors
///
/// A one-line reason, in plain words.
pub fn check_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.len() < 2 {
        return Err("a project name is at least 2 characters".into());
    }
    if n.contains("tbd") {
        return Err("a project is named for what it is; `tbd` is refused".into());
    }
    check_ext_name(n).map_err(|e| e.0)
}

fn fill(text: &str, values: &[(&str, String)]) -> String {
    values
        .iter()
        .fold(text.to_owned(), |t, (k, v)| t.replace(k, v))
}

/// The template's files for `name`, rendered: (relative path, text).
///
/// # Errors
///
/// A bad name, an unknown template, or one that is coming.
pub fn render(
    template: &str,
    name: &str,
    publisher: &str,
    created_at: &str,
) -> Result<Vec<(String, String)>, String> {
    check_name(name)?;
    let files = match TEMPLATES.iter().find(|(id, _, _)| *id == template) {
        Some((_, _, true)) => DEMO,
        Some((id, _, false)) => {
            return Err(format!(
                "the `{id}` template is coming; `demo` is the one that works today"
            ));
        }
        None => return Err(format!("there is no `{template}` template")),
    };
    let values = [
        ("@@ID@@", format!("{publisher}/{name}")),
        ("@@NAME@@", name.to_owned()),
        ("@@MODULE@@", name.replace('-', "_")),
        ("@@CREATED_AT@@", created_at.to_owned()),
    ];
    Ok(files
        .iter()
        .map(|f| (fill(f.path, &values), fill(f.text, &values)))
        .collect())
}

/// One step of init, as the report shows it: what happened on this machine, or what it waits on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub title: String,
    /// `done`, `skipped`, `not-yet`, `failed` or `ask`.
    pub state: String,
    pub said: String,
    /// The API or decision a `not-yet` step waits on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub needs: String,
}

fn step(id: &str, title: &str, state: &str, said: impl Into<String>, needs: &str) -> Step {
    Step {
        id: id.into(),
        title: title.into(),
        state: state.into(),
        said: said.into(),
        needs: needs.into(),
    }
}

/// What init made, and every step of its report.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub name: String,
    pub id: String,
    pub dir: PathBuf,
    pub state: PathBuf,
    pub files: Vec<String>,
    pub steps: Vec<Step>,
}

impl Report {
    /// True when nothing failed; `not-yet` and `ask` are not failures.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.steps.iter().all(|s| s.state != "failed")
    }
}

fn private_dir(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Writes the project: the files, a git repository with its first commit, and the state
/// folders. `dir` must not exist or must be empty; nothing is overwritten. The template's
/// manifest must pass the lint, as `iohr ext init`'s must.
///
/// # Errors
///
/// A bad name or template, a busy folder, or a write that failed.
pub fn init(
    dir: &Path,
    state_root: &Path,
    name: &str,
    template: &str,
    publisher: &str,
    now: &str,
) -> Result<Report, String> {
    let name = name.trim();
    let files = render(template, name, publisher, now)?;
    if dir.exists()
        && std::fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .next()
            .is_some()
    {
        return Err(format!(
            "{} is not empty; choose another name or --dir",
            dir.display()
        ));
    }
    let manifest = files
        .iter()
        .find(|(p, _)| p == SOURCE_FILE)
        .map(|(_, t)| t.as_str())
        .unwrap_or_default();
    let report = lint(manifest);
    if !report.ok {
        return Err(format!(
            "the template's manifest does not pass the lint: {}",
            report
                .findings
                .iter()
                .map(|f| f.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut written = Vec::new();
    for (rel, text) in &files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
        written.push(rel.clone());
    }
    let mut steps = vec![step(
        "scaffold",
        "Scaffold from the template",
        "done",
        format!(
            "{} files from `{template}` in {}",
            files.len(),
            dir.display()
        ),
        "",
    )];
    let committed = git(dir, &["init", "-q", "-b", "main"])
        && git(dir, &["add", "-A"])
        && git(
            dir,
            &[
                "-c",
                "user.name=iohr",
                "-c",
                "user.email=iohr@inorbit.hr",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "-m",
                &format!("{name}: new project from the {template} template"),
            ],
        );
    steps.push(if committed {
        step(
            "git",
            "A git repository with its first commit",
            "done",
            "main, one commit",
            "",
        )
    } else {
        step(
            "git",
            "A git repository with its first commit",
            "skipped",
            "git did not answer; the folder works without it",
            "",
        )
    });
    let state = state_root.join(name);
    private_dir(&state.join("state"))?;
    private_dir(&state.join("cache"))?;
    let record = format!(
        "# iohr's record of a project you build yourself. Nothing sensitive, ever.\nversion = 1\nname = {name:?}\npath = {:?}\ntemplate = {template:?}\ncreated_at = {now:?}\n",
        dir.to_string_lossy()
    );
    std::fs::write(state.join("project.toml"), record)
        .map_err(|e| format!("cannot write the project record: {e}"))?;
    steps.push(step(
        "runtime",
        "Registered with the local runtime",
        "done",
        format!(
            "state and cache under {}; project.toml there is what the worker's project slot reads",
            state.display()
        ),
        "",
    ));
    steps.push(step(
        "keychain",
        "A Keychain slot for the project's own token",
        "not-yet",
        format!("reserved by name, hr.inorbit.project.{name}; no token exists until InOrbit issues per-project scopes"),
        "per-project scopes and tokens (PRD 0088, RFC 0127.1)",
    ));
    steps.push(step(
        "allowed",
        "The editor may open it",
        "ask",
        "InOrbit Desktop asks you before Code may open a folder: Code › Allow a repository…, or New project does it with your confirm",
        "",
    ));
    steps.push(step(
        "space",
        "The project in InOrbit: a Decisions space of kind project",
        "not-yet",
        "decisions_create_space works for a person today; init does it once a project has a scope of its own",
        "POST /v1/decisions/projects (RFC 0115), a project scope",
    ));
    steps.push(step(
        "listing",
        "A private listing in the Marketplace",
        "not-yet",
        "extensions.listings has private visibility; nothing registers one from a developer's machine yet",
        "a developer's POST /v1/extensions/listings, extensions:write (RFC 0127.1)",
    ));
    steps.push(step(
        "memory",
        "A memory namespace of its own",
        "not-yet",
        "memory is per account today",
        "per-project memory namespaces (PRD 0082, RFC 0106, RFC 0127.2)",
    ));
    steps.push(step(
        "monitors",
        "Its own monitors and a notifications channel",
        "not-yet",
        "a heartbeat monitor tagged project:<name> and push to your phone",
        "monitors with a project tag; a notifications API; push as a subscription channel (RFC 0127.2)",
    ));
    Ok(Report {
        name: name.to_owned(),
        id: format!("{publisher}/{name}"),
        dir: dir.to_path_buf(),
        state,
        files: written,
        steps,
    })
}

/// The smoke test's result.
#[derive(Debug, Clone, Serialize)]
pub struct Smoke {
    pub ok: bool,
    pub said: String,
    pub heartbeat: Option<serde_json::Value>,
    pub output: String,
}

/// Runs the project once (`IOHR_EXT_ONCE=1`) with `STATE` and `CACHE` under its own folder, the
/// way `iohr ext dev --test` does, and reads the heartbeat back. No token channel is given, so
/// the project's token ask is refused: that refusal is the proof the channel is its only
/// credential. Nothing of the caller's environment reaches the child.
#[must_use]
pub fn smoke(dir: &Path, state: &Path, name: &str) -> Smoke {
    let st = state.join("state");
    let cache = state.join("cache");
    if private_dir(&st).and(private_dir(&cache)).is_err() {
        return Smoke {
            ok: false,
            said: "the project's state folder could not be made".into(),
            heartbeat: None,
            output: String::new(),
        };
    }
    let module = name.replace('-', "_");
    let out = Command::new("python3")
        .arg("-m")
        .arg(&module)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
        .env("HOME", &cache)
        .env("STATE", &st)
        .env("CACHE", &cache)
        .env("IOHR_EXT_ONCE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONPATH", dir.join("src"))
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            return Smoke {
                ok: false,
                said: format!("python3 could not be started: {e}"),
                heartbeat: None,
                output: String::new(),
            };
        }
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let beat = std::fs::read_to_string(st.join("heartbeat.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let state_ok = beat
        .as_ref()
        .and_then(|b| b.get("state").and_then(serde_json::Value::as_str))
        == Some("ok");
    if out.status.success() && state_ok {
        let tok = beat
            .as_ref()
            .and_then(|b| b.get("token").and_then(serde_json::Value::as_str))
            .unwrap_or("?")
            .to_owned();
        Smoke {
            ok: true,
            said: format!(
                "one heartbeat in {}; the token ask was {tok} (no channel was given, so refused is right)",
                st.display()
            ),
            heartbeat: beat,
            output: text,
        }
    } else {
        Smoke {
            ok: false,
            said: if out.status.success() {
                "it ran, but wrote no heartbeat".into()
            } else {
                format!("it exited with {}", out.status)
            },
            heartbeat: beat,
            output: text,
        }
    }
}

/// The report, one line per step: done first, then asks and skips, then what waits on what.
#[must_use]
pub fn lines(r: &Report, smoke: Option<&Smoke>, signed_in: Option<&str>) -> Vec<String> {
    let mut out = vec![format!("{} is at {}", r.id, r.dir.display())];
    out.push(match signed_in {
        Some(who) => format!("done: signed in as {who}"),
        None => {
            "not yet: not signed in; run iohr login first, so init can make the project in InOrbit"
                .to_owned()
        }
    });
    for s in &r.steps {
        if s.state == "done" {
            out.push(format!("done: {} ({})", s.title.to_lowercase(), s.said));
        }
    }
    if let Some(s) = smoke {
        out.push(if s.ok {
            format!("done: smoke test, {}", s.said)
        } else {
            format!("failed: smoke test, {}", s.said)
        });
    }
    for s in &r.steps {
        match s.state.as_str() {
            "skipped" => out.push(format!("skipped: {} ({})", s.title.to_lowercase(), s.said)),
            "ask" => out.push(format!("asked: {} ({})", s.title.to_lowercase(), s.said)),
            _ => {}
        }
    }
    for s in &r.steps {
        match s.state.as_str() {
            "not-yet" => out.push(format!(
                "not yet: {}; needs {}",
                s.title.to_lowercase(),
                if s.needs.is_empty() {
                    "an API that does not exist"
                } else {
                    &s.needs
                }
            )),
            "failed" => out.push(format!("failed: {} ({})", s.title.to_lowercase(), s.said)),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_rule() {
        for ok in ["demo", "my-shop", "a1"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "d",
            "Demo",
            "-demo",
            "my_shop",
            "tbd-thing",
            "ext",
            "iohr",
            "project",
        ] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_demo_renders_whole_and_its_manifest_passes_the_lint() {
        let files = render("demo", "my-shop", "local", "2026-10-12T10:00:00Z").unwrap();
        assert!(files.iter().any(|(p, _)| p == "src/my_shop/__main__.py"));
        for (p, t) in &files {
            assert!(!t.contains("@@"), "{p} keeps a placeholder");
            assert!(!p.contains("@@"), "{p}");
        }
        let (_, manifest) = files.iter().find(|(p, _)| p == SOURCE_FILE).unwrap();
        let r = lint(manifest);
        assert!(r.ok, "{:?}", r.findings);
        assert!(manifest.contains("id = \"local/my-shop\""));
        assert!(
            render("macos-app", "demo", "local", "")
                .unwrap_err()
                .contains("coming")
        );
        assert!(render("nope", "demo", "local", "").is_err());
        assert!(render("demo", "tbd", "local", "").is_err());
    }

    #[test]
    fn init_writes_the_project_and_its_state_refuses_a_busy_folder_and_reports_every_step() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("development/demo");
        let st = d.path().join("projects");
        let r = init(&dir, &st, "demo", "demo", "local", "2026-10-12T10:00:00Z").unwrap();
        assert_eq!(r.id, "local/demo");
        assert!(dir.join("extension.toml").is_file());
        assert!(dir.join("src/demo/__main__.py").is_file());
        assert!(st.join("demo/state").is_dir() && st.join("demo/cache").is_dir());
        assert!(st.join("demo/project.toml").is_file());
        let ids: Vec<&str> = r.steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "scaffold", "git", "runtime", "keychain", "allowed", "space", "listing", "memory",
                "monitors"
            ]
        );
        assert!(r.ok());
        assert!(
            r.steps
                .iter()
                .filter(|s| s.state == "not-yet")
                .all(|s| !s.needs.is_empty()),
            "a stub names what it needs"
        );
        let l = lines(&r, None, None);
        assert_eq!(
            l[0],
            "local/demo is at ".to_owned() + &dir.display().to_string()
        );
        assert!(l[1].starts_with("not yet: not signed in"));
        assert!(
            l.iter()
                .any(|x| x.starts_with("asked: the editor may open it"))
        );
        assert!(l.last().unwrap().starts_with("not yet: its own monitors"));
        assert!(
            init(&dir, &st, "demo", "demo", "local", "")
                .unwrap_err()
                .contains("not empty")
        );
    }

    #[test]
    fn the_smoke_test_runs_the_project_once_and_reads_one_heartbeat() {
        if Command::new("python3")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("no python3 here: the smoke test is not run");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("development/my-shop");
        let st = d.path().join("projects");
        let r = init(
            &dir,
            &st,
            "my-shop",
            "demo",
            "local",
            "2026-10-12T10:00:00Z",
        )
        .unwrap();
        let s = smoke(&r.dir, &r.state, "my-shop");
        assert!(s.ok, "{} / {}", s.said, s.output);
        let b = s.heartbeat.unwrap();
        assert_eq!(b["id"], "local/my-shop");
        assert_eq!(b["beats"], 1);
        assert_eq!(b["token"], "refused");
        assert!(!dir.join("heartbeat.json").exists());
        assert!(!dir.join("src/my_shop/__pycache__").exists());
    }
}
