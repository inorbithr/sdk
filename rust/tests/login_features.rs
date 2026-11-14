//! The `login` feature is the only way the operating-system credential-store crates
//! enter a build (RFC 0231): a program that calls the API with a token must not link
//! the Keychain, Credential Manager or Secret Service code.
//!
//! This asks Cargo for the real dependency tree of this crate, for every target, and
//! compares the crate names with the list the `login` feature owns. It needs no
//! feature of its own, so it runs in every feature set.

#![allow(clippy::expect_used)]

use std::process::Command;

/// The crates only `login` brings in: the credential-store crates.
const STORE_CRATES: [&str; 4] = [
    "keyring-core",
    "apple-native-keyring-store",
    "windows-native-keyring-store",
    "zbus-secret-service-keyring-store",
];

/// Crates `login` also lists, which a default build may reach through TLS or another
/// dependency (`security-framework` through the platform certificate verifier on macOS).
/// They are checked with `login` on, never against the default tree.
const SHARED_CRATES: [&str; 3] = ["security-framework", "toml_edit", "sha2"];

/// The crates `cargo tree` lists for this package's normal and build dependencies on
/// every target, with the given feature arguments.
fn tree(features: &[&str]) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = Command::new(cargo)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["tree", "--package", "inorbithr", "--target", "all"])
        .args(["--edges", "normal,build", "--prefix", "none", "--locked"])
        .args(features)
        .output()
        .expect("cargo tree runs");
    assert!(
        out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect()
}

#[test]
fn default_features_pull_in_no_store_or_keychain_crate() {
    let names = tree(&[]);
    assert!(
        names.iter().any(|n| n == "inorbithr"),
        "the tree lists the crate itself: {names:?}"
    );
    let leaked: Vec<&str> = STORE_CRATES
        .iter()
        .copied()
        .filter(|c| names.iter().any(|n| n == c))
        .collect();
    assert!(
        leaked.is_empty(),
        "default features pull in credential-store crates: {leaked:?}"
    );
}

#[test]
fn the_login_feature_brings_every_store_crate() {
    let names = tree(&["--features", "login"]);
    for c in STORE_CRATES.into_iter().chain(SHARED_CRATES) {
        assert!(
            names.iter().any(|n| n == c),
            "`login` should bring in {c}; the tree has {} crates",
            names.len()
        );
    }
}
