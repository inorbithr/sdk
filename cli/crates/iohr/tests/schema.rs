//! `iohr schema check` as a binary: exit codes, one line per problem, `--json`, and a
//! copy of a real project tree.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::path::Path;
use std::process::{Command, Output};
use support::kept_env;

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("NO_COLOR", "1")
        .envs(kept_env())
        .output()
        .unwrap()
}

fn put(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

const ID: &str = "https://schemas.inorbit.hr/iohr/demo/thing/v0.json";

fn good_tree(root: &Path) {
    put(
        root,
        "demo/thing/v0.json",
        &format!(
            r#"{{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"{ID}",
            "type":"object","required":["name"],"properties":{{"name":{{"type":"string"}}}},
            "additionalProperties":false}}"#
        ),
    );
    put(root, "demo/thing/examples/ok.json", r#"{"name":"a"}"#);
    put(root, "demo/thing/invalid/no-name.json", "{}");
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn a_good_tree_passes() {
    let d = tempfile::tempdir().unwrap();
    good_tree(&d.path().join("schemas"));
    let o = run(&["schema", "check"], d.path());
    assert!(o.status.success(), "{}{}", text(&o.stdout), text(&o.stderr));
}

#[test]
fn an_invalid_case_that_passes_fails_the_check_with_one_line() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("schemas");
    good_tree(&root);
    put(&root, "demo/thing/invalid/fine.json", r#"{"name":"x"}"#);
    let o = run(&["schema", "check", "schemas"], d.path());
    assert_eq!(o.status.code(), Some(1), "{}", text(&o.stderr));
    let all = format!("{}{}", text(&o.stdout), text(&o.stderr));
    let lines: Vec<&str> = all.lines().filter(|l| l.contains("fine.json")).collect();
    assert_eq!(lines.len(), 1, "{all}");
}

#[test]
fn json_output_counts_and_lists_problems() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("schemas");
    good_tree(&root);
    let ok = run(&["schema", "check", "--json"], d.path());
    let v: serde_json::Value = serde_json::from_slice(&ok.stdout).unwrap();
    assert_eq!(v["types"], 1, "{v}");
    assert_eq!(v["problems"].as_array().unwrap().len(), 0, "{v}");

    put(&root, "demo/thing/examples/bad.json", r#"{"name":1}"#);
    let bad = run(&["schema", "check", "--json"], d.path());
    assert_eq!(bad.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&bad.stdout).unwrap();
    assert_eq!(v["problems"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn a_missing_folder_is_a_usage_error() {
    let d = tempfile::tempdir().unwrap();
    let o = run(&["schema", "check", "nowhere"], d.path());
    assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
    assert!(
        text(&o.stderr).contains("nowhere: no such folder"),
        "{}",
        text(&o.stderr)
    );
}

fn copy_tree(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap().map(Result::unwrap) {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dst).unwrap();
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// A copy of another project's `schemas/` tree, named by `IOHR_TEST_SCHEMAS_TREE`,
/// passes the check unchanged. Skipped when the variable is not set.
#[test]
fn a_copy_of_a_real_project_tree_passes() {
    let Some(src) = std::env::var_os("IOHR_TEST_SCHEMAS_TREE") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let dst = d.path().join("schemas");
    std::fs::create_dir_all(&dst).unwrap();
    copy_tree(Path::new(&src), &dst);
    let o = run(&["schema", "check"], d.path());
    assert!(o.status.success(), "{}{}", text(&o.stdout), text(&o.stderr));
}
