//! The self-test `iohr ext dev --test` runs (as `cargo test`), with the same token
//! channel, `STATE` and `CACHE` as `iohr ext dev`: one heartbeat, then checks what it
//! wrote and where.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn one_heartbeat_in_state_and_nothing_elsewhere() {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let list = || {
        let mut v: Vec<String> = std::fs::read_dir(&here)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "target" && n != "Cargo.lock")
            .collect();
        v.sort();
        v
    };
    let before = list();
    let status = Command::new(env!("CARGO_BIN_EXE_@@NAME@@"))
        .env("IOHR_EXT_ONCE", "1")
        .status()
        .unwrap();
    assert!(status.success());
    let state = PathBuf::from(std::env::var_os("STATE").expect("run with iohr ext dev --test"));
    let beat: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.join("heartbeat.json")).unwrap()).unwrap();
    assert_eq!(beat["id"], "@@ID@@");
    assert_eq!(beat["state"], "ok");
    // Granted when you are signed in, refused when not: either way the channel answered.
    assert!(beat["token"] == "granted" || beat["token"] == "refused", "{beat}");
    assert_eq!(list(), before, "the extension wrote into its folder");
}
