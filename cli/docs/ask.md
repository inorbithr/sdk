# Ask your agent

`iohr ask` asks your own AI agent (Claude Code or Codex, on this machine, on your own
subscription) a question about the folder you are in. This page covers the two ways it
runs, what it sends to your provider, the permission rules that decide what it may run,
the `?` prompt in zsh, and the asks agents file for you. Back to the
[iohr overview](../README.md).

An answer is an AI answer: check it before you rely on it.

## Two ways to run

```sh
iohr ask how do I roll back the last deploy    # ask your own AI agent about this folder
iohr ask --dry-run why did the build fail      # print what it would run, run nothing
```

**The streamed run** is the default. It runs one print-mode run of your agent and streams
it to the terminal: what it thinks (dim), each tool it uses (one row each), the answer as
it arrives, and a footer with the time, turns and cost.

- With no terminal, `NO_COLOR` or `TERM=dumb`, only the answer is printed.
- `--json` passes the agent's own events through as JSON lines.
- Nothing is changed unless you pass `--allow-edits` (files only).
- `--interactive` forces this run even with the flags that select the other one.

**The capped run** is the one the `?` prompt uses. `--headless`, `--card` or a context flag
(`--last-command`, `--exit-code`, `--output-file` and the like) selects it. It is Claude
Code only: one `claude -p` run with model `haiku`, at most 12 turns and 120 seconds, no
session saved, read-only file tools (`Read`, `Grep`, `Glob`), no write access, a scrubbed
environment, and the question on stdin.

- A Bash tool is offered only through your [permission rules](#permission-rules). At a
  terminal Io asks with a small card before it runs a command the rules do not cover: a
  numbered bar, arrows or 1 to 9, `y` once, `a` always in this project, `n` no, starting on
  No. Under `NO_COLOR` or without a terminal it is one plain line.
- `--mode manual|plan|auto` (or `IOHR_ASK_MODE`, or `[ask] mode`) chooses how: `manual`
  asks before each command, `plan` is read only and answers with a plan, `auto` runs what
  your rules do not forbid without asking. Deny rules and the secret rules still win.
  `--permissions prompt|rules|deny` (or `IOHR_ASK_PERMISSIONS`, or `[ask] permissions`)
  still works.
- The answer is printed after one label line on stderr that names the agent and says it is
  an AI model; with `--card` at a terminal it is drawn as a card on stderr (plain on a
  pipe, under `NO_COLOR` or with `--json`).
- With `--json` it prints `{answer, command, questions, agent, via, label, model,
  exit_code, duration_ms, permissions, context}`. `command` is the one command the answer
  suggested, or null; `questions` are the at most two questions the model asked back, each
  `{about, ask, choices}`, or `[]`; `context` is what of the shell's context was sent, as
  data: `{folder, command, exit_code, output, output_bytes, can_attach}`, where `output` is
  `not_sent`, `attached`, `captured` or `pasted`, never the text itself.

Inside a project the prompt also carries iohr's own git facts (the branch, the last 5
commits and the change counts, at most 2 KiB) and the project's notes. At a terminal the
card asks the model's questions, keeps your answer in `.inorbit/memory/` and
`inorbit.toml`, and refreshes the answer once; a dim line says when asks are waiting.

