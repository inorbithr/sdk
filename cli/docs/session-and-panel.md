# Sessions and the panel

How `iohr session` runs a project's roles on this machine, how `iohr stop` ends them, and
the floating panel that bare `iohr` puts over your shell. Back to the
[iohr overview](../README.md).

Sessions and the panel run on Unix only (macOS and Linux).

## `iohr session` and `iohr stop`

```sh
iohr session            # this folder's project; stays open until Ctrl-C or SIGTERM
iohr session ~/work/my-shop --json
iohr stop               # end the project's session host
iohr stop --wait 60
```

`iohr session [DIR] [--json]` opens a session on a project. It starts the project's one
session host if none runs, joins it, and stays open until Ctrl-C or SIGTERM. The host is
this same `iohr` as `iohr runner run --session DIR`, a process of its own that runs the
[runner](runner.md) (heartbeats, a 90-second lease on the service, turns on your `claude -p`,
no self-upgrade while it hosts) plus the project's roles at work on one local queue. With
`--json` it prints `{project, host_pid, roles}`; notes go to stderr.

The host needs a runner of its own. It signs in as the profile `session`, whatever
`--profile` the shell has, with its own runner key: sharing the live runner's key would make
the two overwrite one heartbeat row and take each other's leases. Set it up once:

```sh
iohr login --profile session
iohr --profile session runner register --name HOST-session
```

Without them the host stops at once and says which one is missing. If the host stops at
once for any reason, the message carries the last line of its `host.log`.

The host stops when the last session has left (a shell that was killed counts as left), on
`iohr stop`, or on SIGTERM (exit 0). A host nobody joins within 30 seconds ends by itself.
There is at most one host per project: a lock the kernel frees when the process dies, so a
crash leaves nothing to clean up.

`iohr stop [DIR] [--wait S] [--json]` stops the project's session host and waits up to
`--wait` seconds (0 to 600, default 15) for it to end.

- The host drains: a role takes no more tasks, an answer it is still writing is cut short
  (its child process group is killed), and the task goes back to `queued` with the try not
  counted.
- Until the last role has stopped, the host still answers `status` with `stopping` and holds
  its lock. `--wait` polls that lock, so `stopped` is printed only when the host has really
  ended, and `stopping` when the wait ran out first.
- With no host running it says so on stderr. It exits 0 either way.
- `--json` prints `{project, state, host_pid}`, with `state` one of `not running`,
  `stopped` or `stopping`.
- It makes no API call.

### The queue

