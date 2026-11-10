//! Where `iohr project init` keeps a project's data, and its shape question (PRD 0210): the
//! binary with a private HOME, so nothing of the machine running the test is read or written.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use support::kept_env;

fn iohr(home: &Path, vars: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_iohr"));
    cmd.env_clear()
        .current_dir(home)
        .args(args)
        .envs(kept_env())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .envs(vars.iter().copied())
        .stdin(Stdio::null());
    cmd.output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A blank project at `<home>/w/<name>`, with `extra` flags and `vars` in the environment.
fn init(
    home: &Path,
    name: &str,
    vars: &[(&str, &str)],
    extra: &[&str],
) -> (Output, std::path::PathBuf) {
    let dir = home.join("w").join(name);
    let mut args = vec![
        "project",
        "init",
        name,
        "--template",
        "blank",
        "--dir",
        dir.to_str().unwrap(),
        "--no-smoke",
        "--no-plan",
        "--no-crawl",
        "--yes",
    ];
    args.extend_from_slice(extra);
    (iohr(home, vars, &args), dir)
}

fn recorded(dir: &Path, key: &str) -> Option<String> {
    let t = std::fs::read_to_string(dir.join("inorbit.toml")).ok()?;
    t.parse::<toml::Table>()
        .ok()?
        .get("project")?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

fn files_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, at: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(at) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

#[test]
fn nothing_recorded_keeps_the_layout_every_existing_project_has() {
    let h = tempfile::tempdir().unwrap();
    let (o, dir) = init(h.path(), "old", &[], &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(recorded(&dir, "data"), None, "{}", text(&o));
    // The notes stay in the project, the state in the home folder.
    assert!(dir.join(".inorbit/memory").is_dir(), "{}", text(&o));
    assert!(
        h.path().join(".inorbit/projects/old").is_dir(),
        "{}",
        text(&o)
    );
}

#[test]
fn keep_in_project_puts_everything_in_the_project_and_ignores_the_state() {
    let h = tempfile::tempdir().unwrap();
    let (o, dir) = init(h.path(), "inproj", &[], &["--keep-in", "project"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(recorded(&dir, "data").as_deref(), Some("project"));
    assert!(dir.join(".inorbit/memory").is_dir());
    assert!(
        dir.join(".inorbit/state").is_dir(),
        "{:?}",
        files_under(&dir)
    );
    assert!(
        !h.path().join(".inorbit/projects/inproj").exists(),
        "the home folder holds nothing of this project"
    );
    let ignore = std::fs::read_to_string(dir.join(".inorbit/.gitignore")).unwrap();
    assert!(
        ignore.contains("state/") && ignore.contains("cache/"),
        "{ignore}"
    );
}

#[test]
fn keep_in_home_puts_the_notes_and_state_in_the_home_folder() {
    let h = tempfile::tempdir().unwrap();
    let (o, dir) = init(h.path(), "inhome", &[], &["--keep-in", "home"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(recorded(&dir, "data").as_deref(), Some("home"));
    let home_dir = h.path().join(".inorbit/projects/inhome");
    assert!(
        home_dir.join("memory").is_dir(),
        "{:?}",
        files_under(h.path())
    );
    assert!(
        !dir.join(".inorbit/memory").exists(),
        "the project folder holds no notes: {:?}",
        files_under(&dir)
    );
}

#[test]
fn the_flag_beats_the_env_and_the_env_answers_without_a_flag() {
    let h = tempfile::tempdir().unwrap();
    let (o, dir) = init(
        h.path(),
        "pre",
        &[("IOHR_INIT_KEEP_IN", "home")],
        &["--keep-in", "project"],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(recorded(&dir, "data").as_deref(), Some("project"));
    let (o, dir) = init(h.path(), "env", &[("IOHR_INIT_KEEP_IN", "home")], &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(recorded(&dir, "data").as_deref(), Some("home"));
}

#[test]
fn a_dry_run_records_and_writes_nothing_and_says_what_it_would_do() {
    let h = tempfile::tempdir().unwrap();
    let (o, dir) = init(
        h.path(),
        "dry",
        &[],
        &["--keep-in", "home", "--shape", "combined", "--dry-run"],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("would:") || t.contains("dry run"), "{t}");
    assert!(!dir.exists(), "{t}");
    assert!(!h.path().join(".inorbit/projects").exists(), "{t}");
}

#[test]
fn a_shape_given_is_written_to_inorbit_toml_and_the_knowledge_base() {
    let h = tempfile::tempdir().unwrap();
    let (o, dir) = init(
        h.path(),
        "shaped",
        &[],
        &["--keep-in", "project", "--shape", "microservices"],
    );
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert_eq!(
        recorded(&dir, "shape").as_deref(),
        Some("microservices"),
        "{t}"
    );
    let notes = files_under(&dir.join(".inorbit/memory"));
    let all: String = notes
        .iter()
        .map(|f| std::fs::read_to_string(dir.join(".inorbit/memory").join(f)).unwrap_or_default())
        .collect();
    assert!(
        all.contains("This project is microservices"),
        "{notes:?}\n{t}"
    );
}

#[test]
fn without_a_shape_a_headless_run_records_none_and_does_not_fail() {
    let h = tempfile::tempdir().unwrap();
    let (o, dir) = init(h.path(), "open", &[], &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(recorded(&dir, "shape"), None);
}

#[test]
fn a_bad_place_or_shape_is_refused() {
    let h = tempfile::tempdir().unwrap();
    let (o, _) = init(h.path(), "bad1", &[], &["--keep-in", "cloud"]);
    assert!(!o.status.success(), "{}", text(&o));
    let (o, _) = init(h.path(), "bad2", &[], &["--shape", "serverless"]);
    assert!(!o.status.success(), "{}", text(&o));
}