`--role NAME` runs the agent in a role ([roles](roles-and-memory.md#use-a-role-in-a-run---role)).
`--dry-run` prints the agent's command and prompt and runs nothing.

### More folders to read

`--read FOLDER` (alias `--add-dir`, repeatable), `IOHR_ASK_READ` (a path list) and
`[ask] read` name more folders the run may read. Each is checked like a yes: it must exist,
hold no secrets and not be broad (the root, your home folder and every folder above it are
refused, except an explicit `--read ~`), and the secret deny rules stay on. Each is
written to `permissions.log` by source and path and listed in `--json` under
`context.folders`.

### Not supported yet

- `--via io` (Io over the coordinator) says "not built yet"; use `--via local`.
- `--agent codex` with `--headless`, `--card` or a context flag says "not built yet"; use
  `--agent claude`.

## What goes to your provider

For the capped run, exactly this, and nothing else:

| What | Limit |
|---|---|
| The question | 2 KiB |
| The folder's path | 1 KiB |
| The names at the top of the folder | 60 names and 2 KiB; dotfiles left out except `.inorbit`, `.github`, `.gitignore` and `.claude`; names the folder's own `.gitignore` hides left out; never file contents |
| The last command and its exit status | The command at 1 KiB |
| Output, only when attached | The last 8 KiB (200 lines), with secrets redacted: passwords, tokens, bearer values, URLs, hosts, IP addresses |
| The `IO.md` files of the folder chain | 8 KiB, secrets scrubbed; see [IO.md](projects.md#iomd-instructions-for-the-folder) |
| The whole message | 24 KiB |

The card's `Read:` line says exactly what was sent, including "output not sent" when none
went. Io signs in with your own `claude` login, never an API key, and `iohr` stores neither
the question nor the answer.

A plain zsh hook cannot read a command's output, so it sends none. Output goes only when you
attach it: `--output-file FILE`, or, after a failed command and a question such as "why did
that fail", the menu's **Re-run with output** (it shows the exact command and asks first,
never runs it on its own, runs once for at most 10 seconds and keeps the last 16 KiB) or
**Paste output** (read until Ctrl-D). What you attach goes with the next ask only. Off a
terminal, the card says the output was not sent and how to attach it.

## The `?` prompt in zsh

Add this to `~/.zshrc`:

```sh
eval "$(iohr shell hook zsh)"
```

Then type `? why did that fail` at the prompt.

- The line is never run by the shell and is not kept in its history. The answer prints
  under it, after the label line and a line that says what was read.
- `?? and the second one` asks a follow-up with the last question and answer as context.
- A line such as `?.txt`, or `? ` alone, runs as it always did.
- An accept-line widget you already have is kept and called for every other line.
- Shift+Tab on a `?` line switches the mode between manual, plan and auto. It says which
  under the prompt, and every question carries it.
- A Down arrow on an empty line (or the key in `[ask] menu_key`) opens the action menu.
- The right prompt shows "3 asks" when asks wait for you, read from
  `~/.inorbit/term/asks-count` and refreshed in the background. `INORBIT_ASKS_PROMPT=0`
  turns that off.
- `INORBIT_ASK=0` turns the hook off for the session.
- Inside the InOrbit app (`INORBIT_TERM` is set) the hook asks the app to draw the answer
  (an OSC `7727;ask` escape) instead of printing it.

Not supported yet: bash and fish. `iohr shell hook bash` and `iohr shell hook fish` exit 1
with "not built yet".

### The action menu

`iohr ask --menu --command C ...` is the menu under a `?` card, drawn on the terminal with
letter keys. It offers:

- Run, Copy, Follow up, Explain and Open;
- Undo, for a permission the run saved;
- Add folder, to keep one more folder for Io to read;
- Allow, for a folder Io asked to read (kept in `trusted.toml`, then the question is put on
  the prompt again);
- the answer's next steps as numbered entries;
- Asks, when some wait;
- after a failed command and a "why did that fail" question, Re-run with output and Paste
  output (above).

It prints only the one line for the hook, asks no model and runs nothing. For Run it prints
the command and `run` after a 0x1e character; the hook runs it at once, after a yes when it
is risky, and never when a rule denies it. A command with a placeholder gets Copy only.

## Permission rules

```sh
iohr permissions list                 # the rules, and where each is kept
iohr permissions allow 'Bash(ls:*)'   # allow a command
iohr permissions check Bash ls        # what the rules say about one call
```

The rules for what Io may run or open when you ask it something use Claude Code's rule
grammar. They live in `.inorbit/permissions.toml` (`--project`) and in the user file in the
iohr config folder (`--user`).

- `allow` refuses a rule that could open a secret file, and a blanket `Bash(*)` (exit 2).
- `check TOOL INPUT` prints `allow`, `deny` or `ask` and the rule that decided.
- Every ask decision is logged to `.inorbit/permissions.log`, with no output and no file
  contents.

The file format and the order rules are read in: [docs/config.md](../../docs/config.md),
section 4.5.

## The asks agents file for you

An ask is a question or an approval an agent files for you: a review, a document, a
decision, a run, a merge and the like.

```sh
iohr asks list            # the asks waiting for you
iohr asks count           # how many wait
iohr asks show ASK_ID     # one ask with its timeline
```

`iohr asks count [--cache] [--json] [--account A]` counts only the asks whose turn is yours
(`pending`). One you already answered with a note (`question`) is the agent's turn and is
left out; `iohr asks list` still shows it. With `--json` it prints `{"count": N}`. `--cache`
keeps just the number in `~/.inorbit/term/asks-count` and does nothing while that file is
under 30 seconds old; the shell hook's prompt uses it. It reads the sign-in through a quiet
credential store, with no Keychain dialog and no wait past 1.5 seconds. When the store would
ask, `--cache` keeps the last number and prints nothing, and without `--cache` it exits 1
saying so.

On a terminal, `list` and `show` stay live: the screen is drawn again when an ask moves. A
pipe, `--json` and `--once` print once. `iohr asks question ID` sends a question back to the
asker, `deny ID` refuses, `approve ID` approves, and `decide ID` is a menu of the three for
one open ask. `deny` and `question` need no key. The text of an ask is printed to stdout
with control characters dropped, and never written to a log, an error or stderr.

### Approving on a Mac

`approve` needs a device key's proof. A Mac signs it with its own Secure Enclave key after
Touch ID:

```sh
iohr asks key enroll [--label L] [--proof FILE]
iohr asks key show
iohr asks key remove
```

- The key is a P-256 key made on the first `key enroll`, kept as a file with mode 0600 that
  only that chip can open, and bound to the current fingerprints: change them and enrol
  again. The first `enroll` compiles a small Swift helper with the system `swiftc`, so
  Xcode's command line tools must be installed.
- `enroll` registers it as a `macos` device key. `--proof FILE` takes the countersignature
  JSON (`{key_id, signed_at, nonce, signature}`) an enrolled phone key made over the new
  public key, which activates it at once; otherwise the service holds a new key for 24
  hours. The first key is active at once.
- `key show` reads it back; `key remove` revokes it and deletes the file.
- Before Touch ID, `approve` prints the ask's title, kind and payload hash, and the Touch ID
  prompt names the same title. A refused Touch ID signs nothing and changes nothing.

Only a review, a document, a decision and a run may be signed on a Mac. A merge, a roll, a
migration and an auto-approve rule (found in the kind, title, summary or what approving
means) always say "Approve on your phone", and so does a Mac without a key. Windows and
Linux never sign: `key enroll` refuses there. When the phone has to approve, `approve` says
so and waits until the phone has done it.
