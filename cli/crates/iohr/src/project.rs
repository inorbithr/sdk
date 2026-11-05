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

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::ext::lint::lint;
use crate::ext::manifest::{SOURCE_FILE, check_name as check_ext_name};
use crate::template::Runner;

/// The template used when neither `--template` nor `[project] template` names one.
pub mod folder;

pub const DEFAULT_TEMPLATE: &str = "demo";

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

/// The template to use: the flag, else `[project] template` from iohr's config, else
/// [`DEFAULT_TEMPLATE`].
#[must_use]
pub fn choose_template(flag: Option<&str>, configured: Option<&str>) -> String {
    [flag, configured]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|t| !t.is_empty())
        .unwrap_or(DEFAULT_TEMPLATE)
        .to_owned()
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

pub fn step(id: &str, title: &str, state: &str, said: impl Into<String>, needs: &str) -> Step {
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
        .is_ok_and(|o| o.status.success())
}

/// `git init`, add everything and one commit; false when git did not answer.
fn first_commit(dir: &Path, name: &str, template: &str) -> bool {
    git(dir, &["init", "-q", "-b", "main"])
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
        )
}

/// The steps that wait on InOrbit, each with the API it needs; never done in silence.
#[must_use]
pub fn cloud_steps(name: &str) -> Vec<Step> {
    vec![
        step(
            "keychain",
            "A Keychain slot for the project's own token",
            "not-yet",
            format!(
                "reserved by name, hr.inorbit.project.{name}; no token exists until InOrbit issues per-project scopes"
            ),
            "per-project scopes and tokens (PRD 0088, RFC 0127.1)",
        ),
        step(
            "allowed",
            "The editor may open it",
            "ask",
            "InOrbit Desktop asks you before Code may open a folder: Code › Allow a repository…, or New project does it with your confirm",
            "",
        ),
        step(
            "space",
            "The project in InOrbit: a Decisions space of kind project",
            "not-yet",
            "decisions_create_space works for a person today; init does it once a project has a scope of its own",
            "POST /v1/decisions/projects (RFC 0115), a project scope",
        ),
        step(
            "listing",
            "A private listing in the Marketplace",
            "not-yet",
            "extensions.listings has private visibility; nothing registers one from a developer's machine yet",
            "a developer's POST /v1/extensions/listings, extensions:write (RFC 0127.1)",
        ),
        step(
            "memory",
            "A memory namespace of its own",
            "not-yet",
            "memory is per account today",
            "per-project memory namespaces (PRD 0082, RFC 0106, RFC 0127.2)",
        ),
        step(
            "monitors",
            "Its own monitors and a notifications channel",
            "not-yet",
            "a heartbeat monitor tagged project:<name> and push to your phone",
            "monitors with a project tag; a notifications API; push as a subscription channel (RFC 0127.2)",
        ),
    ]
}

/// Writes the rendered files under `dir`, making it first; the paths written, in order. A path
/// in `programs` gets the executable bit.
fn write_files(
    dir: &Path,
    files: &[(String, String)],
    programs: &[String],
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut written = Vec::new();
    for (rel, text) in files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
        if programs.contains(rel) {
            make_executable(&p)?;
        }
        written.push(rel.clone());
    }
    Ok(written)
}

#[cfg(unix)]
fn make_executable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("cannot make {} executable: {e}", p.display()))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<(), String> {
    Ok(())
}

/// The folder must not exist or must hold nothing but dotfiles (an empty folder, or the one a
/// person is standing in), and no file the template writes may already be there: nothing is
/// overwritten. A read, so a dry run checks it too.
///
/// # Errors
///
/// A busy folder.
pub fn check_target(dir: &Path, files: &[(String, String)]) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let busy = !folder::only_dotfiles(dir) || files.iter().any(|(p, _)| dir.join(p).exists());
    if busy {
        return Err(format!(
            "{} is not empty; choose another name or --dir",
            dir.display()
        ));
    }
    Ok(())
}

