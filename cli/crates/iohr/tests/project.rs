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
        .env("IOHR_NO_BROWSER", "1")
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
