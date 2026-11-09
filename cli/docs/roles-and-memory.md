# Roles and memory

The built-in roles a project gets, how a run uses one with `--role`, and `iohr memory`, the
project's local, file-based memory. Back to the [iohr overview](../README.md).

## Built-in roles

`iohr project init` and `iohr atlas init` also write the built-in roles into
`atlas/roles/`:

- `security`, which every project gets whatever its language;
- one engineer for the project's language: `rust-engineer`, `python-engineer`,
  `typescript-engineer` or `go-engineer`. `atlas init` reads the language from
  `inorbit.toml` or the build file.

Each file has front matter (`name`, `parent`, `domain`, the catalogue it came from) and its
lines: a core of at most 700 tokens, and recall lines found by topic, each with the primary
source it comes from (NIST SSDF, OWASP ASVS, the Rust API guidelines, the PEPs and the
like). The catalogue is data in the binary, labelled origin `doctrine`, and `inorbit.toml`
records the names in `[roles]`.

### Importing them into your account

Signed in, init imports them into your account and prints one line each: `written v1`,
`upgraded v1 → v2`, `unchanged` or `left alone`.

- A role already in your account that an earlier project put there is read from its
  version history. When every version of it was an import and its lines are an older
  catalogue's, nobody changed it, and it is upgraded (`v1 → v2`).
- A copy you changed (a version that is not an import, or lines of your own), or one you
  changed in InOrbit since this project's import, is left alone. The line says the
  catalogue's version differs and names the command that takes it,
  `iohr atlas roles import --upgrade`, which is your go to replace the changed copy.
- Provenance is read from that history; there is no built-in marker on the server, so a
  role whose history cannot be read stays `left alone`. The catalogue never replaces your
  changes on its own.
- When the account has no root role, `base-engineer` comes with them.
- Only the account owner or an admin may import.

Signed out, offline or refused, init still succeeds, says so once and keeps the files;
`iohr sync` (or `iohr atlas roles import`) imports later. `--json` reports each role.

The folder was called `atlas/personalities/` before. `iohr atlas init`, `roles list`,
`roles import` and `iohr sync` move it to `atlas/roles/` once and say so in one line; a file
that `atlas/roles/` already has is never overwritten. `iohr atlas personalities` still works
as a hidden alias of `iohr atlas roles`.

### Dates on facts

Every built-in fact carries `checked: 2026-11-06`, the day it was last read against its
primary source. Every prompt that asks a model to research or write facts
(`project init`, `--describe`, the agent step) ends with the system clock's date: "Today is
YYYY-MM-DD. Use the newest edition or version published as of today; name it with its
publication date; if you are unsure a newer one exists, say so."

## Use a role in a run: `--role`

`iohr ask --role NAME` and `iohr init --role NAME` (the same as `iohr project init`) run the
agent in a role. The role is looked up in this order:

1. the project's `atlas/roles/NAME.md`;
2. your account, when you are signed in;
3. the built-in catalogue in the binary, so `--role rust-engineer` works offline.

A dry run never asks the account. A name that is nowhere ends the run before anything
starts (exit 2).

What the role changes:

- **Its text.** Claude Code gets the role's core (the "Core" section of the file, or the
  account's compiled core, at most 8,000 characters, cut with a note when longer) through
  `--append-system-prompt`; the streamed `?` run puts it first in the prompt in a `<role>`
  block. Codex has no system-prompt flag, so the text leads the question. The role guides
  how the agent works; it never widens the run's permissions, edits or limits.
- **Its model.** The order is the `--model` flag, then `[ask] model` in the config, then
  the role's own model, then the default (`haiku` for `ask`, the project agent's default
  for `init`). Codex ignores the role's model. A role model that is not a plain model name
  is ignored, not an error.
- **Its brief for a job.** `iohr ask --role NAME --job JOB_ID` (signed in) asks the account
  for the role's brief for that job (the job id goes in the request body, never the URL)
  and uses it in place of the role's own text. When the brief is not available the run
  goes on with the role's text and says so in one line (the status only).

