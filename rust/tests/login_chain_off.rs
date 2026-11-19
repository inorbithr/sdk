//! Without the `login` feature a person's profile is skipped by the credential chain
//! with a reason that names the feature: no panic, no silent fallback (RFC 0231,
//! `docs/config.md` section 5.1).

#![cfg(not(feature = "login"))]
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use inorbithr::config::LoadOptions;
use inorbithr::{Client, Public};

#[test]
fn a_person_profile_is_skipped_with_the_feature_named() {
    let dir = tempfile::tempdir().expect("a folder");
    let file = dir.path().join("config.toml");
    std::fs::write(
        &file,
        "default = \"dev\"\n[profiles.dev]\nkind = \"person\"\naccount = \"acc_1\"\n\
         storage = \"keyring\"\n",
    )
    .expect("a config");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).expect("a home");
    let env = BTreeMap::from([
        (
            "INORBIT_BASE_URL".to_owned(),
            "http://127.0.0.1:9".to_owned(),
        ),
        ("PATH".to_owned(), String::new()),
    ]);

    let err = Client::<Public>::builder()
        .config_file(&file)
        .load_options(LoadOptions::new().env(env).home(Some(home)).cwd(dir.path()))
        .load()
        .expect_err("no credential applies without the feature");

    let text = err.to_string();
    assert!(text.contains("login"), "the login source is named: {text}");
    assert!(
        text.contains("`login` cargo feature"),
        "the reason names the feature: {text}"
    );
}
