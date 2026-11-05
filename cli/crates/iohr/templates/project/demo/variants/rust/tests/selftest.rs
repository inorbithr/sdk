//! The self-test: `cargo test` runs the built program once with a STATE and CACHE of its own,
//! as `iohr ext dev --test` and InOrbit Desktop's smoke test do. One heartbeat, then what it
//! wrote and where.

use std::process::Command;

#[test]
fn one_heartbeat_in_state_and_nothing_elsewhere() {
    let tmp = std::env::temp_dir().join(format!("@@NAME@@-selftest-{}", std::process::id()));
    let (state, cache) = (tmp.join("state"), tmp.join("cache"));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let here = env!("CARGO_MANIFEST_DIR");
    let before = std::fs::read_dir(here).unwrap().count();
    let out = Command::new(env!("CARGO_BIN_EXE_@@NAME@@"))
        .env("STATE", &state)
        .env("CACHE", &cache)
        .env("IOHR_EXT_ONCE", "1")
        .env_remove("IOHR_EXT_TOKEN_SOCKET")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let beat = std::fs::read_to_string(state.join("heartbeat.json")).unwrap();
    assert!(beat.contains("\"id\":\"@@ID@@\""), "{beat}");
    assert!(beat.contains("\"state\":\"ok\""), "{beat}");
    assert!(beat.contains("\"beats\":1"), "{beat}");
    // Granted when you are signed in and the channel is up, refused when not: either way it answered.
    assert!(beat.contains("\"token\":\"refused\"") || beat.contains("\"token\":\"granted\""), "{beat}");
    assert_eq!(std::fs::read_dir(here).unwrap().count(), before, "the project wrote into its own folder");
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn an_undeclared_scope_is_refused_without_a_channel() {
    // The channel module is part of the binary; this exercises it through the program's refusal:
    // with no socket the token ask is refused and the heartbeat still says so.
    let out = Command::new(env!("CARGO_BIN_EXE_@@NAME@@"))
        .env("STATE", std::env::temp_dir())
        .env("CACHE", std::env::temp_dir())
        .env("IOHR_EXT_ONCE", "1")
        .env_remove("IOHR_EXT_TOKEN_SOCKET")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("token refused"));
}
