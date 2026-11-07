//! `iohr init` and `iohr project init`, run as the binary with a private HOME and no network:
//! the two names are one command, the written `rust-version` is the installed toolchain's (and
//! the agent is told to keep it), and init run again in its own folder leaves what is there.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use support::kept_env;

fn script(dir: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join(name);
    std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A rustc that says `1.90.0` and a claude that does nothing, first on the PATH.
fn fake_tools(home: &Path) {
    let bin = home.join("bin");
    script(&bin, "rustc", "echo 'rustc 1.90.0 (abcdef012 2025-09-01)'");
    script(&bin, "claude", "exit 0");
}

fn iohr(home: &Path, args: &[&str]) -> Output {
    let base = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![home.join("bin")];
    paths.extend(std::env::split_paths(&base));
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(args)
        .env_clear()
        .envs(kept_env())
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("NO_COLOR", "1")
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The `--flags` a help page lists.
fn flags(help: &str) -> BTreeSet<String> {
    help.split_whitespace()
        .filter(|w| w.starts_with("--"))
        .map(|w| {
            w.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_owned()
        })
        .collect()
}

/// The arguments of a local init of `name` into `dir`.
fn init_args<'a>(lead: &[&'a str], name: &'a str, dir: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = lead.to_vec();
    a.extend([
        name,
        "--template",
        "blank",
        "--lang",
        "rust",
        "--no-smoke",
        "--no-sync",
        "--yes",
        "--dir",
        dir,
    ]);
    a.extend_from_slice(extra);
    a
}

#[test]
fn iohr_init_is_the_one_command_that_project_init_is() {
    let h = tempfile::tempdir().unwrap();
    // Listed at the top, and `project init` says so.
    let top = text(&iohr(h.path(), &["--help"]));
    assert!(
        top.lines().any(|l| l.trim_start().starts_with("init ")),
        "{top}"
    );
    let project = text(&iohr(h.path(), &["project", "init", "--help"]));
    assert!(project.contains("iohr init"), "{project}");
    // The same arguments, not a second set.
    let short = text(&iohr(h.path(), &["init", "--help"]));
    assert!(flags(&project).len() > 10, "{project}");
    assert_eq!(flags(&short), flags(&project));
    // The same plan for the same words: one code path.
    let d = h.path().join("w/a");
    let d = d.to_str().unwrap();
    let a = iohr(h.path(), &init_args(&["init"], "xx", d, &["--dry-run"]));
    let b = iohr(
        h.path(),
        &init_args(&["project", "init"], "xx", d, &["--dry-run"]),
    );
    assert!(a.status.success() && b.status.success(), "{}", text(&a));
    // (Sizes of files that carry a timestamp vary by a byte, so the file lines are left out.)
    let plan = |o: &Output| {
        text(o)
            .lines()
            .filter(|l| !l.trim_end().ends_with("bytes)"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(plan(&a), plan(&b));
}

#[test]
fn the_rust_version_is_the_installed_toolchain_and_the_agent_is_told_to_keep_it() {
    let h = tempfile::tempdir().unwrap();
    fake_tools(h.path());
    let d = h.path().join("w/xx");
    let o = iohr(
        h.path(),
        &init_args(&["init"], "xx", d.to_str().unwrap(), &[]),
    );
    assert!(o.status.success(), "{}", text(&o));
    let cargo = std::fs::read_to_string(d.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("rust-version = \"1.90\""), "{cargo}");
    // The agent that builds on it may pick the latest stable of what it uses, but is told
    // the language's minimum stays at the installed toolchain.
    let e = h.path().join("w/yy");
    let o = iohr(
        h.path(),
        &init_args(
            &["init"],
            "yy",
            e.to_str().unwrap(),
            &["--describe", "a small tool", "--dry-run"],
        ),
    );
    let said = text(&o);
    assert!(o.status.success(), "{said}");
    assert!(said.contains("Installed toolchain: rustc 1.90.0"), "{said}");
    assert!(
        said.contains("keep what the scaffold wrote") && said.contains("rust-version"),
        "{said}"
    );
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn init_run_again_in_its_folder_leaves_what_a_person_changed_and_writes_nothing_twice() {
    let h = tempfile::tempdir().unwrap();
    let d = h.path().join("w/xx");
    let ds = d.to_str().unwrap();
    let args = init_args(&["init"], "xx", ds, &[]);
    let o = iohr(h.path(), &args);
    assert!(o.status.success(), "{}", text(&o));
    // The person edits one file and adds one.
    std::fs::write(d.join("README.md"), "my own words\n").unwrap();
    std::fs::write(d.join("notes.txt"), "mine\n").unwrap();
    let cargo = d.join("Cargo.toml");
    let before = std::fs::metadata(&cargo).unwrap().modified().unwrap();
    let commits = git(&d, &["rev-list", "--count", "HEAD"]);
    let o = iohr(h.path(), &args);
    let said = text(&o);
    assert!(o.status.success(), "{said}");
    assert_eq!(
        std::fs::read_to_string(d.join("README.md")).unwrap(),
        "my own words\n"
    );
    assert_eq!(
        std::fs::read_to_string(d.join("notes.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        std::fs::metadata(&cargo).unwrap().modified().unwrap(),
        before,
        "an unchanged file is not written again"
    );
    assert!(said.contains("0 made"), "{said}");
    assert!(
        said.contains("unchanged") && said.contains("left alone") && said.contains("README.md"),
        "{said}"
    );
    // Nothing of theirs is committed for them.
    assert_eq!(git(&d, &["rev-list", "--count", "HEAD"]), commits);
    assert!(git(&d, &["status", "--porcelain"]).contains("README.md"));
    // A folder that is not this project is still refused.
    let other = h.path().join("w/other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("keep.txt"), "x").unwrap();
    let o = iohr(
        h.path(),
        &init_args(&["init"], "xx", other.to_str().unwrap(), &[]),
    );
    assert!(!o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("is not empty"), "{}", text(&o));
}
