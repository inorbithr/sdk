//! The profile table `iohr login` writes, read by the SDK (RFC 0231 slice 1,
//! docs/config.md section 5.6).
//!
//! These tests need the `login` feature and `inorbithr::login`, which slice 2 of
//! RFC 0231 adds by moving `iohr-auth`'s profile reader into this crate. Until then the file
//! sits in `tests/pending/`, which Cargo does not build, so main stays green under
//! `--all-features`. Slice 2 moves it to `tests/login_profile.rs` with the feature and the
//! module; it fails to compile before that change and passes after it.
#![cfg(feature = "login")]

use std::path::Path;

/// The exact text `iohr_auth::Config::to_toml` writes for a person signed in with the
/// operating system's store (`work`, an account chosen with `--account`) and a profile
/// made with `--with-token --insecure-storage` (`ci`).
///
/// Generated, not written by hand: a throwaway example in `cli/crates/iohr-auth` built the
/// `Config` with `Profile::person("acc_8d2e", Storage::Keyring,
/// "https://auth.inorbit.hr", "iohr-cli")` (with `account_pinned = true`), and
/// `Profile::new(Kind::Token, "acc_ci01", Storage::File)`, set `default` to `work`, and
/// printed `to_toml()`, at the commit this test was written on. The test below guards
/// that the SDK reads what iohr writes; `cli/crates/iohr-auth/src/config.rs` is the
/// source of the shape.
const IOHR_WRITES: &str = r#"default = "work"

[profiles.ci]
kind = "token"
account = "acc_ci01"
storage = "file"

[profiles.work]
kind = "person"
account = "acc_8d2e"
account_pinned = true
storage = "keyring"
issuer = "https://auth.inorbit.hr"
client_id = "iohr-cli"
"#;

#[test]
fn the_sdk_reads_the_profile_table_iohr_writes() {
    let config = inorbithr::login::Config::parse(IOHR_WRITES, Path::new("config.toml"))
        .expect("the table iohr writes is valid");

    assert_eq!(config.default().map(|n| n.as_str()), Some("work"));

    let work = config
        .profile("work")
        .expect("the person profile is listed");
    assert_eq!(work.kind(), inorbithr::login::Kind::Person);
    assert_eq!(work.account(), "acc_8d2e");
    assert!(work.account_pinned());
    assert_eq!(work.storage(), inorbithr::login::Storage::Keyring);
    assert_eq!(work.issuer(), Some("https://auth.inorbit.hr"));
    assert_eq!(work.client_id(), Some("iohr-cli"));

    let ci = config.profile("ci").expect("the token profile is listed");
    assert_eq!(ci.kind(), inorbithr::login::Kind::Token);
    assert_eq!(ci.storage(), inorbithr::login::Storage::File);
    assert_eq!(ci.issuer(), None);
    assert_eq!(ci.client_id(), None);
}
