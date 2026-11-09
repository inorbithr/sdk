# The terminal theme

Back to the [iohr overview](../README.md).

`iohr theme` gives the real Terminal.app and zsh the look of the terminal in the InOrbit Mac
app (RFC 0212). The four themes, the font (Geist Mono 12.5, then SF Mono, then Menlo) and the
cursor (a blinking bar) are the app's own: `tools/terminal-themes.py` reads `THEMES`,
`DEFAULTS` and `fontStack` from the app's `src/lib/term-config.ts` and writes
`cli/crates/iohr/src/commands/theme/themes.json`, and both the profile and the prompt are made
from that file. `mise run cli:themes` regenerates it; `mise run cli:themes:check` fails when it
drifted from the app (it needs an iohr-mac checkout, `IOHR_MAC_DIR`; CI runs the offline
check that the file is whole).

Nothing happens until you ask. `iohr theme install nord` writes two files, both under
`~/.inorbit/term` (`IOHR_TERM_DIR` moves it), and says so: `themes/nord.terminal` and
`theme.json`. It never edits `~/.zshrc`. Three things are flags:

- `--terminal` runs `open` on the profile, so Terminal.app imports it as "InOrbit Nord" and opens a
  window with it. It is not made your default profile: Terminal's Settings, Profiles, Default does that.
  Terminal.app takes one face, so the profile uses Geist Mono when it is installed in
  `~/Library/Fonts` or `/Library/Fonts`, else SF Mono, else Menlo.
- `--prompt` turns on the zsh prompt: the folder, the git branch and, after a failed command, the
  status on one line, and `❯` (the accent colour; red after a failure) on the next. It is part of
  the shell hook, so `eval "$(iohr shell hook zsh)"` must be in `~/.zshrc`; `install` tells you
  when it is not. The right prompt (the "3 asks" segment) is not touched. The colours are
  24-bit where the terminal says so (`COLORTERM`), else the nearest of 256 (Terminal.app has no
  24-bit colour, so the prompt there is an approximation of the profile's colours).
- `--replace-prompt` is `--prompt` that also replaces a prompt you set yourself. Without it, the
  prompt is set only when `PROMPT` is empty, is zsh's default (`%m%# `) or macOS's
  (`%n@%m %1~ %# `), or is the one the theme set; a prompt of your own, or a framework's, stays.

`INORBIT_PROMPT=0` turns the prompt off for one shell and `INORBIT_PROMPT_DIRTY=1` adds a mark
when the working tree has changes (slower in a big repository). `iohr theme off` turns it off
for new shells and forgets the theme; it does not remove a profile you imported into
Terminal.app, and does not touch your shells that are already open. Under `--json` every
subcommand prints one object and nothing else.
