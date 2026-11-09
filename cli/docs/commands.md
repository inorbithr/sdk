# Command reference

Every `iohr` command in one table, the flags every command takes, how the help works, what
a usage error looks like, and the exit codes. The commands that have their own page are
described there in full; this page describes the rest. Back to the
[iohr overview](../README.md).

## Commands

`iohr --help` lists the commands in five groups, the same five as below. Every command
ends its `--help` with real examples.

### Sign-in

| Command | What it does | More |
|---|---|---|
| `iohr login [--web \| --device] [--account ID\|SLUG] [--insecure-storage] [--scope S,...]` | Sign in and add a profile | [Sign in](sign-in.md) |
| `iohr login --with-token` | Read an API token from stdin and add a profile | [API tokens](sign-in.md#api-tokens) |
| `iohr logout` | Revoke the session and forget the profile on this machine | [Sign in](sign-in.md) |
| `iohr whoami` | Subject, account, plan, scopes and expiry of the active profile | |
| `iohr profile list \| use \| show \| account NAME ID\|SLUG` | The profiles here, the default one, and which account a profile acts for | [Profiles](sign-in.md#profiles) |
| `iohr auth token [--format text\|json]` | The profile's access token, for programs that call the API with your login | [Your login in other programs](sign-in.md#your-login-in-other-programs) |
| `iohr token create \| list \| revoke` | API tokens for an account (a signed-in person only) | [API tokens](sign-in.md#api-tokens) |

### Account

| Command | What it does | More |
|---|---|---|
| `iohr accounts list` | The accounts the credential can see | [Sign in](sign-in.md#which-account-a-sign-in-acts-for) |
| `iohr domains add \| verify \| confirm \| list \| rm` | Prove the account controls a domain with one DNS TXT record | [Domains](domains-and-connections.md#domains) |
| `iohr connectors list \| show` | The catalogue of apps a connection can be made from | [Connections](domains-and-connections.md#connections) |
| `iohr connections ...` | The account's connections and the grants on them | [Connections](domains-and-connections.md#connections) |
| `iohr config set \| get \| unset` | `ext.registry` (a mirror) and `ext.trusted_keys` (keys a mirror re-signs with) | [Extensions](extensions.md#a-mirror) |

### Build

| Command | What it does | More |
|---|---|---|
| `iohr init`, `iohr project init \| sync \| template ...` | Set up a project in this folder, from a template or built by your agent; `iohr init`, `iohr project init` and `iohr project create` are one command | [Projects](projects.md) |
| `iohr atlas roles list \| import [--dry-run]` | The built-in roles and whether each is in your account | [Roles](roles-and-memory.md#built-in-roles) |
| `iohr sync` | Send the project's atlas to your account | [Projects](projects.md#the-atlas-and-iohr-sync) |
| `iohr ext ...` | Find, install, verify, pin and build extensions; `iohr <name> ...` runs one | [Extensions](extensions.md) |
| `iohr sdk add \| config \| generate \| check \| examples` | The SDKs in your project | [SDKs and the API](sdk.md) |
| `iohr api <METHOD> <PATH>` | One API call | [`iohr api`](sdk.md#iohr-api) |
| `iohr openapi pull` | The OpenAPI document this credential sees, to `openapi.json` | [SDKs and the API](sdk.md#iohr-openapi-pull) |
| `iohr lab check` | Check RFC and study files offline | [below](#iohr-lab-check) |
| `iohr decisions ...` | RFCs, PRDs, ADRs and studies in Decisions | [below](#iohr-decisions) |

### Agents

| Command | What it does | More |
|---|---|---|
| `iohr ask ...` | Ask your own AI agent about this folder | [Asking your agent](ask.md) |
| `iohr asks count \| list \| show \| approve \| deny \| question \| decide \| key ...` | The asks agents file for you | [Asks](ask.md#the-asks-agents-file-for-you) |
| `iohr permissions list \| allow \| deny \| remove \| check` | What your agent may run or open when you ask it something | [Permissions](ask.md#permission-rules) |
| `iohr context [DIR] [--full] [--json]` | The IO.md files a `?` or `??` question would load from DIR (default: here), the way a `CLAUDE.md` works: a table of each file with its bytes, how many were sent and why one was cut or left out, farthest first and the nearest winning; `--full` adds the text that would be sent, `--json` the same as data. Reads files only; asks no model and sends nothing | [IO.md](projects.md#iomd-instructions-for-the-folder) |
| `iohr shell hook zsh` | The `?` prompt for zsh | [The `?` prompt](ask.md#the--prompt-in-zsh) |
| `iohr runner register \| run \| status \| config show \| gate-report \| run-key` | This machine as a coordinator runner | [Runner](runner.md) |
| `iohr session [DIR]`, `iohr stop [DIR]` | Open and stop a project's session | [Sessions](session-and-panel.md) |
| `iohr coordinator ...` | The coordinator's jobs, asks and plans | [below](#iohr-coordinator) |
| `iohr memory ...` | Memory kept as local files | [Memory](roles-and-memory.md#memory) |

### Operate

| Command | What it does | More |
|---|---|---|
| `iohr` with no command | On a terminal: your shell inside the project's session, with the floating panel | [The panel](session-and-panel.md#bare-iohr-and-the-panel) |
| `iohr tunnel` | A loopback SSH pipe into an environment's edge | [below](#iohr-tunnel) |
| `iohr browser install [--print]` | Install InOrbit Trails, the browser extension | [below](#iohr-browser-install) |
| `iohr upgrade [--check]` | Update `iohr` the way it was installed | [Upgrade](install.md#upgrade) |
| `iohr completion <shell>` | A completion script for bash, zsh, fish, elvish or PowerShell | |
| `iohr help [COMMAND]` | The help | [below](#the-help) |

Kept but hidden from the help: `iohr packages` (now `iohr ext packages`),
`iohr project templates` (now `iohr project template list`) and `iohr panel`
([the panel](session-and-panel.md#bare-iohr-and-the-panel)).

## Flags every command takes

| Flag | Environment | What it does |
|---|---|---|
| `--profile NAME` | `IOHR_PROFILE` | The profile to act as |
| `--json` | | Results as JSON on stdout |
| `-v`, `--verbose` | | Method, path, status, time and request id of each call on stderr; never a header, a query value or a body |
| `--env production\|local` | `IOHR_ENV` | Which environment to talk to; `--base-url` and `IOHR_ISSUER` win over it |
| `--base-url URL` | `IOHR_BASE_URL` | The API's address, HTTPS only (loopback excepted) |
| `--sign-in io` | `IOHR_SIGN_IN` | `iohr runner` and `iohr coordinator` only: act as Io, the account's agent identity, instead of the profile ([Runner](runner.md#signing-in-as-io)) |
| `--via-role NAME`, `--via-session ID`, `--via-job ID` | `IOHR_VIA_ROLE`, `IOHR_VIA_SESSION`, `IOHR_VIA_JOB` | With `--sign-in io` only: say which role, session or job a call is made for. Recorded with the call for attribution and grants nothing. Each value is 1 to 64 characters of `A-Z a-z 0-9 . _ : -` |

Results go to stdout (tables, or JSON with `--json`) and notes to stderr, so a pipe gets
only data.

## The help

`iohr --help` groups the commands the way `gh` does: Sign-in, Account, Build, Agents and
Operate. `-h` prints one line per command and `--help` the long text. The help is coloured
on a terminal, and plain under `NO_COLOR`, `--json` or a pipe.

## Usage errors

A usage error (exit 2) helps instead of only refusing. It:

1. says in plain words what was missing or wrong;
2. prints the usage line;
3. shows two to four real examples for that command, the same `Examples:` that end its
   `--help`. They come from one table in
   [`src/examples.rs`](../crates/iohr/src/examples.rs), and a test parses every one and
   checks the `iohr api` routes against [`spec/openapi.json`](../../spec/openapi.json);
4. points at `--help`;
5. offers one line to get AI help, `iohr ask '...'`, with the command name and what was
   wrong filled in. That line is an offer: nothing runs and nothing is sent unless you run
   it.

A value you typed is never repeated in that line or in the JSON, because a value may be a
secret. An error `iohr` finds in the arguments names the argument or flag
(`invalid value for --days <DAYS>`), never the value. A usage error a command raises while
it runs may quote what you typed on the terminal, as `iohr profile use NAME` does; that
quote is left out of the AI line and the JSON, which say only that the arguments were
refused. A bare `iohr token` is a usage error too: it lists the subcommands with the
examples, exits 2 and prints nothing on stdout.

Everything goes to stderr. Under `--json` it is one line in the platform's error envelope:

```json
{"code":"usage","error":"`iohr api` is missing <METHOD> and <PATH>","details":[{"type":"usage","command":"iohr api","usage":"...","notes":["..."],"examples":[{"command":"iohr api GET /v1/me","what":"..."}],"help":"iohr api --help","ask_ai":"iohr ask '...'"}]}
```

With no terminal, `NO_COLOR` or `TERM=dumb` it is plain text.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | OK |
| 1 | A finding or a failure: a failed call, a lint finding, a check that did not pass |
| 2 | Usage: a flag or argument the command does not take |
| 3 | Not signed in, or the token was refused |
| 4 | Forbidden by scope, role or plan |
| 75 | `EX_TEMPFAIL`: `iohr runner run` stopped after installing a newer `iohr`, so that its launchd or systemd supervisor starts the new one (`upgrade_restart = "exit"`). Not a failure ([staying current](runner.md#staying-current)) |

These codes are a contract.

## `iohr lab check`

```sh
iohr lab check [PATH...] [--config FILE] [--strict FILE]
```

Checks RFC and study files the way the InOrbit site checks its own
([lab documents](https://docs.inorbit.hr/docs/lab)): file names, front matter (with the
review's `reviewed: YYYY-MM-DD` and `reviewer`), status logs, classified span markers, and
redaction on public documents as a reader without clearance sees them. Text inside
`[[classified:LEVEL reason="..."]]...[[/classified]]` or a
```` ```classified level=LEVEL reason="..." ```` block is exempt; its reason is not.

- `PATH...` defaults to `docs/rfcs` and `docs/studies`.
- `--config` defaults to `docs/lab/redaction.json` when that file exists.
- `--strict FILE` adds a rules file of your own; its findings name the rule, never what
  matched.

It works offline and exits 1 with every finding.

## `iohr decisions`

```sh
iohr decisions list | show | create | save | status | comment | link-pr | review | diagram
```

Documents in the Decisions product, through the API's `/v1/decisions/...` operations, with
the scopes `decisions:read` and `decisions:write`.

- **Naming a document.** By its number: `0042`, or a part, `0042.1`. `PRD 0001` or
  `ADR 0003` names the kind; otherwise `--kind rfc|prd|adr|study|all` (default `rfc`).
  It is looked up across the account's spaces, and `--space` narrows that; `--account`
  acts on a team.
- **`create`** makes an RFC, or with `--kind prd|adr` a PRD or an ADR. With `--file` it
  saves the text at once.
- **`save`** writes a new version from a file, and is refused when someone saved since you
  read it.
- **`status`** sets the document's status.
- **`comment`** and **`link-pr`** write comments; `link-pr` names a pull request first
  (`owner/repo#N "title": open`, or `merged` or `live` with `--merged` or `--live`).
- **`review`** asks for a review: `--reviewer` (or `IOHR_DECISIONS_REVIEWER`), and
  `--reason` posted as a comment.
- **`diagram`** sets a diagram from a model file.

Each one takes `--json`.

## `iohr coordinator`

The coordinator's jobs, queued asks and plans, for people and runners that work with it.
These commands need a person's `iohr login`, not an API key, or `--sign-in io` to act as
Io. `--account ID` acts on a team you belong to; the default is your own account.

- **`jobs list | create`** lists jobs by state (`--limit`, 1 to 200, default 50) and files
  one; `--brief -` reads the brief from standard input and `--ask` links a queued ask.
- **`asks list | create`** lists the queued asks and queues one:
  `asks create --kind K --body -`. The kind is required so the planner can file it.
- **`brief JOB`** prints the job's brief (task, acceptance, files, document excerpt, capped
  at the kind's token budget) for a worker to start from; the size goes to stderr.
- **`plan list | show | create`**: `plan create TITLE --external-id project:NAME` makes a
  plan, or returns the one with that id (the output says `created` when it made one);
  `plan show PLAN` lists its items and latest notes.
- **`item PLAN ITEM add | claim | release | note | done | fail | run`** writes one item:
  - `add` sets the title (1 to 200 characters), kind (default `task`) and body; an
    existing item keeps what you do not set;
  - `claim` takes a lease for one holder (default `iohr:` and your profile) for
    `--seconds`, 1 to 86,400 (default 3,600). Another holder's live claim is refused, and
    `--force` takes it over. `release` gives the claim up;
  - `note` writes 1 to 2,000 characters, or `-` for standard input;
  - `done` finishes the item, with `--note` for a closing note;
  - `fail --reason TEXT` notes the failure, sets the item back to `proposed` and releases
    it. The Planner has no failed status, so the item is ready to be claimed again;
  - `run PLAN ITEM -- COMMAND` runs a build step as an item. It adds the item when it is
    missing, claims it, runs the command, then notes it and marks it done, or notes the
    failure and puts it back. An item that is already `done` is skipped, so a build
    resumes. The command's exit code is this command's (1 when it could not start). The
    note names the program, never its arguments.

## `iohr tunnel`

```sh
iohr tunnel --host HOST [--env production|local] [--port P] [--no-engine] [--dry-run]
```

For people with SSH access to an environment's host: a loopback pipe (`ssh -N -L`) from
this machine to `HOST` (or `IOHR_TUNNEL_HOST`), bound to `127.0.0.1` on both ends, so the
site opens at `http://www.localhost:<port>/` (also `console`, `docs` and `auth`) with no
DNS or certificate. It ends at the environment's edge proxy, so every check there still
applies.

- `--port` moves the listening side; `--edge-port` and `--engine-port` name the far side.
- `--no-engine` skips the second forward the local environment also has.
- `--dry-run` prints the addresses and the `ssh` command and runs nothing.

Not supported: signed-in pages. Sign-in cookies belong to the production domain, so they do
not work on `localhost`.

## `iohr browser install`

Opens the console's install page for InOrbit Trails, the browser extension, in your
default browser. The page adds Trails from that browser's store (Chrome Web Store, Edge
Add-ons or Firefox Add-ons), or shows the steps to load it unpacked while a store has no
listing yet. `--print` prints the address instead, for a browser on another machine or
where none can open.
