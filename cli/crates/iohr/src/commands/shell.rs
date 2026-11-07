//! `iohr shell hook`: what a shell evaluates to get the `? question` prompt (PRD 0187).

use crate::actions::MenuKey;
use crate::cli::{Global, HookShell, ShellCommand};
use crate::context::Ctx;
use crate::error::Error;
use crate::output::Out;
use crate::permissions::{Mode, Words};

/// The zsh hook. A file of its own, so `zsh -n` can check it and the tests can run it.
pub(crate) const ZSH_HOOK: &str = include_str!("shell/hook.zsh");

/// The command this binary was started as, when it is a plain command name (letters,
/// digits, `.`, `_`, `-`): the hook is evaluated by a shell, so anything else is `iohr`.
/// A trailing `.exe` is dropped: Windows finds `iohr.exe` when a person types `iohr`, and
/// `iohr` is what the project's instructions and the `?` card tell them to run.
pub(crate) fn bin_name(arg0: Option<&std::ffi::OsStr>) -> &str {
    let name = arg0
        .and_then(|a| std::path::Path::new(a).file_name())
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("iohr");
    let name = match name.len().checked_sub(4) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".exe") => {
            &name[..at]
        }
        _ => name,
    };
    let plain = !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if plain { name } else { "iohr" }
}

/// The name this process was started as (`iohr`, or `iohr-preview`): what a project's
/// instructions tell an agent to run, so they call back the binary that made them.
pub(crate) fn running_bin() -> String {
    bin_name(std::env::args_os().next().as_deref()).to_owned()
}

/// The menu key's `bindkey` strings as zsh words, each in single quotes (they hold `^` and `[`).
fn key_words(key: MenuKey) -> String {
    key.sequences()
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The mode the hook starts in: what `IOHR_ASK_MODE` / `IOHR_ASK_PERMISSIONS` and `[ask]` name,
/// when it is one of the three of the switch; empty otherwise (a `deny` or `rules` choice stays
/// the config's, and the first Shift+Tab goes to the first mode of the ring). A bad word is an
/// error here as it is in `iohr ask`.
fn start_mode(ctx: &Ctx) -> Result<&'static str, Error> {
    let env = |k: &str| std::env::var(k).ok();
    let (mode, perms) = (env("IOHR_ASK_MODE"), env("IOHR_ASK_PERMISSIONS"));
    let flag = Words {
        mode: mode.as_deref(),
        permissions: perms.as_deref(),
    };
    let ask = &ctx.config.ask;
    let config = Words {
        mode: ask.mode.as_deref(),
        permissions: ask.permissions.as_deref(),
    };
    Ok(Mode::choose(flag, config, true)
        .map_err(Error::Usage)?
        .shown()
        .unwrap_or(""))
}

pub(crate) fn run(g: &Global, cmd: &ShellCommand) -> Result<(), Error> {
    let ShellCommand::Hook { shell } = cmd;
    match *shell {
        HookShell::Zsh => {
            // Text that is evaluated: printed as it is, also under `--json`. It calls back the
            // binary that printed it, so `iohr-preview shell hook zsh` asks `iohr-preview`.
            let ctx = Ctx::load(g)?;
            let key = MenuKey::parse(ctx.config.ask.menu_key.as_deref()).map_err(Error::Usage)?;
            let hook = ZSH_HOOK
                .replace("__IOHR_BIN__", &running_bin())
                .replace("__IOHR_ASK_MODE__", start_mode(&ctx)?)
                .replace("__IOHR_MODES__", &Mode::ring())
                .replace("__IOHR_MENU_KEYS__", &key_words(key));
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
        // Windows: the command people type, not the file Windows found.
        assert_eq!(bin_name(Some(OsStr::new("iohr.exe"))), "iohr");
        assert_eq!(
            bin_name(Some(OsStr::new("iohr-preview.EXE"))),
            "iohr-preview"
        );
        assert_eq!(bin_name(Some(OsStr::new(".exe"))), "iohr");
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