The queue is a DuckDB file, `queue.duckdb`, in `session/<project hash>/` below the data
directory (`IOHR_DATA_DIR`, else the platform's own), beside the host's socket and its log
(`host.log`, the host's own output).

- The host is the only process that opens it; shells talk to the host over its socket.
- The folder must be yours and mode 0700 (the host refuses one that is not, or that is a
  link), and the socket is mode 0600. A socket path over 103 bytes is refused with a message
  that names the fix. A host holds at most 64 connections; more are answered `error busy`.
- The queue holds `roles`, `tasks` and an append-only `task_events`.

A task moves through these states:

| State | Next | When |
|---|---|---|
| `queued` | `claimed` | A role takes the next task in one statement, with a 90-second lease its heartbeat renews |
| `claimed` | `running` | The role starts it |
| `running` | `done` or `failed` | The role finishes it, or it fails |
| `claimed`, `running` | `queued` | The lease ran out after the role's thread ended, or `iohr stop` cut it short (the try not counted), or a host found it orphaned at start |
| `queued` | `failed` | It was tried 3 times; the reason is kept |

Leases are timed on the host process's monotonic clock, never the wall clock, so a laptop
that sleeps or a clock that is set back cannot make a live role's task look abandoned. While
a host runs, a task is taken from its role only when that role's thread has ended and its
lease has run out; a check every few seconds does that. A host that starts holds the lock
alone, so every claim left in the file is an orphan and goes back to `queued` with the
attempt counted. A statement that meets another role's edit of the same row is tried again,
not failed.

A task is meant to be worked once at a time, but a host killed in the middle of one runs it
again on the next start (at least once), so a handler must be safe to repeat.

### Asks answered by the roles

When a host runs for the folder an ask is made in (or one above it), each `?` or `??` ask
([the `?` prompt](ask.md#the--prompt-in-zsh)) is put in the queue once per role, for every
role the host runs or only the ones `--role a,b` names. Its id is `ask:<id>:<role>`, so a
repeat adds nothing.

- Each role answers as a child of the host, `iohr --json ask --headless --role <role>
  --cwd <folder>`: read-only, and `plan` when the ask was `plan`. It runs at the same time
  as Io's own answer.
- When Io's card is drawn, the roles' answers follow, one card each as it lands. The stored
  answer is JSON `{text, model, duration_ms}`, with the text cut at 48 KiB (49,152
  characters). A role that fails shows its reason in its card.
- Ctrl-C leaves the wait; the roles keep working and `iohr stop` ends them. A task still
  queued or cut short when the host stops is answered by the next host.

The ask goes the way it always did with no host, with `--json`, `--menu`, `--dry-run`,
`--allow-edits` or `--mode auto`, with a `--role` the host does not run, or with an older
host.

Not supported yet: re-attaching to an ask's answers after the wait ended, pruning finished
tasks, and starting a session host at login (there is no launchd or systemd unit for it).

### What a role keeps between turns

Each role keeps two plain files in the project between its turns; nothing is sent anywhere.

- **A handoff note**, `.inorbit/handoff/<role>.md`, written by the host when a role's turn
  ends: what it was asked and what it answered (cut to 800 and 2,000 characters), replaced
  by each turn, and written through a temporary file and a rename.
- **A memory area**, `.inorbit/memory/<role>/`, in the same format as
  [`iohr memory`](roles-and-memory.md#memory). The project's own store skips the
  sub-folder, so a role's lessons are not the project's.

Every `iohr ask --role <role>` turn, routed or not, is told the note and the newest 8
memories of the area (about 2,000 characters, pinned first) after the role's text. A role
name that is not lower-case letters, digits, `-` and `_` (at most 64) keeps and reads
nothing. Not supported yet: nothing writes the memory area for you; add a memory file there
by hand.

## Bare `iohr` and the panel

On a terminal, `iohr` with no subcommand enters the session: it joins this folder's session
host (starting it if none runs, as `iohr session` does) and gives you your shell with the
floating panel over it. If the host cannot start, it says so on stderr and the panel still
runs without one. `iohr stop` ends the host, which closes the panel and leaves your shell
open.

Under `NO_COLOR`, with `--json`, or off a terminal, bare `iohr` prints the usage report and
exits 2. `iohr --help` and `iohr help` print the help.

Inside the panel, bare `iohr` prints one line (that the panel is on, and which key opens the
menu) and exits 0. After `iohr stop` it brings the panel back first. If nothing runs it,
`iohr stop` says the panel is closed.

### How it draws

The panel runs your shell inside a small terminal proxy and paints itself over the top right
corner after every chunk of output, saving and restoring the cursor.

- It takes no rows: the shell keeps its full height, what you half-typed stays where it is,
  and the panel moves when the window is resized.
- Nothing is drawn under `NO_COLOR`, off a terminal, or on the alternate screen (vim,
  less); it comes back when the program ends.
- Colours are truecolor when `COLORTERM=truecolor`, else the nearest of the 256.
- The shell runs with `IOHR_PANEL` set to the pid of the panel around it, which counts only
  while that process lives. A nested panel runs plainly and a bare `iohr` says it is already
  in the session. When the panel was stopped (`iohr stop`, "Stop panel", or its host ending),
  a bare `iohr` brings it back first, so running `iohr` again gives a live panel. A shell
  that outlived its panel starts a new one.
- While the panel is stopped, `^G` is not sent to the shell (it would echo as `^G` under a
  command).
- Arrows count in the menu with any modifier (Shift, Option, Ctrl, or the kitty keyboard
  form); a release event is ignored.

`iohr panel [--demo] [-- COMMAND...]` runs the same panel on its own, without a session
host, around your shell or COMMAND. It is hidden from the help. `--demo` fills it with stub
data labelled "stub".

### Keys and the mouse

At rest the panel is a pill: `auto ● on   N waiting   ⌥i menu`. The pill names the key that
works in your terminal:

- `⌥i` where Option is sent as Meta;
- `^G` (Ctrl-G) in Terminal.app, iTerm2, VS Code, Ghostty, WezTerm and Hyper, which type a
  dead key `ˆ` for Option+i by default (found through `TERM_PROGRAM`). Ctrl-G is the same on
  every keyboard layout, means nothing to the terminal driver, and its only shell binding is
  "abort" in readline and zsh.

Both keys always work. For `⌥i`, set Option to send Esc or Meta (iTerm2: Profiles, Keys,
"Esc+"; Terminal.app: "Use Option as Meta key"). If `[ask] menu_key = "ctrl-g"` is set, the
panel leaves Ctrl-G to the `?` menu and shows `⌥i`.

The key opens the menu: `a` auto mode, `p` approvals, `w` workloads, `r` run, `s` status,
`h` share to phone, `t` let the phone type, `?` help, `x` stop the panel. Arrows and Enter move and open, Esc closes. Keys are taken only
while the panel is open; every other byte goes to your shell untouched.

The mouse works on the menu and the views while they are open: a click on a row selects and
opens it, as Enter does; a click outside closes the view; the wheel moves the selection. The
panel asks the terminal for mouse reports (SGR, `?1000h` with `?1006h`, never drag tracking)
only while a view is open, and turns them off when it closes, when the program on the screen
enters the alternate screen, and when the panel ends, stops or panics. Reports are never sent
to your shell, and at rest it asks for none, so drag-to-select works as usual. `[panel]
mouse` (or `IOHR_PANEL_MOUSE`) changes that: `menu` (the default), `always` (a click on the
pill opens the menu and every other click goes through to the program) or `off`.

The chat box (RFC 0212, zsh only) is a framed box of four rows at the bottom of the terminal, kept
clear of your shell's output by a scroll region. `?` at an empty prompt focuses it and `??` asks
deeper; one Enter sends either (the question goes to your shell as if you had typed it, so it is in
your history). While Io answers, the status row shows a spinner, what it is doing ("reading 3
files"), a moving bar with the count and the time (the number of steps is not known, so it is not a
percentage), and a `■ Stop esc` button; Esc or a click on it stops the answer, and Ctrl-C still
does. Shift+Tab changes the answer mode (manual, plan, auto), Up recalls the last question, Esc
clears the line, and a click on the mode chip changes the mode. Option+h (`⌥h`; Alt+h on Linux)
folds the box to a corner chip and brings it back, in every terminal, and the box names it so. So
do `^G` then `c` and a click on `hide`; the chip is a click away, and `[panel] chat = "hidden"` or
`"off"` (or `IOHR_PANEL_CHAT`) sets the start. `Ctrl-]` does the same, for a keyboard layout where
Option+h types another character; it is rebound only inside the panel. A full-screen program has the whole
screen: the box steps aside and comes back after it. While the box shows, the panel takes the
mouse for its buttons, so select text with Option or Shift held.

If something cannot be read, the panel says "unavailable", never a number.

### The views

**Auto mode** (`a`) is the account's auto-approve policy, the one the InOrbit Mac app and
the phone show: on until a time, how many asks it decided, and the kinds it never decides.
Enter turns it off. Turning it on files an ask for 8 hours that goes to your phone; only the
phone sets the policy, so the panel says "asked your phone" and shows the new state at the
next read. A second Enter before the phone answers asks once more.

**Approvals** (`p`) are the [asks](ask.md#the-asks-agents-file-for-you) waiting for you:
`↑↓` pick an ask, `o` or Enter opens its details (`↑↓` scroll, Esc back), `q` types a
question that is sent to the asker, `d` denies after a y/n confirmation, and `a` approves.
The panel never asks for Touch ID itself. An ask a Mac may sign is handed to
`iohr asks approve <id>` in a shell, which asks Touch ID; every other ask goes to your phone.
A merge, a roll, a migration and an auto-approve rule always stay on the phone.

**Share to phone** (`h`) shows this panel's terminal on your phone, and **Let phone type**
(`t`) lets the phone type into it. The panel does not serve the phone itself: it hands its
shell to the InOrbit Mac app's terminal host as an external session (host protocol 2), and the
app's relay, pairing and questions do the rest, so the Mac asks you "Share this terminal?"
before anything leaves it. While it is on, the pill says `phone ● shared` (and `· types`
while typing is on). The row says why in one line when it cannot: the Mac app is not running,
is too old (update it), or the link was lost (an app update restarts the host); turn the row
on again. Typing is off until you turn it on here, and it needs a second yes on the Mac;
a key the phone sends is dropped unless both are on, and the app's own rules for typing (the
terminal is idle, Claude is in front, no permission dialog waiting, one clean line, rate
limits, `IOHR_TERMINAL_INPUT`) still apply before it gets here. The demo shares nothing. Needs
the Mac app; a Linux machine or a server has no app to hand the shell to yet.

**Workloads** (`w`) is the InOrbit Mac app's workload tree, flattened: a coordinator root for
this machine, its children with `├` and `└`, then one "Coordinator service" row for the
queue.

- A dot shows the state: green working, amber needs you, grey idle, red stuck. The header
  counts `N running · M need you`, counting sessions and jobs (not the root and service
  rows), and the menu shows the same number.
- The data is read every 20 seconds from the same places the Mac app reads: the local
  sessions registry `~/.inorbit/agents/sessions/*.json` and the coordinator's jobs and
  queue. A session is listed only while its process is alive; working means seen in the last
  10 minutes, else idle. Its model family and latest assistant line come from the end of its
  transcript, and memory and CPU from `ps`.
- A session needs you when an open ask names it (its name, its id, or an id prefix of 6 or
  more characters).
- If the coordinator cannot be reached, the service row says "unavailable" and the sessions
  still show.

`↑↓` move, Enter opens a row in place (Why, Latest, Since, Uses, Waits, PR, then the
actions), Esc goes back. The actions:

| Key | Action |
|---|---|
| `m` | Message the session: one JSON line `{at, from, to, text}` appended to its `~/.inorbit/agents/inbox/<id>.jsonl` and to `~/.inorbit/agents/log.jsonl` |
| `t` | Bring the Terminal.app or iTerm2 window with that tty forward through `osascript` (macOS only; it says so elsewhere) |
| `l` | The end of its transcript in a scrolling reader |
| `p` | Open its pull request in the browser: a job's own, else the one `gh pr view` finds for the session's branch; it says so when there is none |
| `a` | Its approval |
| `x` | Stop it after a y/n confirmation: SIGINT, a second one after 600 ms, never SIGKILL |

`m` and `t` also work from the list. Under `--demo` the list is a stub tree labelled "stub"
in the title, and every action says "stub: nothing was ..." and does nothing.

### Not supported yet

- The Run and Status views, shown as "coming next".
- A session's checks, worker nodes under a session, and the stuck and failed states of a
  session (only jobs can be stuck), shown as "not available".
- Shells other than zsh are untested (the terminal test uses `/bin/sh`).
- The panel saves and restores the cursor with DECSC and DECRC, which can overwrite a
  program's own saved cursor.