/// The manifest check of a template's rendered files: when there is an `extension.toml`, it
/// must pass the lint, as `iohr ext init`'s must. A read.
///
/// # Errors
///
/// What the lint found.
pub fn check_manifest(files: &[(String, String)]) -> Result<(), String> {
    let Some((_, manifest)) = files.iter().find(|(p, _)| p == SOURCE_FILE) else {
        return Ok(());
    };
    let report = lint(manifest);
    if report.ok {
        return Ok(());
    }
    Err(format!(
        "the template's manifest does not pass the lint: {}",
        report
            .findings
            .iter()
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// The scaffold step: writes the files under `dir` (a real action: callers reach it only
/// through the dry-run gate).
///
/// # Errors
///
/// A busy folder, a manifest that fails the lint, or a write that failed.
pub fn scaffold(
    dir: &Path,
    files: &[(String, String)],
    programs: &[String],
    template: &str,
) -> Result<(Vec<String>, Step), String> {
    check_target(dir, files)?;
    check_manifest(files)?;
    let written = write_files(dir, files, programs)?;
    let s = step(
        "scaffold",
        "Scaffold from the template",
        "done",
        format!(
            "{} files from `{template}` in {}",
            files.len(),
            dir.display()
        ),
        "",
    );
    Ok((written, s))
}

/// The git step: a repository with its first commit, or a skipped step when git did not
/// answer.
#[must_use]
pub fn git_step(dir: &Path, name: &str, template: &str) -> Step {
    if first_commit(dir, name, template) {
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
    }
}

/// The register step: the project's state folders and its record under `state_root/<name>/`.
/// Returns the state folder with the step.
///
/// # Errors
///
/// A folder or the record could not be written.
pub fn register_step(
    dir: &Path,
    state_root: &Path,
    name: &str,
    template: &str,
    now: &str,
) -> Result<(PathBuf, Step), String> {
    let state = state_root.join(name);
    private_dir(&state.join("state"))?;
    private_dir(&state.join("cache"))?;
    let record = format!(
        "# iohr's record of a project you build yourself. Nothing sensitive, ever.\nversion = 1\nname = {name:?}\npath = {:?}\ntemplate = {template:?}\ncreated_at = {now:?}\n",
        dir.to_string_lossy()
    );
    std::fs::write(state.join("project.toml"), record)
        .map_err(|e| format!("cannot write the project record: {e}"))?;
    let s = step(
        "runtime",
        "Registered with the local runtime",
        "done",
        format!(
            "state and cache under {}; project.toml there is what the worker's project slot reads",
            state.display()
        ),
        "",
    );
    Ok((state, s))
}

/// The prompt the agent gets: the template's own words, then the project, the answers and
/// what the person described. It goes to the agent's argv and nowhere else.
#[must_use]
pub fn agent_prompt(
    template_prompt: Option<&str>,
    describe: Option<&str>,
    name: &str,
    template: &str,
    answers: &crate::template::Answers,
    vals: &[(String, String)],
) -> String {
    let base = template_prompt.unwrap_or(
        "This folder is a fresh InOrbit project. Build what is described below by editing this folder. Keep it free of secrets.",
    );
    let mut p = crate::template::fill_text(base, vals);
    let _ = write!(
        p,
        "\n\nThe project is `{name}`, scaffolded from the `{template}` template; you are in its folder."
    );
    if !answers.is_empty() {
        p.push_str("\n\nThe answers given at setup:");
        for (k, a) in answers {
            let _ = write!(p, "\n- {k}: {}", a.text());
        }
    }
    if let Some(d) = describe {
        let _ = write!(p, "\n\nWhat the person wants:\n{d}");
    }
    p
}

/// The smoke test's result.
#[derive(Debug, Clone, Serialize)]
pub struct Smoke {
    pub ok: bool,
    pub said: String,
    pub heartbeat: Option<serde_json::Value>,
    pub output: String,
}

/// `$DIR`, `$STATE` and `$CACHE` in a runner word.
fn paths_in(word: &str, dir: &Path, state: &Path, cache: &Path) -> String {
    word.replace("$DIR", &dir.to_string_lossy())
        .replace("$STATE", &state.to_string_lossy())
        .replace("$CACHE", &cache.to_string_lossy())
}

fn failed(said: String, output: String) -> Smoke {
    Smoke {
        ok: false,
        said,
        heartbeat: None,
        output,
    }
}

/// What ran before languages were variants: the Python module of the project.
fn legacy_runner(name: &str) -> Runner {
    Runner {
        label: "Python".into(),
        build: Vec::new(),
        run: vec!["python3".into(), "-m".into(), name.replace('-', "_")],
        env: std::collections::BTreeMap::from([
            ("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned()),
            ("PYTHONPATH".to_owned(), "$DIR/src".to_owned()),
        ]),
    }
}

/// The language's build, in the caller's own environment (a toolchain keeps its home there),
/// writing only under `$CACHE`. `Some` is the failure.
fn build_step(
    build: &[String],
    label: &str,
    dir: &Path,
    st: &Path,
    cache: &Path,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<Smoke> {
    let (prog, args) = build.split_first()?;
    let Some(bin) = find(prog) else {
        return Some(failed(
            format!("{prog} was not found on your PATH"),
            String::new(),
        ));
    };
    match Command::new(&bin)
        .args(args)
        .current_dir(dir)
        .env("CACHE", cache)
        .env("STATE", st)
        .output()
    {
        Ok(o) if o.status.success() => None,
        Ok(o) => Some(failed(
            format!("{label} did not build: it exited with {}", o.status),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )),
        Err(e) => Some(failed(
            format!("{prog} could not be started: {e}"),
            String::new(),
        )),
    }
}

/// Runs the project once (`IOHR_EXT_ONCE=1`) with `STATE` and `CACHE` under its own folder, the
/// way `iohr ext dev --test` does, and reads the heartbeat back. No token channel is given, so
/// the project's token ask is refused: that refusal is the proof the channel is its only
/// credential.
///
/// What runs is the language's own `runner` (`$DIR`, `$STATE` and `$CACHE` filled in): its
/// `build` first, in your environment but writing only under `$CACHE`, then its `run` with
/// nothing of your environment but a fixed `PATH`, so the program is found by its full path.
/// A template without languages runs as Python: `python3 -m NAME`.
#[must_use]
pub fn smoke(dir: &Path, state: &Path, name: &str, runner: Option<&Runner>) -> Smoke {
    let st = state.join("state");
    let cache = state.join("cache");
    if private_dir(&st).and(private_dir(&cache)).is_err() {
        return failed(
            "the project's state folder could not be made".into(),
            String::new(),
        );
    }
    let legacy = legacy_runner(name);
    let runner = runner.filter(|r| !r.run.is_empty()).unwrap_or(&legacy);
    let words =
        |v: &[String]| -> Vec<String> { v.iter().map(|w| paths_in(w, dir, &st, &cache)).collect() };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let find = |prog: &str| -> Option<PathBuf> {
        if prog.contains('/') {
            Some(PathBuf::from(prog))
        } else {
            crate::ask::find_on_path(prog, &path)
        }
    };
    if let Some(fail) = build_step(
        &words(&runner.build),
        &runner.label,
        dir,
        &st,
        &cache,
        &find,
    ) {
        return fail;
    }
    let run = words(&runner.run);
    let Some((prog, args)) = run.split_first() else {
        return failed("the language has nothing to run".into(), String::new());
    };
    let Some(bin) = find(prog) else {
        return failed(
            format!("{prog} was not found on your PATH: install it, or pick another language"),
            String::new(),
        );
    };
    let mut cmd = Command::new(&bin);
    cmd.args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
        .env("HOME", &cache)
        .env("STATE", &st)
        .env("CACHE", &cache)
        .env("IOHR_EXT_ONCE", "1");
    for (k, v) in &runner.env {
        cmd.env(k, paths_in(v, dir, &st, &cache));
    }
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) => return failed(format!("{prog} could not be started: {e}"), String::new()),
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
    use crate::template::{self, Answers, Spec};

    fn rendered(id: &str, name: &str) -> Vec<(String, String)> {
        rendered_in(id, name, "python").0
    }

    /// The files and the runner of template `id` in language `lang`.
    fn rendered_in(id: &str, name: &str, lang: &str) -> (Vec<(String, String)>, Option<Runner>) {
        let t = template::load(&Spec::Id(id.into()), Path::new("/no-such-iohr-root")).unwrap();
        let mut given = Answers::new();
        if !t.meta.variants.is_empty() {
            given.insert("language".into(), template::Answer::Text(lang.into()));
        }
        let answers = template::answer(&t, &given, None).unwrap();
        let vals = template::all_values(&t, name, "local", "2026-10-12T10:00:00Z", &answers);
        let tv = t.with_variant(lang).unwrap_or_else(|_| t.clone());
        (template::render(&tv, &vals).unwrap(), t.runner(lang, &vals))
    }

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
        let files = rendered("demo", "my-shop");
        assert!(files.iter().any(|(p, _)| p == "src/my_shop/__main__.py"));
        for (p, t) in &files {
            assert!(!t.contains("@@"), "{p} keeps a placeholder");
            assert!(!p.contains("@@"), "{p}");
        }
        let (_, manifest) = files.iter().find(|(p, _)| p == SOURCE_FILE).unwrap();
        assert!(lint(manifest).ok);
        assert!(manifest.contains("id = \"local/my-shop\""));
        assert!(check_manifest(&files).is_ok());
    }

    #[test]
    fn the_coordinated_template_is_the_demo_plus_a_coordination_manifest() {
        let files = rendered("coordinated", "my-shop");
        let get = |p: &str| files.iter().find(|(f, _)| f == p).map(|(_, t)| t.as_str());
        for (p, t) in &files {
            assert!(!t.contains("@@"), "{p} keeps a placeholder");
        }
        assert!(get("src/my_shop/__main__.py").is_some());
        let v: toml::Table = get("coordination.toml").unwrap().parse().unwrap();
        assert_eq!(v["plan"]["id"].as_str(), Some(""));
        assert_eq!(v["ask"][0]["external_id"].as_str(), Some("my-shop-first"));
        assert!(
            get("inorbit-project.toml")
                .unwrap()
                .contains("template = \"coordinated\"")
        );
        let mut paths: Vec<&String> = files.iter().map(|(p, _)| p).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), files.len());
        let demo = rendered("demo", "my-shop");
        assert_eq!(demo.len() + 1, files.len());
    }

    #[test]
    fn the_template_is_the_flag_then_the_config_then_demo() {
        assert_eq!(choose_template(Some("demo"), Some("coordinated")), "demo");
        assert_eq!(choose_template(None, Some("coordinated")), "coordinated");
        assert_eq!(choose_template(None, None), "demo");
        assert_eq!(choose_template(Some(" "), Some(" ")), "demo");
    }

    #[test]
    fn the_steps_write_the_project_and_its_state_and_refuse_a_busy_folder() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("development/demo");
        let st = d.path().join("projects");
        let files = rendered("demo", "demo");
        let (written, s) = scaffold(&dir, &files, &[], "demo").unwrap();
        assert_eq!(s.id, "scaffold");
        assert_eq!(written.len(), files.len());
        assert!(dir.join("src/demo/__main__.py").is_file());
        let (state, r) = register_step(&dir, &st, "demo", "demo", "2026-10-12T10:00:00Z").unwrap();
        assert_eq!(r.id, "runtime");
        assert!(state.join("state").is_dir() && state.join("cache").is_dir());
        assert!(st.join("demo/project.toml").is_file());
        assert!(
            cloud_steps("demo")
                .iter()
                .all(|s| s.state != "not-yet" || !s.needs.is_empty())
        );
        assert!(
            scaffold(&dir, &files, &[], "demo")
                .unwrap_err()
                .contains("not empty")
        );
    }

    #[test]
    fn the_agent_prompt_holds_the_template_words_the_answers_and_the_description() {
        let mut a = Answers::new();
        a.insert("flavour".into(), template::Answer::Text("fancy".into()));
        let vals = template::values("my-shop", "local", "", &a);
        let p = agent_prompt(
            Some("Module @@MODULE@@."),
            Some("sell hats"),
            "my-shop",
            "demo",
            &a,
            &vals,
        );
        assert!(p.starts_with("Module my_shop."));
        assert!(p.contains("- flavour: fancy") && p.contains("sell hats"));
    }

    /// The program a variant needs, on this machine's PATH?
    fn have(program: &str) -> bool {
        crate::ask::find_on_path(program, &std::env::var_os("PATH").unwrap_or_default()).is_some()
    }

    #[test]
    fn the_smoke_test_runs_the_project_once_and_reads_one_heartbeat() {
        if !have("python3") {
            eprintln!("no python3 here: the smoke test is not run");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("development/my-shop");
        let st = d.path().join("projects");
        let (files, runner) = rendered_in("demo", "my-shop", "python");
        scaffold(&dir, &files, &[], "demo").unwrap();
        let (state, _) = register_step(&dir, &st, "my-shop", "demo", "").unwrap();
        let s = smoke(&dir, &state, "my-shop", runner.as_ref());
        assert!(s.ok, "{} / {}", s.said, s.output);
        let b = s.heartbeat.unwrap();
        assert_eq!(b["id"], "local/my-shop");
        assert_eq!(b["beats"], 1);
        assert_eq!(b["token"], "refused");
        assert!(!dir.join("heartbeat.json").exists());
        assert!(!dir.join("src/my_shop/__pycache__").exists());
    }

    /// Every live language of the demo: its files render, its manifest passes the lint, its
    /// smoke test reads one heartbeat, and its own self-test passes. Skipped (said on stderr)
    /// when the toolchain is not on this machine.
    #[test]
    fn every_live_demo_language_renders_runs_and_passes_its_own_test() {
        let tpl =
            template::load(&Spec::Id("demo".into()), Path::new("/no-such-iohr-root")).unwrap();
        for v in tpl.live_variants() {
            let missing: Vec<&String> = v.needs.iter().filter(|n| !have(n)).collect();
            if !missing.is_empty() {
                eprintln!("{}: {missing:?} not on this PATH: not run", v.id);
                continue;
            }
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("my-shop");
            let (files, runner) = rendered_in("demo", "my-shop", &v.id);
            assert!(check_manifest(&files).is_ok(), "{}", v.id);
            for (p, x) in &files {
                assert!(!p.contains("@@") && !x.contains("@@"), "{}: {p}", v.id);
            }
            let tv = tpl.with_variant(&v.id).unwrap();
            let programs = template::programs(&tv, &[]);
            scaffold(&dir, &files, &programs, "demo").unwrap();
            let st = tmp.path().join("projects");
            let (state, _) = register_step(&dir, &st, "my-shop", "demo", "").unwrap();
            let said = smoke(&dir, &state, "my-shop", runner.as_ref());
            assert!(said.ok, "{}: {} / {}", v.id, said.said, said.output);
            let b = said.heartbeat.unwrap();
            assert_eq!(b["id"], "local/my-shop", "{}", v.id);
            assert_eq!(b["beats"], 1, "{}", v.id);
            assert_eq!(b["token"], "refused", "{}", v.id);
            // The project's own test, the way its README says.
            let test = &v.test;
            let (prog, args) = test.split_first().unwrap();
            std::fs::create_dir_all(tmp.path().join("tstate")).unwrap();
            std::fs::create_dir_all(tmp.path().join("tcache")).unwrap();
            let out = Command::new(prog)
                .args(args)
                .current_dir(&dir)
                .env("CARGO_TARGET_DIR", tmp.path().join("tt"))
                .env("STATE", tmp.path().join("tstate"))
                .env("CACHE", tmp.path().join("tcache"))
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("PYTHONPATH", dir.join("src"))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{} self-test: {}{}",
                v.id,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
