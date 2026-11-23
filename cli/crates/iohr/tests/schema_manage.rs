//! `iohr schemas new | bump | deprecate | list | show | validate` as a binary, in a temp
//! folder: what is written, what is refused, and that every write ends by passing the check.

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

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn all(o: &Output) -> String {
    format!("{}{}", text(&o.stdout), text(&o.stderr))
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

/// A project folder named `shop`, with a schemas type `billing/invoice` made by `new`.
fn project() -> tempfile::TempDir {
    let d = tempfile::Builder::new().prefix("shop").tempdir().unwrap();
    let o = run(
        &["schemas", "new", "billing/invoice", "--owner", "billing"],
        d.path(),
    );
    assert!(o.status.success(), "{}", all(&o));
    d
}

#[test]
fn new_scaffolds_a_type_that_passes_the_check() {
    let d = project();
    let root = d.path().join("schemas");
    for f in [
        "billing/invoice/v0.json",
        "billing/invoice/examples/minimal.json",
        "billing/invoice/invalid/unknown-field.json",
        "manifest.json",
    ] {
        assert!(root.join(f).is_file(), "{f}");
    }
    let v0: serde_json::Value =
        serde_json::from_str(&read(&root, "billing/invoice/v0.json")).unwrap();
    let id = v0["$id"].as_str().unwrap();
    assert!(
        id.starts_with("https://schemas.inorbit.hr/shop")
            && id.ends_with("/billing/invoice/v0.json"),
        "{id}"
    );
    assert_eq!(v0["title"], "Invoice");
    assert!(v0["description"].as_str().unwrap().contains("Draft line"));
    let m = read(&root, "manifest.json");
    assert!(
        m.contains(r#""billing/invoice": { "visibility": "account", "status": "draft", "owner": "billing" }"#),
        "{m}"
    );
    let c = run(&["schemas", "check"], d.path());
    assert!(c.status.success(), "{}", all(&c));
}

#[test]
fn the_id_base_is_the_manifests_when_it_names_one() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("schemas")).unwrap();
    std::fs::write(
        d.path().join("schemas/manifest.json"),
        r#"{ "base": "https://schemas.example.test/acme", "schemas": {} }"#,
    )
    .unwrap();
    let o = run(&["schemas", "new", "a/b"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    assert!(
        read(d.path(), "schemas/a/b/v0.json")
            .contains("\"https://schemas.example.test/acme/a/b/v0.json\""),
    );
}

#[test]
fn the_project_name_comes_from_inorbit_toml() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("inorbit.toml"), "name = \"eye\"\n").unwrap();
    let o = run(&["schemas", "new", "a/b"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    assert!(
        read(d.path(), "schemas/a/b/v0.json")
            .contains("https://schemas.inorbit.hr/eye/a/b/v0.json")
    );
}

#[test]
fn new_twice_refuses_and_changes_nothing() {
    let d = project();
    let before = read(&d.path().join("schemas"), "manifest.json");
    let o = run(&["schemas", "new", "billing/invoice"], d.path());
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
    assert!(all(&o).contains("exists already"), "{}", all(&o));
    assert_eq!(read(&d.path().join("schemas"), "manifest.json"), before);
}

#[test]
fn a_bad_path_or_visibility_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let o = run(&["schemas", "new", "Invoice"], d.path());
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
    let o = run(
        &["schemas", "new", "a/b", "--visibility", "everyone"],
        d.path(),
    );
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
    for bad in ["../outside", "a/../b", "/etc/x", "a//b", "a/./b"] {
        let o = run(&["schemas", "new", bad], d.path());
        assert_eq!(o.status.code(), Some(2), "{bad}: {}", all(&o));
    }
    assert!(!d.path().join("schemas").exists());
    assert!(!d.path().parent().unwrap().join("outside").exists());
}

#[test]
fn a_failed_write_leaves_the_manifest_as_it_was() {
    let d = project();
    let root = d.path().join("schemas");
    let before = read(&root, "manifest.json");
    // A file where the new type's area folder has to go: the folder cannot be made.
    std::fs::write(root.join("orders"), "in the way").unwrap();
    let o = run(&["schemas", "new", "orders/order"], d.path());
    assert!(!o.status.success(), "{}", all(&o));
    assert_eq!(read(&root, "manifest.json"), before);
    let o = run(&["schemas", "new", "orders2/order"], d.path());
    assert!(o.status.success(), "a later new is refused: {}", all(&o));
}

#[test]
fn bump_rewrites_only_the_id_member() {
    let d = project();
    let root = d.path().join("schemas");
    let v0 = read(&root, "billing/invoice/v0.json");
    let doc: serde_json::Value = serde_json::from_str(&v0).unwrap();
    let id = doc["$id"].as_str().unwrap().to_owned();
    let marked = v0.replacen('{', &format!("{{\n  \"$comment\": {id:?},"), 1);
    std::fs::write(root.join("billing/invoice/v0.json"), &marked).unwrap();
    let o = run(&["schemas", "bump", "billing/invoice"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    let v1: serde_json::Value =
        serde_json::from_str(&read(&root, "billing/invoice/v1.json")).unwrap();
    assert_eq!(v1["$comment"].as_str(), Some(id.as_str()));
    assert!(v1["$id"].as_str().unwrap().ends_with("/v1.json"), "{v1}");
}

#[test]
fn dry_run_writes_nothing_and_says_what_it_would() {
    let d = tempfile::tempdir().unwrap();
    let o = run(&["schemas", "new", "a/b", "--dry-run"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    let out = text(&o.stdout);
    assert!(
        out.contains("would write") && out.contains("a/b/v0.json"),
        "{out}"
    );
    assert!(!d.path().join("schemas").exists(), "a dry run wrote");

    let d = project();
    let root = d.path().join("schemas");
    let before = read(&root, "manifest.json");
    for args in [
        vec!["schemas", "bump", "billing/invoice", "--dry-run"],
        vec!["schemas", "deprecate", "billing/invoice", "--dry-run"],
    ] {
        let o = run(&args, d.path());
        assert!(o.status.success(), "{}", all(&o));
    }
    assert!(!root.join("billing/invoice/v1.json").exists());
    assert_eq!(read(&root, "manifest.json"), before);
}

#[test]
fn bump_makes_v1_with_its_own_id_and_the_manifest_says_stable() {
    let d = project();
    let root = d.path().join("schemas");
    let o = run(&["schemas", "bump", "billing/invoice"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    let v0 = read(&root, "billing/invoice/v0.json");
    let v1: serde_json::Value =
        serde_json::from_str(&read(&root, "billing/invoice/v1.json")).unwrap();
    assert!(
        v1["$id"]
            .as_str()
            .unwrap()
            .ends_with("/billing/invoice/v1.json")
    );
    assert!(!v1["description"].as_str().unwrap().contains("Draft line"));
    assert!(v0.contains("Draft line"), "v0 stays as it was");
    assert!(read(&root, "manifest.json").contains(r#""status": "stable""#));
    // A second bump makes v2 and leaves v1 alone.
    let v1_text = read(&root, "billing/invoice/v1.json");
    let o = run(&["schemas", "bump", "billing/invoice"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    assert_eq!(read(&root, "billing/invoice/v1.json"), v1_text);
    assert!(root.join("billing/invoice/v2.json").is_file());
}

#[test]
fn a_deprecated_type_gets_no_new_version_and_an_unknown_replacement_is_refused() {
    let d = project();
    let root = d.path().join("schemas");
    let o = run(
        &[
            "schemas",
            "deprecate",
            "billing/invoice",
            "--replaced-by",
            "billing/nope",
        ],
        d.path(),
    );
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
    assert!(
        read(&root, "manifest.json").contains(r#""status": "draft""#),
        "refusal changed the manifest"
    );
    let o = run(
        &[
            "schemas",
            "deprecate",
            "billing/invoice",
            "--replaced-by",
            "billing/invoice",
        ],
        d.path(),
    );
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));

    let o = run(&["schemas", "new", "billing/receipt"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    let o = run(
        &[
            "schemas",
            "deprecate",
            "billing/invoice",
            "--replaced-by",
            "billing/receipt",
        ],
        d.path(),
    );
    assert!(o.status.success(), "{}", all(&o));
    let m = read(&root, "manifest.json");
    assert!(
        m.contains(
            r#""status": "deprecated", "owner": "billing", "replaced_by": "billing/receipt""#
        ),
        "{m}"
    );
    assert!(root.join("billing/invoice/v0.json").is_file(), "files stay");
    let o = run(&["schemas", "bump", "billing/invoice"], d.path());
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
}

#[test]
fn a_manifest_and_a_tree_that_disagree_are_problems() {
    let d = project();
    let root = d.path().join("schemas");
    // A type on disk that the manifest does not list.
    std::fs::remove_file(root.join("manifest.json")).unwrap();
    std::fs::write(root.join("manifest.json"), r#"{ "schemas": {} }"#).unwrap();
    let o = run(&["schemas", "check"], d.path());
    assert_eq!(o.status.code(), Some(1), "{}", all(&o));
    assert!(
        all(&o).contains("billing/invoice: is not in manifest.json"),
        "{}",
        all(&o)
    );
    // A manifest line with no files.
    std::fs::write(
        root.join("manifest.json"),
        r#"{ "schemas": { "billing/invoice": { "visibility": "account", "status": "draft" },
                          "billing/ghost": { "visibility": "account", "status": "draft" } } }"#,
    )
    .unwrap();
    let o = run(&["schemas", "check", "--json"], d.path());
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let p = v["problems"].as_array().unwrap();
    assert_eq!(p.len(), 1, "{v}");
    assert_eq!(p[0]["target"], "billing/ghost");
}

#[test]
fn a_tree_without_a_manifest_passes_and_a_write_adopts_its_types() {
    let d = project();
    let root = d.path().join("schemas");
    std::fs::remove_file(root.join("manifest.json")).unwrap();
    let o = run(&["schemas", "check"], d.path());
    assert!(o.status.success(), "no manifest is fine: {}", all(&o));
    // `new` in a tree with no manifest lists what is there, and the check passes.
    let o = run(&["schemas", "new", "billing/receipt"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    assert!(all(&o).contains("did not exist"), "{}", all(&o));
    let m = read(&root, "manifest.json");
    assert!(
        m.contains("billing/invoice") && m.contains("billing/receipt"),
        "{m}"
    );
}

#[test]
fn list_and_show_read_the_tree() {
    let d = project();
    run(&["schemas", "bump", "billing/invoice"], d.path());
    let o = run(&["schemas", "list"], d.path());
    assert!(o.status.success(), "{}", all(&o));
    let t = text(&o.stdout);
    assert!(
        t.contains("billing/invoice") && t.contains("v1") && t.contains("stable"),
        "{t}"
    );
    let o = run(&["schemas", "list", "--json"], d.path());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["types"][0]["path"], "billing/invoice");
    assert_eq!(v["types"][0]["newest"], 1);
    assert_eq!(v["types"][0]["examples"], 1);
    assert_eq!(v["types"][0]["invalid"], 1);

    let o = run(
        &["schemas", "show", "billing/invoice@v0", "--json"],
        d.path(),
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["major"], 0);
    assert_eq!(v["source"], "project");
    assert!(
        v["id"]
            .as_str()
            .unwrap()
            .ends_with("/billing/invoice/v0.json")
    );
    let o = run(&["schemas", "show", "billing/invoice@v9"], d.path());
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
    let o = run(&["schemas", "show", "scrum/card"], d.path());
    assert!(o.status.success(), "an embedded type shows: {}", all(&o));
    assert!(
        text(&o.stdout).contains("shipped inside iohr"),
        "{}",
        all(&o)
    );
}

#[test]
fn validate_passes_an_example_and_refuses_an_invalid_case_naming_no_value() {
    let d = project();
    let root = d.path().join("schemas");
    let ok = root.join("billing/invoice/examples/minimal.json");
    let o = run(
        &[
            "schemas",
            "validate",
            ok.to_str().unwrap(),
            "--type",
            "billing/invoice",
        ],
        d.path(),
    );
    assert!(o.status.success(), "{}", all(&o));
    std::fs::write(
        d.path().join("secret.json"),
        r#"{"unknown": "a-secret-value"}"#,
    )
    .unwrap();
    let o = run(
        &[
            "schemas",
            "validate",
            "secret.json",
            "--type",
            "billing/invoice@v0",
        ],
        d.path(),
    );
    assert_eq!(o.status.code(), Some(1), "{}", all(&o));
    assert!(all(&o).contains("field /"), "{}", all(&o));
    assert!(!all(&o).contains("a-secret-value"), "{}", all(&o));
    let o = run(
        &[
            "schemas",
            "validate",
            "secret.json",
            "--type",
            "billing/invoice",
            "--json",
        ],
        d.path(),
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["valid"], false);
    // The embedded set, with no schemas folder at all.
    let e = tempfile::tempdir().unwrap();
    std::fs::write(e.path().join("c.json"), "{}").unwrap();
    let o = run(
        &["schemas", "validate", "c.json", "--type", "scrum/card"],
        e.path(),
    );
    assert_eq!(o.status.code(), Some(1), "{}", all(&o));
    let o = run(
        &["schemas", "validate", "c.json", "--type", "nope/nothing"],
        e.path(),
    );
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
}

#[test]
fn a_missing_folder_shows_the_schemas_examples_not_the_generic_ones() {
    let d = tempfile::tempdir().unwrap();
    let o = run(&["schemas", "check", "nowhere"], d.path());
    assert_eq!(o.status.code(), Some(2));
    let err = text(&o.stderr);
    assert!(err.contains("iohr schemas"), "{err}");
    assert!(!err.contains("iohr whoami"), "{err}");
}

#[test]
fn the_old_singular_name_is_gone() {
    let d = tempfile::tempdir().unwrap();
    let o = run(&["schema", "check"], d.path());
    assert_eq!(o.status.code(), Some(2), "{}", all(&o));
}
