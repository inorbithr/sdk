//! `iohr shell hook`: what a shell evaluates to get the `? question` prompt (PRD 0187).

use crate::cli::{HookShell, ShellCommand};
use crate::error::Error;
use crate::output::Out;

/// The zsh hook. A file of its own, so `zsh -n` can check it and the tests can run it.
pub(crate) const ZSH_HOOK: &str = include_str!("shell/hook.zsh");

pub(crate) fn run(cmd: &ShellCommand) -> Result<(), Error> {
    let ShellCommand::Hook { shell } = cmd;
    match *shell {
        HookShell::Zsh => {
            // Text that is evaluated: printed as it is, also under `--json`.
            Out::raw(ZSH_HOOK.as_bytes());
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
