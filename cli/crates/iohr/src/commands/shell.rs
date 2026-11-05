//! `iohr shell hook`: what a shell evaluates to get the `? question` prompt (PRD 0187).

use crate::cli::{HookShell, ShellCommand};
use crate::error::Error;
use crate::output::Out;

/// The zsh hook. A file of its own, so `zsh -n` can check it and the tests can run it.
pub(crate) const ZSH_HOOK: &str = include_str!("shell/hook.zsh");

/// The file name this binary was started as, when it is a plain command name (letters,
/// digits, `.`, `_`, `-`): the hook is evaluated by a shell, so anything else is `iohr`.
fn bin_name(arg0: Option<&std::ffi::OsStr>) -> &str {
    let name = arg0
        .and_then(|a| std::path::Path::new(a).file_name())
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("iohr");
    let plain = !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if plain { name } else { "iohr" }
}

pub(crate) fn run(cmd: &ShellCommand) -> Result<(), Error> {
    let ShellCommand::Hook { shell } = cmd;
    match *shell {
        HookShell::Zsh => {
            // Text that is evaluated: printed as it is, also under `--json`. It calls back the
            // binary that printed it, so `iohr-preview shell hook zsh` asks `iohr-preview`.
            let name = std::env::args_os().next();
            let hook = ZSH_HOOK.replace("__IOHR_BIN__", bin_name(name.as_deref()));
            Out::raw(hook.as_bytes());
            Ok(())
        }
        HookShell::Bash => Err(Error::Failed(
            "`iohr shell hook bash` is not built yet; only zsh is (`iohr shell hook zsh`)"
                .to_owned(),
        )),
        HookShell::Fish => Err(Error::Failed(
            "`iohr shell hook fish` is not built yet; only zsh is (`iohr shell hook zsh`)"
                .to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::bin_name;
    use std::ffi::OsStr;

    #[test]
    fn the_hook_calls_back_a_plain_binary_name_and_iohr_otherwise() {
        assert_eq!(
            bin_name(Some(OsStr::new("/Users/x/.local/bin/iohr-preview"))),
            "iohr-preview"
        );
        assert_eq!(bin_name(Some(OsStr::new("iohr"))), "iohr");
        assert_eq!(bin_name(None), "iohr");
        for odd in [
            "/tmp/a b",
            "/tmp/x;rm",
            "/tmp/$(id)",
            "-i",
            "/tmp/i\u{f6}hr",
        ] {
            assert_eq!(bin_name(Some(OsStr::new(odd))), "iohr", "{odd}");
        }
    }
}