With `--json` the role is named: `ask --dry-run` has `role` beside `args`, the
`iohr_summary` line and the headless answer carry it, and `init`'s agent step and report
carry it, each as `{name, source: project|account|catalogue, brief: role|job, model, job}`.
The role's text is never in that object.

## Memory

`iohr memory` is local first: files, no database. Each memory is one Markdown file with
front matter (`id`, `kind`, `scope`, `created`, `updated`, `checked`, `why`, `pinned`,
`refs`) and the text as the body. There are three scopes; `--scope` picks one per command
and `[memory] scope` in the config sets the default (else `project`):

| Scope | Where | Notes |
|---|---|---|
| `project` | `<repo root>/.inorbit/memory/` | Committed with the code. The repo root is the nearest folder up with `.inorbit/`, `atlas/` or `.git` (the home folder's own `.inorbit/` is not one). Outside a project the default falls back to `user`, said on stderr; `--scope project` there is an error |
| `user` | `~/.inorbit/memory/notes/` | Yours, on this machine |
| `account` | InOrbit | Needs `[memory] sync = true`; the only scope that is not a file |

```sh
iohr memory remember "We use sqlx" --why "typed SQL" --ref src/db.rs   # offline; --preference, --pin, --scope
iohr memory recall "database"          # ranked by word match, recency (preferences never decay) and pin
iohr memory list
iohr memory forget <id>
iohr memory check [--strict]           # --strict exits 1 on any finding
iohr memory drift [--remote] [--strict]
iohr memory sync [--dry-run] [--pull]  # off until [memory] sync = true or you run it
```

A memory is never printed to a log; `--json` prints it to stdout only, and text is never in
a URL. A text that looks like a secret is refused.

### `iohr memory check`

| Finding | Means |
|---|---|
| `dangling-ref` | A path that is gone, or a `cmd:` program not on `PATH` |
| `bad-ref` | An absolute path or one with `..` |
| `missing-why` | No `why` |
| `stale` | Not confirmed in 180 days (`--stale-days`); pinned ones are exempt |
| `unchecked` | No `checked:` date, or one older than `--checked-days`, else `[memory] check_days`, else 180 days; pinned ones are exempt |
| `never-confirmed` | Never confirmed again since it was written, for a third of the stale limit |
| `duplicate`, `near-duplicate` | The same text, or text that overlaps another's |
| `gap` | Topics no memory covers: the language, the build and test commands, the owners |

### `iohr memory drift`

`drift` only reads. It compares each `--ref` with `git log` and `git status`:
`ref-changed` when the path changed after the memory was last updated, `ref-uncommitted`
when it has edits not committed. It says it skipped when the folder is not a git
repository, and it asks the service only when sync is on or `--remote` is given.

### `iohr memory sync`

`sync` (also run by `iohr sync` when `[memory] sync = true`) shares the `project` or `user`
memories with your account, tagged `project:<slug>` or `user`. The account's memory service
has create, list, recall and delete, and no update or version, so the check for a change on
the other side is made here: `remote_sha` (a hash of kind and text) and `remote_version`
are kept in the file's front matter, local only. Per memory the plan is one of:

| Plan | What happens |
|---|---|
| `push` | New: it is created |
| `link` | The service already has the same text; the ids are written back |
| `replace` | Changed here: the new copy is created, the ids are written back, then the old one is deleted. This is create-then-delete, not atomic, and a failed delete is reported |
| `remote newer`, `remote gone` | Reported; nothing is overwritten |
| `conflict` | Changed on both sides: reported, exit 1, nothing overwritten |

Entries only on the service are written as files only with `--pull`. `--dry-run` sends
nothing. Signed out, `sync` is a sign-in error and the files stay as they are.

### Seed memories

`iohr project init` and `iohr atlas init` write `.inorbit/memory/README.md` and the
built-in roles' recall lines as seed memories, locally, with no network; a seed you forget
stays forgotten. The agent-instructions file (`AGENTS.md` or `CLAUDE.md`) gets a short
section that tells agents to run `iohr memory recall` and `iohr memory remember`.
