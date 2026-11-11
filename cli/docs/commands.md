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
| `iohr atlas search QUERY [--site S] [--limit N]` | The best chunks of the text of the sites you crawled, each with its page, heading path and offsets (local) | [Projects](projects.md#the-site-knowledge-base) |
| `iohr atlas facts [--site S] [--predicate P] [--grep TEXT]` | The facts of the site model, each with the chunk it came from | [Projects](projects.md#the-site-knowledge-base) |
| `iohr atlas claims [--site S] [--prove ID] [--limit N]` | The claims behind the facts, each with its support; `--prove` prints the proof tree down to the page and checks the page still matches its hash | [Projects](projects.md#the-site-knowledge-base) |
| `iohr atlas index [--pages] [--site S]` | The crawled sites with pages, chunks, words, entities, facts and links; `--pages` lists the pages | [Projects](projects.md#the-site-knowledge-base) |
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
| `iohr theme list \| show NAME` | The terminal themes of the InOrbit app (`inorbit`, `solarized-dark`, `nord`, `light`), the one that is on, and a theme's colours, font and cursor; local, no sign-in | [The terminal theme](terminal-theme.md) |
| `iohr theme install NAME [--terminal] [--prompt] [--replace-prompt]` | Writes the Terminal.app profile `~/.inorbit/term/themes/NAME.terminal` and remembers the theme; `--terminal` opens the profile, `--prompt` turns on the zsh prompt, `--replace-prompt` replaces a prompt you set yourself | [The terminal theme](terminal-theme.md) |
| `iohr theme off` | Turns the prompt off and forgets the chosen theme; an imported profile stays | [The terminal theme](terminal-theme.md) |
| `iohr runner register \| run \| status \| config show \| gate-report \| run-key \| api-key \| personal-use` | This machine as a coordinator runner | [Runner](runner.md) |
| `iohr session [DIR]`, `iohr stop [DIR]` | Open and stop a project's session | [Sessions](session-and-panel.md) |
| `iohr coordinator ...` | The coordinator's jobs, asks and plans | [below](#iohr-coordinator) |
| `iohr memory ...` | Memory kept as local files | [Memory](roles-and-memory.md#memory) |

### Operate

| Command | What it does | More |
|---|---|---|
| `iohr` with no command | On a terminal: your shell inside the project's session, with the floating panel | [The panel](session-and-panel.md#bare-iohr-and-the-panel) |
| `iohr ui [serve] \| build \| dev \| check \| open` | Serve the built web UI on loopback, or run its package's own scripts | [below](#iohr-ui) |
| `iohr tunnel` | A loopback SSH pipe into an environment's edge | [below](#iohr-tunnel) |
| `iohr browser install [--print]` | Install InOrbit Trails, the browser extension | [below](#iohr-browser-install) |
| `iohr doctor [--fix]` | Check this machine for the known problems (signed out, MCP bridge down, low disk, unreadable `config.toml`), say how to fix each, exit 1 on a failure. `--fix`, or `[doctor] auto_fix = true`, runs the safe fixes (restart a stopped bridge); the `?` card shows what it found | [below](#iohr-doctor) |
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

On a terminal, lists and `show` views are drawn: the header in the help colour, ids dimmed so
names lead, a `STATUS` or `STATE` cell as a mark and its word (a green check for `active` or
`done`, an amber `!` for `pending` or `expiring`, a red cross for `failed` or `revoked`; a word
not on that list stays plain), keys dimmed, and every line indented two spaces as `iohr
doctor` is. Under `NO_COLOR`, `TERM=dumb`, `--json` or a pipe the bytes are the same as
before: columns two spaces apart, no indent, no marks.

A list (`token list`, `connections list`, `domains list`, `ext list` and the other `list`
commands) ends on a terminal with a line that counts it and, where one fits, names the next
command (`2 tokens · new one: iohr token create`). An empty list there is one line, such as
`No tokens yet. Next: iohr token create`. Plain output keeps the table alone, and an empty
list keeps its header line, so scripts read the same bytes.

The other checks are drawn the same way on a terminal: `iohr lab check`, `iohr memory check`,
`iohr memory drift` (its code part), `iohr sdk check` and `iohr ext verify`. Each problem is a
row with its mark, where it is, what is wrong, and labelled lines under it (the `rule` that
found it, the operations a moved profile gained `+` and lost `-`, the `fix`). A memory finding
is an amber `!`, since it fails only under `--strict`. The last line says what was checked and
what needs doing (`12 documents checked · 3 to fix`), or a green check with `nothing found`.
The plain lines, tables and `--json` stay as they are under `NO_COLOR`, `TERM=dumb` or a pipe.

## `iohr doctor`

Looks at four things and prints one line each: `ok`, or `FAIL` with the fix on the next line.
On a terminal each line starts with a coloured mark (a green check or a red cross), the fix
sits under the line it fixes, a passing line is dimmed, and a count closes the list
(`3 ok · 1 to fix`). Under `NO_COLOR`, `TERM=dumb`, `--json` or a pipe the output is the plain
lines above, unchanged, so scripts read the same text.

| Check | Fails when | Fix |
|---|---|---|
| `auth` | no usable sign-in on this machine | `iohr login` (shown, never run) |
| `bridge` | the MCP bridge job exists but nothing answers on its port | restart it (the one safe fix) |
| `disk` | less than 2 GiB free | free space (shown, never run) |
| `config` | `config.toml` does not parse | edit it (shown, never run; `iohr` never rewrites it) |

A machine without the bridge installed is not a failure. `--json` prints status codes
(`ok`, `signed_out`, `down`, `low`, `invalid`) and counts, never file contents or tokens. Exit
1 when a check fails after any fix. Turn on the safe fixes for good with
[`[doctor] auto_fix = true`](../../docs/config.md#49-the-doctor-table); the `?` and `??` card
then also names what it found, from the local checks only (it never asks the Keychain).

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

## Other errors

Any other error (a refused call, a file that cannot be written) also goes to stderr. On a
colour terminal it is drawn as a block: a red mark and the message, then a line each for
the API's error code and HTTP status, the request id, and a hint when `iohr` has one:

```text
✗ the token cannot read this account's keys
  code     forbidden · HTTP 403
  request  req_01H...
  hint     an API token needs the account:read scope
```

In a zsh with the `?` hook (`iohr shell hook zsh`), the block ends with one more line,
`? why did that fail`, which you can type to ask your own agent about the command. It is
not shown when `INORBIT_ASK=0`.

With no terminal, `NO_COLOR` or `TERM=dumb` it stays one plain line, `iohr: <message>`,
with the hint on the line after it. Under `--json` it is one line in the same envelope as a
usage error: `code` is the API's code for a refused call (else `failed`, `unauthenticated`,
`connection`, `timeout` and the like), and a `failure` detail carries the other lines, the
HTTP status, the request id and the hint when there are any:

```json
{"code":"forbidden","error":"the token cannot read this account's keys","details":[{"type":"failure","notes":[],"status":403,"request_id":"req_01H...","hint":"an API token needs the account:read scope"}]}
```

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

## `iohr ui`

```sh
iohr ui [serve [--port P] [--dir D] [--open]]
iohr ui build|dev|check [--package DIR]
iohr ui open
```

`serve` serves the built web UI (`sdk/ui`, RFC 0217) on loopback: `127.0.0.1`, else `[::1]`,
never another address. `--dir` (or `IOHR_UI_DIR`) names a built `dist/`; without it the
nearest `ui/dist` above the working folder is used. With no verb it serves and opens the
browser (`IOHR_NO_BROWSER` prints the address instead).

- Pages are GET and HEAD only. The `Host` must be this listener's own address or `localhost` on its
  port. Paths cannot leave the folder or follow a link.
- `/api/*` answers only this listener's own pages (`Sec-Fetch-Site`, `Origin`).
  `GET /api/extensions` lists the installed extensions that declare a `[panel]` (name,
  version, title, entry) from the same store `iohr ext install` writes, and starts none of
  them.
- Reaching the page takes a per-launch secret (RFC 0217, round 4). `iohr ui open` and
  `serve --open` put it in the first address (`/?launch=...`); the server answers `303` to
  `/` and sets a session cookie (`HttpOnly`, `SameSite=Strict`, named by the port), and
  every other request without that cookie is `401`. The secret is one-shot: only a GET or
  HEAD can use it, the first use spends it, and a second use is `401`. The server then makes
  a fresh one and rewrites it in `ui/serving.json` (private), so the next `open` still gets
  in. `serve` never prints the secret; `open` prints the address only when no browser could
  be opened.
- Error text the page can see (a failed sign-in, a failed proxy call) goes through the same
  redaction as `iohr ask` output: values after a secret's name, bearer values, JWTs, tokens
  and URLs are replaced.
- The page never holds a credential. `iohr ui serve` signs in as `iohr login` does (the
  same profile, the same credential store; `--profile`, `--env` and `IOHR_TOKEN` apply) and
  calls the API for the page: `GET /api/session` says who (never a token),
  `POST /api/session/login` starts `iohr login --web` (the system browser, PKCE) and
  `POST /api/session/logout` is `iohr logout`, and `/api/proxy/v1/...` forwards one call
  to the API with that credential. Each needs the page's own `x-iohr-token` header.
  The proxy is under `/v1/`, with plain path segments, and never takes the WebSocket path.
- Reads (`GET`, `HEAD`) need the token and this page's origin. Writes (`POST`, `PUT`,
  `PATCH`, `DELETE`) need the same plus `Sec-Fetch-Site: same-origin` and an `Origin`
  equal to the serve's own address; the token travels in the header only, never a cookie
  or an address. A write goes only to the areas a view manages (accounts, approvals, atlas,
  coordinator, decisions), takes a JSON body of at most 1 MiB
  that arrives within 10 s (`Content-Type: application/json`, no chunked bodies; `DELETE`
  and reads take none) and is answered within 125 s. Bodies and tokens are never logged.
- A `GET` with `Accept: text/event-stream` passes the API's server-sent events through
  as they arrive: each event is flushed at once, the API's resume cursor (`sse_id`)
  becomes the event `id`, a `: keepalive` comment goes out every 15 s, and a stream with no
  event for 5 minutes is closed (the page reconnects). At most 4 streams run per serve
  (the 5th gets `429`; a browser keeps six connections per origin); a stream ends when the
  page goes away or the serve stops. The page's client is `apiStream` in `ui/src/lib/api.ts`
  (reconnect with backoff, one shared connection per identical stream) and `useLive`.
- `GET /api/live` is one WebSocket for all of the page's streams, so six open streams do
  not use up the browser's six connections. It is for the routes the API has a socket
  call for (coordinator picture, asks, thread and open items, approval events, atlas
  signals, events); `apiStream` uses it for those and reads every other path, and any
  stream when the socket cannot open, as server-sent events. The serve carries the calls
  to the API on one `/v1/ws` through the SDK's `Streams::Socket`. The upgrade needs the
  session cookie, the `Host`, `Sec-Fetch-Site: same-origin` and an exact `Origin`. A
  browser cannot set a header on a WebSocket handshake, so the per-serve token is the first
  frame (`{"type":"auth","token"}`, constant-time compare, nothing is read before it; a
  wrong one closes the socket). Then `{"type":"call","id","path":"/v1/..."}` and
  `{"type":"cancel","id"}` go in; `data`, `end` and `error` frames (with the call's `id`)
  come out, `unmapped` for a path with no socket call. A socket takes one of the 4 stream
  slots, carries at most 16 calls, accepts frames of at most 64 KiB, is pinged every 15 s,
  and closes after 5 minutes with nothing either way or when the serve stops. Nothing it
  carries is logged.
- A taken `--port` is an error. A running `serve` records its address in the data folder
  (`ui/serving.json`, private) and removes it when it stops.

`build`, `dev` and `check` run the UI package's own `pnpm run build`, `dev` or `check` in
the nearest `ui/` holding a `package.json` (or `--package`), without a shell, and pass its
exit code on. The mise tasks `ui:build`, `ui:dev`, `ui:check` and `ui:serve` call these
verbs.

`open` opens the UI the running `serve` recorded, only when it is a loopback address that
still answers; a stale record is removed. `IOHR_NO_BROWSER` prints the address instead.

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
