//! Whether this machine can show a browser, and opening one. `IOHR_NO_BROWSER` (any
//! non-empty value) turns both off: the test harnesses set it so a run never opens a tab.

use std::process::{Command, Stdio};

/// Whether a browser on this machine can be opened for the person at the keyboard.
/// No over SSH, in CI, in a container, on Linux without a display, or with `IOHR_NO_BROWSER`.
pub(crate) fn available() -> bool {
    available_with(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

fn available_with(set: impl Fn(&str) -> bool) -> bool {
    if set("IOHR_NO_BROWSER")
        || set("CI")
        || set("SSH_CONNECTION")
        || set("SSH_TTY")
        || set("SSH_CLIENT")
    {
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
    if std::env::var_os("IOHR_NO_BROWSER").is_some_and(|v| !v.is_empty()) {
        return false;
    }
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

#[cfg(test)]
mod tests {
    use super::available_with;

    #[test]
    fn no_browser_var_turns_it_off_even_where_one_exists() {
        assert!(!available_with(|k| k == "IOHR_NO_BROWSER" || k == "DISPLAY"));
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn a_desktop_has_a_browser_without_the_var() {
        assert!(available_with(|_| false));
    }
}
