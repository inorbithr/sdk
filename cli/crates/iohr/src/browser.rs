//! Whether this machine can show a browser, and opening one.

use std::process::{Command, Stdio};

/// Whether a browser on this machine can be opened for the person at the keyboard.
/// No over SSH, in CI, in a container, or on Linux without a display.
pub(crate) fn available() -> bool {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if set("CI") || set("SSH_CONNECTION") || set("SSH_TTY") || set("SSH_CLIENT") {
        return false;
    }
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return true;
    }
    let container = std::path::Path::new("/.dockerenv").exists()
        || std::path::Path::new("/run/.containerenv").exists();
    !container && (set("DISPLAY") || set("WAYLAND_DISPLAY"))
}

/// Opens `url` in the default browser, without a shell: the URL is one argument.
pub(crate) fn open(url: &str) -> bool {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        // Not `cmd /c start`, whose parser would read `&` in the URL.
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}
