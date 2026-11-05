//! `iohr project init`, its `create` alias and `templates`: the binary with a private HOME
//! and config folder, so nothing of the machine running the test is read or written.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::path::Path;
use std::process::{Command, Output, Stdio};

fn iohr(home: &Path, args: &[&str]) -> Output {
    let path = std::env::var_os("PATH").unwrap_or_default();
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(args)
        .env_clear()
        .env("PATH", path)
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
            .contains("(default)")
    );

    let out = iohr(h.path(), &["project", "templates", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["name"], "demo");
    assert_eq!(rows[1]["status"], "live");
    assert_eq!(rows[2]["status"], "coming");
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
    let t = home.join(".inorbit/templates/probe");
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
    assert!(d.join("extension.toml").is_file());
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
    for id in ["demo", "coordinated", "probe"] {
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
