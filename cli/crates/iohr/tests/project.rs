//! `iohr project init`, its `create` alias and `template list|show|path`: the binary with a private HOME
//! and config folder, so nothing of the machine running the test is read or written.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use support::{canon, kept_env};

fn iohr(home: &Path, args: &[&str]) -> Output {
    iohr_in(home, home, args)
}

/// The same, run from `cwd`.
fn iohr_in(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    let path = std::env::var_os("PATH").unwrap_or_default();
    // The run's HOME is a temp folder, so the toolchain manager keeps pointing at the real one
    // (the smoke test runs the language's own test command).
    let real = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_iohr"));
    cmd.env_clear();
    for (var, dir) in [("RUSTUP_HOME", ".rustup"), ("CARGO_HOME", ".cargo")] {
        let set = std::env::var_os(var).map(std::path::PathBuf::from);
        if let Some(p) = set.or_else(|| real.as_ref().map(|h| h.join(dir))) {
            cmd.env(var, p);
        }
    }
    cmd.current_dir(cwd)
        .args(args)
        .envs(kept_env())
        .env("PATH", path)
        // The home directory is HOME on unix and USERPROFILE on Windows.
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn config(home: &Path, text: &str) {
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(home.join("config/config.toml"), text).unwrap();
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn create_is_init() {
    let h = tempfile::tempdir().unwrap();
    let dir = h.path().join("work/made");
    let out = iohr(
        h.path(),
        &[
            "project",
            "create",
            "made",
            "--dir",
            dir.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(dir.join("extension.toml").is_file());
    assert!(
        !dir.join("coordination.toml").exists(),
        "the built-in default is demo"
    );
    assert!(
        h.path()
            .join(".inorbit/projects/made/project.toml")
            .is_file()
    );
    let help = iohr(h.path(), &["project", "--help"]);
    assert!(text(&help).contains("create"), "{}", text(&help));
}

#[test]
fn templates_lists_every_template_as_a_table_and_as_json() {
    let h = tempfile::tempdir().unwrap();
    let out = iohr(h.path(), &["project", "templates"]);
    assert!(out.status.success(), "{}", text(&out));
    let t = text(&out);
    assert!(t.starts_with("NAME"), "{t}");
    for (name, status) in [
        ("blank", "live"),
        ("demo", "live"),
        ("coordinated", "live"),
        ("macos-app", "coming"),
        ("ui-panel", "coming"),
    ] {
        let row = t.lines().find(|l| l.starts_with(name)).unwrap_or_default();
        assert!(row.contains(status), "{name}: {t}");
    }
    assert!(
        t.lines()
            .find(|l| l.starts_with("demo"))
            .unwrap()
            .contains('*')
    );
    assert!(!t.contains("(default)"), "the default is the * column: {t}");

    let out = iohr(h.path(), &["project", "templates", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 5);
    // `blank` is listed first, after the menu's "Describe it"; demo keeps its own description.
    assert_eq!(rows[0]["name"], "blank");
    assert_eq!(rows[0]["description"], "An empty project in your language");
    assert_eq!(rows[1]["name"], "demo");
    assert_eq!(rows[2]["status"], "live");
    assert_eq!(rows[3]["status"], "coming");
}

#[test]
fn the_flag_beats_the_config_which_beats_demo() {
    let h = tempfile::tempdir().unwrap();
    config(h.path(), "[project]\ntemplate = \"coordinated\"\n");

    // The config's template when there is no flag.
    let first = h.path().join("a");
    let out = iohr(
        h.path(),
        &[
            "project",
            "create",
            "proj-a",
            "--dir",
            first.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(first.join("coordination.toml").is_file());

    // The flag over the config.
    let second = h.path().join("b");
    let out = iohr(
        h.path(),
        &[
            "project",
            "create",
            "proj-b",
            "--template",
            "demo",
            "--dir",
            second.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(!second.join("coordination.toml").exists());

    // The listing marks the configured default.
    let out = iohr(h.path(), &["project", "templates", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let default: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["default"] == true)
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(default, ["coordinated"]);
}

#[test]
fn a_template_that_is_coming_still_refuses_and_says_what_is_live() {
    let h = tempfile::tempdir().unwrap();
    let d = h.path().join("c");
    let out = iohr(
        h.path(),
        &[
            "project",
            "create",
            "proj-c",
            "--template",
            "ui-panel",
            "--dir",
            d.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    assert!(!out.status.success());
    let t = text(&out);
    assert!(t.contains("coming") && t.contains("`coordinated`"), "{t}");
    assert!(!d.exists());
    config(h.path(), "[project]\ntemplate = \"macos-app\"\n");
    let out = iohr(
        h.path(),
        &[
            "project",
            "create",
            "proj-d",
            "--dir",
            d.to_str().unwrap(),
            "--no-smoke",
        ],
    );
    assert!(
        !out.status.success(),
        "a coming template in the config refuses too"
    );
    assert!(!d.exists());
}

// ---- templates as data ----------------------------------------------------------------------

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A user template with a choice, a `when` question, a templated path and an agent step.
fn user_template(home: &Path) -> std::path::PathBuf {
    let t = home.join(".inorbit").join("templates").join("probe");
    write(
        &t.join("template.toml"),
        r#"id = "probe"
description = "a probe"
status = "live"
extends = "demo"
agent_prompt = "Make @@NAME@@ do @@KIND@@."

[[questions]]
key = "kind"
prompt = "Kind?"
type = "choice"
choices = ["alpha", "beta"]
default = "alpha"

[[questions]]
key = "extra"
prompt = "Extra?"
type = "string"
default = "none"
when = "kind == beta"

[[steps]]
name = "git"

[[steps]]
name = "agent"
"#,
    );
    write(
        &t.join("notes-@@KIND@@.md"),
        "kind=@@KIND@@ extra=@@EXTRA@@ for @@NAME@@\n",
    );
    t
}

#[test]
#[allow(clippy::too_many_lines, reason = "one scenario, several invocations")]
fn a_user_template_answers_set_answers_yes_and_when() {
    let h = tempfile::tempdir().unwrap();
    user_template(h.path());
    let list = text(&iohr(h.path(), &["project", "templates"]));
    assert!(
        list.contains("probe") && list.contains("ORIGIN") && list.contains("user"),
        "{list}"
    );

    // --set picks beta, so the `when` question is asked (here answered by --set too).
    let d1 = h.path().join("w/one");
    let o = iohr(
        h.path(),
        &[
            "project",
            "init",
            "one",
            "--template",
            "probe",
            "--set",
            "kind=beta",
            "--set",
            "extra=hi",
            "--dir",
            d1.to_str().unwrap(),
            "--no-smoke",
            "--yes",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(
        std::fs::read_to_string(d1.join("notes-beta.md")).unwrap(),
        "kind=beta extra=hi for one\n"
    );
    assert!(d1.join("extension.toml").is_file(), "extends demo");

    // --answers file, and a skipped `when` question stays empty.
    let f = h.path().join("a.json");
    std::fs::write(&f, r#"{"kind":"alpha","extra":"ignored"}"#).unwrap();
    let d2 = h.path().join("w/two");
    let o = iohr(
        h.path(),
        &[
            "project",
            "init",
            "two",
            "--template",
            "probe",
            "--answers",
            f.to_str().unwrap(),
            "--dir",
            d2.to_str().unwrap(),
            "--no-smoke",
            "--yes",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(
        std::fs::read_to_string(d2.join("notes-alpha.md")).unwrap(),
        "kind=alpha extra= for two\n"
    );

    // --yes takes defaults; a bad choice and an unknown key are refused.
    let d3 = h.path().join("w/three");
    let o = iohr(
        h.path(),
        &[
            "project",
            "init",
            "three",
            "--template",
            "probe",
            "--dir",
            d3.to_str().unwrap(),
            "--no-smoke",
            "--yes",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(d3.join("notes-alpha.md").is_file());
    let d4 = h.path().join("w/four");
    let o = iohr(
        h.path(),
        &[
            "project",
            "init",
            "four",
            "--template",
            "probe",
            "--set",
            "kind=gamma",
            "--dir",
            d4.to_str().unwrap(),
            "--yes",
        ],
    );
    assert!(!o.status.success() && !d4.exists(), "{}", text(&o));
    let o = iohr(
        h.path(),
        &[
            "project",
            "init",
            "four",
            "--template",
            "probe",
            "--set",
            "nope=1",
            "--dir",
            d4.to_str().unwrap(),
            "--yes",
        ],
    );
    assert!(!o.status.success() && !d4.exists(), "{}", text(&o));
}

#[test]
fn schema_is_json_and_lint_catches_a_bad_template() {
    let h = tempfile::tempdir().unwrap();
    let o = iohr(h.path(), &["project", "template", "schema"]);
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(v.to_string().contains("questions"), "{v}");

    let o = iohr(h.path(), &["project", "template", "new", "fresh"]);
    assert!(o.status.success(), "{}", text(&o));
    let made = h.path().join(".inorbit/templates/fresh");
    assert!(made.join("template.toml").is_file());
    let o = iohr(
        h.path(),
        &["project", "template", "lint", made.to_str().unwrap()],
    );
    assert!(
        o.status.success(),
        "a new template lints clean: {}",
        text(&o)
    );
    let o = iohr(h.path(), &["project", "template", "new", "fresh"]);
    assert!(!o.status.success(), "an existing folder is refused");

    let bad = h.path().join("bad");
    write(
        &bad.join("template.toml"),
        "id = \"bad\"\ndescription = \"x\"\n\n[[questions]]\nkey = \"A\"\nprompt = \"A?\"\ntype = \"choice\"\n\n[[questions]]\nkey = \"B\"\nprompt = \"B?\"\ntype = \"string\"\nwhen = \"C == 1\"\n",
    );
    write(&bad.join("f.txt"), "@@MISSING@@\n");
    let o = iohr(
        h.path(),
        &["project", "template", "lint", bad.to_str().unwrap()],
    );
    assert!(!o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("choice") || t.contains("choices"), "{t}");
    assert!(t.contains('C') && t.contains("MISSING"), "{t}");
}

#[test]
fn a_template_from_a_path_beats_the_user_folder() {
    let h = tempfile::tempdir().unwrap();
    user_template(h.path());
    let p = h.path().join("mine");
    write(
        &p.join("template.toml"),
        "id = \"probe\"\ndescription = \"from path\"\nstatus = \"live\"\nextends = \"demo\"\n",
    );
    write(&p.join("PATHMARK.txt"), "@@NAME@@\n");
    let d = h.path().join("w/pp");
    let o = iohr(
        h.path(),
        &[
            "project",
            "init",
            "pp",
            "--template",
            p.to_str().unwrap(),
            "--dir",
            d.to_str().unwrap(),
            "--no-smoke",
            "--yes",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(d.join("PATHMARK.txt").is_file());
}

/// A `claude` that records it was called, so a test can tell if an agent ever started.
#[cfg(unix)]
fn fake_claude(dir: &Path, marker: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join("claude");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\necho \"$PWD|$*\" > '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn iohr_with_path(home: &Path, extra: &Path, args: &[&str]) -> Output {
    let base = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![extra.to_path_buf()];
    paths.extend(std::env::split_paths(&base));
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(args)
        .env_clear()
        .envs(kept_env())
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[cfg(unix)]
#[test]
fn describe_starts_the_detected_agent_in_the_new_folder_with_the_ai_label() {
    let h = tempfile::tempdir().unwrap();
    let marker = h.path().join("called");
    fake_claude(&h.path().join("bin"), &marker);
    let d = h.path().join("w/ai");
    let o = iohr_with_path(
        h.path(),
        &h.path().join("bin"),
        &[
            "project",
            "init",
            "ai",
            "--describe",
            "a sensor watcher",
            "--dir",
            d.to_str().unwrap(),
            "--no-smoke",
            "--yes",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("AI"), "{}", text(&o));
    let called = std::fs::read_to_string(&marker).unwrap();
    assert!(
        called.contains("a sensor watcher") && called.contains("ai"),
        "{called}"
    );
    // Described projects are plain: `blank`, no panel and no manifest.
    assert!(d.join("inorbit.toml").is_file() && d.join(".inorbit/space").is_file());
    assert!(!d.join("extension.toml").exists() && !d.join("panel").exists());
    let rec = std::fs::read_to_string(d.join("inorbit.toml")).unwrap();
    assert!(
        rec.contains("template = \"blank\"") && rec.contains("kind = \"project\""),
        "{rec}"
    );
    let readme = std::fs::read_to_string(d.join("README.md")).unwrap();
    assert!(readme.contains("a sensor watcher"), "{readme}");
    // No agent anywhere: it says how to configure one.
    let o = iohr(
        h.path(),
        &[
            "project",
            "init",
            "noai",
            "--describe",
            "x",
            "--dir",
            h.path().join("w/noai").to_str().unwrap(),
            "--yes",
        ],
    );
    if !text(&o).contains("[ask]") {
        // a machine with its own claude on PATH: nothing to assert about the missing case
        assert!(
            o.status.success() || text(&o).contains("AI"),
            "{}",
            text(&o)
        );
    }
}

#[cfg(unix)]
#[test]
fn dry_run_changes_nothing_on_every_built_in_and_never_starts_the_agent() {
    let h = tempfile::tempdir().unwrap();
    let marker = h.path().join("called");
    fake_claude(&h.path().join("bin"), &marker);
    user_template(h.path());
    for id in ["blank", "demo", "coordinated", "probe"] {
        for flag in ["--dry-run", "--plan"] {
            let d = h.path().join("w").join(format!("{id}-x"));
            let o = iohr_with_path(
                h.path(),
                &h.path().join("bin"),
                &[
                    "project",
                    "init",
                    &format!("{id}-x"),
                    "--template",
                    id,
                    "--dir",
                    d.to_str().unwrap(),
                    flag,
                    "--yes",
                ],
            );
            let t = text(&o);
            assert!(o.status.success(), "{id}: {t}");
            assert!(
                t.contains("dry run: nothing was changed") || t.contains("would:"),
                "{id}: {t}"
            );
            assert!(
                t.contains(".inorbit/space"),
                "{id}: the dry run lists the link: {t}"
            );
            assert!(!d.exists(), "{id}: the folder must not exist");
            assert!(
                !h.path().join(".inorbit/projects").exists(),
                "{id}: state untouched"
            );
            assert!(!marker.exists(), "{id}: the agent must not start");
        }
    }
    // --json: the plan as data, with the agent's command and the full prompt.
    let d = h.path().join("w/jj");
    let o = iohr_with_path(
        h.path(),
        &h.path().join("bin"),
        &[
            "project",
            "init",
            "jj",
            "--template",
            "probe",
            "--describe",
            "do a thing",
            "--dir",
            d.to_str().unwrap(),
            "--dry-run",
            "--json",
            "--yes",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["dry_run"], true);
    assert!(
        v["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "notes-alpha.md" && f["bytes"].as_u64().unwrap() > 0),
        "{v}"
    );
    assert_ne!(v["steps"].as_array().unwrap().len(), 0);
    assert!(
        v["agent"]["prompt"]
            .as_str()
            .unwrap()
            .contains("do a thing"),
        "{v}"
    );
    assert!(!d.exists() && !marker.exists() && !h.path().join(".inorbit/projects").exists());
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(o)))
}

#[test]
fn the_dry_run_names_the_folder_under_where_you_are_and_why() {
    let h = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let cwd_path = canon(cwd.path());
    let out = iohr_in(
        h.path(),
        &cwd_path,
        &[
            "project",
            "init",
            "my-shop",
            "--template",
            "demo",
            "--dry-run",
            "--yes",
        ],
    );
    assert!(out.status.success(), "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains("would: write 35 files into ./my-shop (in the folder you are in)"),
        "{t}"
    );
    assert!(
        !t.contains(h.path().to_str().unwrap())
            || t.contains(
                &h.path()
                    .join(".inorbit")
                    .join("projects")
                    .join("my-shop")
                    .display()
                    .to_string()
            ),
        "{t}"
    );
    let j = iohr_in(
        h.path(),
        &cwd_path,
        &[
            "project",
            "init",
            "my-shop",
            "--template",
            "demo",
            "--dry-run",
            "--yes",
            "--json",
        ],
    );
    let v = json(&j);
    assert_eq!(v["folder"]["shown"], "./my-shop");
    assert_eq!(v["folder"]["kind"], "here");
    assert_eq!(
        v["folder"]["dir"],
        cwd_path.join("my-shop").to_str().unwrap()
    );
    assert_eq!(v["language"], "python");
    assert!(
        !cwd_path.join("my-shop").exists(),
        "a dry run writes nothing"
    );
}

#[test]
fn a_folder_that_is_not_empty_is_refused_and_dotfiles_alone_are_fine() {
    let h = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(cwd.path().join("my-shop")).unwrap();
    std::fs::write(cwd.path().join("my-shop/notes.txt"), "x").unwrap();
    let args = [
        "project",
        "init",
        "my-shop",
        "--template",
        "demo",
        "--yes",
        "--no-smoke",
    ];
    let out = iohr_in(h.path(), cwd.path(), &args);
    assert!(!out.status.success());
    assert!(text(&out).contains("not empty"), "{}", text(&out));
    std::fs::remove_file(cwd.path().join("my-shop/notes.txt")).unwrap();
    std::fs::write(cwd.path().join("my-shop/.DS_Store"), "x").unwrap();
    let out = iohr_in(h.path(), cwd.path(), &args);
    assert!(out.status.success(), "{}", text(&out));
    assert!(cwd.path().join("my-shop/extension.toml").is_file());
}

#[test]
fn inside_a_git_repository_the_dry_run_warns_that_it_would_nest() {
    let h = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let cwd_path = canon(cwd.path());
    let git = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&cwd_path)
        .status();
    if !git.is_ok_and(|s| s.success()) {
        return;
    }
    let out = iohr_in(
        h.path(),
        &cwd_path,
        &[
            "project",
            "init",
            "my-shop",
            "--template",
            "demo",
            "--dry-run",
            "--yes",
            "--json",
        ],
    );
    assert!(out.status.success(), "{}", text(&out));
    let v = json(&out);
    let w = v["warnings"][0].as_str().unwrap_or_default();
    assert!(w.contains("inside the git repository"), "{v}");
    assert!(w.contains("../my-shop"), "{v}");
}

#[test]
fn lang_picks_the_variant_and_a_wrong_one_is_refused() {
    let h = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    for (lang, file) in [
        ("typescript", "src/main.ts"),
        ("go", "main.go"),
        ("rust", "Cargo.toml"),
    ] {
        let out = iohr_in(
            h.path(),
            cwd.path(),
            &[
                "project",
                "init",
                "my-shop",
                "--template",
                "demo",
                "--lang",
                lang,
                "--dry-run",
                "--yes",
                "--json",
            ],
        );
        assert!(out.status.success(), "{lang}: {}", text(&out));
        let v = json(&out);
        assert_eq!(v["language"], lang);
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(files.contains(&file), "{lang}: {files:?}");
    }
    let bad = iohr_in(
        h.path(),
        cwd.path(),
        &[
            "project",
            "init",
            "my-shop",
            "--template",
            "demo",
            "--lang",
            "cobol",
            "--dry-run",
            "--yes",
        ],
    );
    assert!(!bad.status.success());
    assert!(
        text(&bad).contains("python, typescript, go, rust"),
        "{}",
        text(&bad)
    );
    let clash = iohr_in(
        h.path(),
        cwd.path(),
        &[
            "project",
            "init",
            "my-shop",
            "--template",
            "demo",
            "--lang",
            "go",
            "--set",
            "language=rust",
            "--dry-run",
            "--yes",
        ],
    );
    assert!(!clash.status.success());
    assert!(text(&clash).contains("disagree"), "{}", text(&clash));
}

fn on_path(program: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| d.join(program).is_file())
}

#[test]
fn every_live_language_scaffolds_and_its_smoke_test_passes_when_its_toolchain_is_here() {
    for (lang, program) in [
        ("python", "python3"),
        ("typescript", "node"),
        ("go", "go"),
        ("rust", "cargo"),
    ] {
        if !on_path(program) {
            continue;
        }
        let h = tempfile::tempdir().unwrap();
        // The toolchains keep their own homes under the real HOME.
        let cwd = tempfile::tempdir().unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let real = std::env::var_os("HOME").unwrap_or_default();
        let name = format!("smoke-{lang}");
        let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
        c.args([
            "project",
            "init",
            &name,
            "--template",
            "demo",
            "--lang",
            lang,
            "--yes",
        ])
        .current_dir(cwd.path())
        .env_clear()
        .envs(kept_env())
        .env("PATH", path)
        .env("HOME", h.path())
        .env("USERPROFILE", h.path())
        .env("IOHR_CONFIG_DIR", h.path().join("config"))
        .env("CARGO_HOME", Path::new(&real).join(".cargo"))
        .env("RUSTUP_HOME", Path::new(&real).join(".rustup"))
        .env("GOPATH", h.path().join("go"))
        .env("GOCACHE", h.path().join("gocache"))
        .env("GOFLAGS", "-modcacherw")
        .stdin(Stdio::null());
        let out = c.output().unwrap();
        assert!(out.status.success(), "{lang}: {}", text(&out));
        assert!(
            text(&out).contains("done: smoke test, one heartbeat"),
            "{lang}: {}",
            text(&out)
        );
        assert!(cwd.path().join(&name).join("extension.toml").is_file());
    }
}

// ---- template list, show, path: one noun --------------------------------------------------------

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn template_list_and_the_old_templates_name_print_the_same_bytes() {
    let h = tempfile::tempdir().unwrap();
    user_template(h.path());
    for extra in [&[][..], &["--json"][..]] {
        let mut new = vec!["project", "template", "list"];
        let mut old = vec!["project", "templates"];
        new.extend(extra);
        old.extend(extra);
        let (a, b) = (iohr(h.path(), &new), iohr(h.path(), &old));
        assert!(a.status.success() && b.status.success(), "{}", text(&a));
        assert_eq!(a.stdout, b.stdout, "{new:?} against {old:?}");
        assert!(a.stdout.len() > 10, "{new:?} printed nothing");
    }
}

#[test]
fn template_list_columns_and_a_piped_run_has_no_footer_or_colour() {
    let h = tempfile::tempdir().unwrap();
    user_template(h.path());
    let t = stdout(&iohr(h.path(), &["project", "template", "list"]));
    let head: Vec<&str> = t.lines().next().unwrap().split_whitespace().collect();
    assert_eq!(
        head,
        ["NAME", "STATUS", "ORIGIN", "DEFAULT", "DESCRIPTION"],
        "{t}"
    );
    let demo = t.lines().find(|l| l.starts_with("demo")).unwrap();
    assert!(demo.contains(" * "), "the default is marked: {demo}");
    assert!(!t.contains("(default)"), "{t}");
    assert!(t.contains("user: ~/.inorbit/templates/probe"), "{t}");
    assert!(!t.contains("Next:"), "a pipe gets data only: {t}");
    assert!(!t.contains('\u{1b}'), "no colour in a pipe: {t}");
    assert!(!t.contains('\u{25cf}'), "the plain status is the word: {t}");
}

#[test]
fn show_a_built_in_says_where_it_is_from_what_it_writes_and_has_no_folder() {
    let h = tempfile::tempdir().unwrap();
    let o = iohr(h.path(), &["project", "template", "show", "demo"]);
    assert!(o.status.success(), "{}", text(&o));
    let t = stdout(&o);
    for want in [
        "demo",
        "Origin",
        "built-in",
        "Questions",
        "Files",
        "extension.toml",
        "panel/index.html",
        "Languages",
    ] {
        assert!(t.contains(want), "{want}: {t}");
    }
    assert!(!t.contains("Next:"), "{t}");

    let p = iohr(h.path(), &["project", "template", "path", "demo"]);
    assert_eq!(p.status.code(), Some(1), "{}", text(&p));
    assert_eq!(stdout(&p), "", "nothing on stdout");
    assert!(
        text(&p).contains("template new NAME --extends demo"),
        "{}",
        text(&p)
    );
    let missing = iohr(h.path(), &["project", "template", "show", "nope"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        text(&missing).contains("template list"),
        "{}",
        text(&missing)
    );
}

#[test]
fn show_and_path_for_a_user_template() {
    let h = tempfile::tempdir().unwrap();
    let dir = user_template(h.path());
    let o = iohr(h.path(), &["project", "template", "show", "probe"]);
    assert!(o.status.success(), "{}", text(&o));
    let t = stdout(&o);
    for want in [
        "Extends",
        "demo",
        "kind",
        "choice (alpha, beta)",
        "\"alpha\"",
        "kind == beta",
        "notes-@@KIND@@.md",
        "Agent step",
        "Make @@NAME@@ do @@KIND@@.",
    ] {
        assert!(t.contains(want), "{want}: {t}");
    }
    assert!(t.contains(dir.to_str().unwrap()), "{t}");

    let p = iohr(h.path(), &["project", "template", "path", "probe"]);
    assert!(p.status.success(), "{}", text(&p));
    assert_eq!(
        stdout(&p),
        format!("{}\n", dir.display()),
        "only the folder"
    );
    let by_folder = iohr(
        h.path(),
        &["project", "template", "path", dir.to_str().unwrap()],
    );
    assert_eq!(stdout(&by_folder), stdout(&p));
}

#[test]
fn show_json_carries_everything() {
    let home = tempfile::tempdir().unwrap();
    let dir = user_template(home.path());
    let shown = iohr(
        home.path(),
        &["project", "template", "show", "probe", "--json"],
    );
    assert!(shown.status.success(), "{}", text(&shown));
    let got: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(got["id"], "probe");
    assert_eq!(got["status"], "live");
    assert_eq!(got["origin"]["kind"], "user");
    assert_eq!(got["path"], dir.to_str().unwrap());
    assert_eq!(got["extends"], "demo");
    assert_eq!(got["default"], false);
    assert_eq!(got["steps"], serde_json::json!(["git", "agent"]));
    assert_eq!(got["agent"]["step"], true);
    assert_eq!(
        got["agent"]["describe_prompt"],
        "Make @@NAME@@ do @@KIND@@."
    );
    let q = got["questions"].as_array().unwrap();
    assert_eq!(q[0]["key"], "kind");
    assert_eq!(q[0]["type"], "choice");
    assert_eq!(q[0]["choices"], serde_json::json!(["alpha", "beta"]));
    assert_eq!(q[0]["default"], "alpha");
    assert_eq!(q[1]["when"], "kind == beta");
    assert!(
        got["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "notes-@@KIND@@.md")
    );

    let built_in = iohr(
        home.path(),
        &["project", "template", "show", "demo", "--json"],
    );
    let built_in: serde_json::Value = serde_json::from_slice(&built_in.stdout).unwrap();
    assert_eq!(built_in["origin"]["kind"], "built-in");
    assert!(built_in["path"].is_null());
    assert_eq!(built_in["default"], true);
}

#[test]
fn lint_with_no_path_checks_this_folder() {
    let h = tempfile::tempdir().unwrap();
    let made = iohr(h.path(), &["project", "template", "new", "here"]);
    assert!(made.status.success(), "{}", text(&made));
    let dir = h.path().join(".inorbit/templates/here");
    let o = iohr_in(h.path(), &dir, &["project", "template", "lint"]);
    assert!(o.status.success(), "{}", text(&o));
    let o = iohr_in(h.path(), h.path(), &["project", "template", "lint"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "no template.toml here: {}",
        text(&o)
    );
}

#[test]
fn show_lists_the_languages_a_template_offers_in_text_and_in_json() {
    let h = tempfile::tempdir().unwrap();
    let t = stdout(&iohr(h.path(), &["project", "template", "show", "demo"]));
    assert!(t.contains("\nLanguages\n"), "{t}");
    let go = t
        .lines()
        .find(|l| l.trim_start().starts_with("go "))
        .unwrap();
    assert!(go.contains("live") && go.contains("Go 1.26"), "{go}");
    let py = t
        .lines()
        .find(|l| l.trim_start().starts_with("python "))
        .unwrap();
    assert!(
        py.contains("default"),
        "the default language is marked: {py}"
    );
    assert!(!t.contains('\u{1b}'), "no colour in a pipe: {t}");

    let j = iohr(h.path(), &["project", "template", "show", "demo", "--json"]);
    let got = json(&j);
    let langs = got["languages"].as_array().unwrap();
    let ids: Vec<&str> = langs.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["python", "typescript", "go", "rust"]);
    assert_eq!(langs[0]["default"], true);
    assert_eq!(langs[2]["needs"], serde_json::json!(["go"]));
    assert!(langs[0]["ready"].is_boolean());
    let files: Vec<&str> = langs[0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(files.contains(&"src/@@MODULE@@/__main__.py"), "{files:?}");

    // A template without variants has no Languages section.
    let c = stdout(&iohr(
        h.path(),
        &["project", "template", "show", "macos-app"],
    ));
    assert!(!c.contains("Languages"), "{c}");
}

// ---- --describe: the agent builds in the background, then the smoke test runs -------------------

/// A fake `claude` for `init --describe`: records its folder and arguments in `$FAKE_OUT`,
/// writes `built.txt`, then by `$FAKE_MODE`: `break` removes `src/` (so only a smoke test that
/// runs after it can tell), `hang` waits 30 s. It prints the recorded stream `$FAKE_STREAM` and
/// exits with `$FAKE_EXIT`.
#[cfg(unix)]
fn fake_builder(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join("claude");
    std::fs::write(
        &bin,
        "#!/bin/sh\npwd > \"$FAKE_OUT/cwd\"\nprintf '%s\\n' \"$@\" > \"$FAKE_OUT/args\"\necho built > built.txt\ncase \"$FAKE_MODE\" in\n  break) rm -rf src;;\n  hang) exec sleep 30;;\nesac\nif [ -n \"$FAKE_STREAM\" ]; then while IFS= read -r l; do printf '%s\\n' \"$l\"; done < \"$FAKE_STREAM\"; fi\nexit \"${FAKE_EXIT:-0}\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `iohr project init NAME --describe ... --yes` in `h` with the fake builder first on PATH,
/// the recorded stream `stream` and `mode`, `exit`; returns the output and the project folder.
#[cfg(unix)]
fn init_described(
    h: &Path,
    extra: &[&str],
    stream: &str,
    mode: &str,
    exit: &str,
) -> (Output, std::path::PathBuf) {
    fake_builder(&h.join("bin"));
    std::fs::create_dir_all(h.join("out")).unwrap();
    let d = h.join("w/bg");
    let base = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![h.join("bin")];
    paths.extend(std::env::split_paths(&base));
    let o = Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args([
            "project",
            "init",
            "bg",
            "--describe",
            "a watcher that logs a line",
            "--lang",
            "python",
            "--dir",
            d.to_str().unwrap(),
            "--yes",
        ])
        .args(extra)
        .env_clear()
        .envs(kept_env())
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HOME", h)
        .env("USERPROFILE", h)
        .env("IOHR_CONFIG_DIR", h.join("config"))
        .env("FAKE_OUT", h.join("out"))
        .env(
            "FAKE_STREAM",
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data")
                .join(stream),
        )
        .env("FAKE_MODE", mode)
        .env("FAKE_EXIT", exit)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (o, d)
}

#[cfg(unix)]
/// The scaffold's commit, then the agent's work as its own AI-labelled commit, tree clean.
fn assert_two_commits_the_second_ai_built(d: &Path) {
    let git = |a: &[&str]| {
        String::from_utf8_lossy(
            &std::process::Command::new("git")
                .arg("-C")
                .arg(d)
                .args(a)
                .output()
                .unwrap()
                .stdout,
        )
        .into_owned()
    };
    let log = git(&["log", "--format=%s"]);
    assert_eq!(log.lines().count(), 2, "{log}");
    assert!(
        log.lines()
            .next()
            .unwrap()
            .contains("AI-built changes by claude"),
        "{log}"
    );
    assert!(
        git(&["status", "--porcelain"]).is_empty(),
        "the tree is committed"
    );
}

#[test]
fn describe_runs_the_agent_in_print_mode_in_the_background_then_the_smoke_test() {
    if !on_path("python3") {
        return;
    }
    let h = tempfile::tempdir().unwrap();
    let (o, d) = init_described(h.path(), &[], "ask-claude.jsonl", "", "0");
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    // It streamed and exited: tool rows and the footer on stderr, no session, no resume line.
    assert!(t.contains("- Read ") && t.contains("AI-built"), "{t}");
    assert!(
        !t.to_lowercase().contains("resume") && !t.contains("--resume"),
        "{t}"
    );
    assert!(t.contains("generated by an AI model (claude)"), "{t}");
    // Print mode, in the new folder, files only, bounded; the prompt is the last argument.
    let args: Vec<String> = std::fs::read_to_string(h.path().join("out/args"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let has = |a: &str, b: &str| args.windows(2).any(|w| w[0] == a && w[1] == b);
    assert!(args.contains(&"-p".to_owned()), "{args:?}");
    assert!(has("--output-format", "stream-json"), "{args:?}");
    assert!(has("--permission-mode", "acceptEdits"), "{args:?}");
    // The language's own commands are allowed by prefix, the web is allowed, and the rest of the
    // shell is not (no blanket `Bash` denial, which would beat the scoped allows); secrets stay
    // unreadable.
    assert!(!has("--disallowedTools", "Bash"), "{args:?}");
    assert!(
        args.contains(&"Bash(python3 -m unittest:*)".to_owned()),
        "{args:?}"
    );
    assert!(
        args.contains(&"WebSearch".to_owned()) && args.contains(&"WebFetch".to_owned()),
        "{args:?}"
    );
    assert!(args.contains(&"Read(./.env)".to_owned()), "{args:?}");
    assert!(
        has("--max-turns", "40") && has("--model", "haiku"),
        "{args:?}"
    );
    // The prompt is several lines (the file holds one argument line by line): the description
    // is in it, and the last line tells the model today's date from the clock.
    assert!(
        args.iter()
            .any(|l| l.contains("a watcher that logs a line")),
        "{args:?}"
    );
    let today = args
        .iter()
        .find(|l| l.starts_with("- Today is "))
        .unwrap_or_else(|| panic!("no dated line: {args:?}"));
    assert!(
        today.contains("Use the newest edition or version published as of today;")
            && today.contains("if you are unsure a newer one exists, say so."),
        "{today}"
    );
    assert!(
        today.as_bytes()[11..15].iter().all(u8::is_ascii_digit),
        "{today}"
    );
    assert!(&today[11..21] >= "2026-11-05", "{today}");
    assert!(
        args.iter()
            .any(|l| l.starts_with("- Installed toolchain: Python ")),
        "{args:?}"
    );
    assert!(
        args.iter().any(|l| l.contains("upstream repository")),
        "{args:?}"
    );
    let cwd = std::fs::read_to_string(h.path().join("out/cwd")).unwrap();
    assert_eq!(
        std::fs::canonicalize(cwd.trim()).unwrap(),
        std::fs::canonicalize(&d).unwrap()
    );
    assert!(d.join("built.txt").is_file());
    assert_two_commits_the_second_ai_built(&d);
    assert!(t.contains("done: the agent's work committed"), "{t}");
    // The smoke test ran on what the agent left, and the report says so.
    assert!(t.contains("done: smoke test, `python3 -m unittest"), "{t}");
    assert!(t.contains("done: agent build"), "{t}");
    // The same run with --json: the agent's events as lines, then one summary and the report.
    let h2 = tempfile::tempdir().unwrap();
    let (o, _) = init_described(h2.path(), &["--json"], "ask-claude.jsonl", "", "0");
    assert!(o.status.success(), "{}", text(&o));
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect();
    let kind = |i: usize| lines[i]["type"].as_str().unwrap_or_default().to_owned();
    assert_eq!(kind(lines.len() - 2), "iohr_summary", "{lines:?}");
    assert_eq!(kind(lines.len() - 1), "iohr_report", "{lines:?}");
    assert_eq!(lines[lines.len() - 1]["smoke"]["ok"], true);
}

#[cfg(unix)]
#[test]
fn the_smoke_test_runs_after_the_agent_so_the_report_is_true_for_what_it_built() {
    if !on_path("python3") {
        return;
    }
    let h = tempfile::tempdir().unwrap();
    // The agent removes src/: a smoke test that ran before it would have passed.
    let (o, d) = init_described(h.path(), &[], "ask-claude.jsonl", "break", "0");
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(t.contains("failed: smoke test"), "{t}");
    assert!(t.contains("the smoke test failed"), "{t}");
    assert!(d.join("built.txt").is_file() && !d.join("src").exists());
}

#[cfg(unix)]
#[test]
fn a_failing_or_stuck_agent_keeps_the_scaffold_exits_non_zero_and_says_how_to_retry() {
    let h = tempfile::tempdir().unwrap();
    let (o, d) = init_described(
        h.path(),
        &["--no-smoke"],
        "ask-claude-failed.jsonl",
        "",
        "1",
    );
    let t = text(&o);
    assert_eq!(o.status.code(), Some(1), "{t}");
    assert!(
        t.contains("the agent did not finish: Not logged in")
            && t.contains("iohr ask --allow-edits"),
        "{t}"
    );
    assert!(d.join("inorbit.toml").is_file(), "the scaffold stays");
    assert!(t.contains("failed: agent build"), "{t}");
    // An agent that never ends is stopped at the time bound.
    let h = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let (o, d) = init_described(
        h.path(),
        &["--no-smoke", "--agent-timeout", "1"],
        "ask-claude.jsonl",
        "hang",
        "0",
    );
    let t = text(&o);
    assert!(started.elapsed().as_secs() < 20, "{t}");
    assert_eq!(o.status.code(), Some(1), "{t}");
    assert!(t.contains("did not finish in 1 s and was stopped"), "{t}");
    assert!(d.join("inorbit.toml").is_file());
    // The dry run starts nothing and shows the command it would run, bounds included.
    let h = tempfile::tempdir().unwrap();
    let (o, d) = init_described(h.path(), &["--dry-run", "--agent-turns", "9"], "", "", "0");
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        t.contains("claude -p --output-format stream-json")
            && t.contains("--max-turns 9")
            && t.contains("--permission-mode acceptEdits"),
        "{t}"
    );
    assert!(!d.exists() && !h.path().join("out/args").exists());
}

/// Every template's init writes `.inorbit/space` (RFC 0103), committed with the first commit,
/// and no template's ignore file excludes `.inorbit/`.
#[test]
fn every_template_writes_the_space_link_and_git_commits_it() {
    let h = tempfile::tempdir().unwrap();
    for id in ["blank", "demo", "coordinated"] {
        let d = h.path().join("w").join(id);
        let o = iohr(
            h.path(),
            &[
                "project",
                "init",
                &format!("{id}-x"),
                "--template",
                id,
                "--dir",
                d.to_str().unwrap(),
                "--no-smoke",
                "--yes",
            ],
        );
        assert!(o.status.success(), "{id}: {}", text(&o));
        let link = std::fs::read_to_string(d.join(".inorbit/space")).unwrap();
        let v: toml::Table = link.parse().unwrap();
        assert_eq!(v["id"].as_str(), Some(""), "{id}");
        assert!(link.contains("/inorbit:link <space>"), "{id}: {link}");
        let ignore = std::fs::read_to_string(d.join(".gitignore")).unwrap();
        let ignored = ignore
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .any(|l| l.contains(".inorbit"));
        assert!(!ignored, "{id}: {ignore}");
        if Command::new("git").arg("--version").output().is_ok() {
            let tracked = Command::new("git")
                .args(["-C", d.to_str().unwrap(), "ls-files"])
                .output()
                .unwrap();
            let files = String::from_utf8_lossy(&tracked.stdout).into_owned();
            assert!(files.contains(".inorbit/space"), "{id}: {files}");
            assert!(files.contains("inorbit.toml"), "{id}: {files}");
        }
    }
}

/// Every live language of `blank` passes its own test through `init`'s smoke step, where the
/// toolchain is here, and the project holds the blank files only.
#[test]
fn every_blank_language_passes_the_smoke_test_through_init() {
    let h = tempfile::tempdir().unwrap();
    for (lang, prog) in [
        ("python", "python3"),
        ("typescript", "node"),
        ("go", "go"),
        ("rust", "cargo"),
    ] {
        if !on_path(prog) {
            eprintln!("{lang}: {prog} not on PATH: not run");
            continue;
        }
        let d = h.path().join("w").join(lang);
        let o = iohr(
            h.path(),
            &[
                "project",
                "init",
                "plain",
                "--template",
                "blank",
                "--lang",
                lang,
                "--dir",
                d.to_str().unwrap(),
                "--yes",
            ],
        );
        let t = text(&o);
        assert!(o.status.success(), "{lang}: {t}");
        assert!(t.contains("done: smoke test"), "{lang}: {t}");
        assert!(
            !d.join("extension.toml").exists() && !d.join("panel").exists(),
            "{lang}"
        );
        assert!(!t.to_lowercase().contains("macos"), "{lang}: {t}");
    }
}
