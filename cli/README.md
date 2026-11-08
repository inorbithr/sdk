# iohr, the InOrbit command line

Pre-release (0.x): commands and output can still change between versions. This page
describes `main`; a change merged after the latest pre-release ships with the next one
([CHANGELOG.md](CHANGELOG.md), [releases](https://github.com/inorbithr/sdk/releases?q=iohr)).

Guides for each part, with captures of real output, are on the developer site:
[what iohr can do](https://developers.inorbit.hr/docs/tools/iohr/),
[projects](https://developers.inorbit.hr/docs/tools/projects/) (`iohr project init`, the
atlas, memory, `iohr sync`), [the `?` prompt](https://developers.inorbit.hr/docs/tools/ask/),
[extensions](https://developers.inorbit.hr/docs/tools/extensions/) and
[iohr and iohr-preview](https://developers.inorbit.hr/docs/tools/preview/). This README is
the reference they link back to.

`iohr-preview` is the next build of this program under another name: it carries changes that
are checked but not yet in a release (today the `?` mode switch, the extension offer in
`iohr project init` and `iohr ext show` with no name). It is not published as a package.

```sh
brew trust inorbithr/tap                                          # once per machine: Homebrew refuses untrusted taps
brew install inorbithr/tap/iohr                                   # macOS, Linux
curl -fsSL https://packages.inorbit.hr/install.sh | sh            # Linux, macOS, no sudo
powershell -c "irm https://packages.inorbit.hr/install.ps1 | iex" # Windows
```

To update, run `iohr upgrade` (`iohr upgrade --check` only says whether a newer release exists
and what would run). It follows how iohr was installed: a Homebrew copy runs
`brew upgrade inorbithr/tap/iohr` (and tells you to run `brew trust inorbithr/tap` when
Homebrew refuses the tap); a copy from `install.sh` is downloaded, checked against the
release's `SHA256SUMS` and its GitHub build attestation (checked inside iohr, so the GitHub
CLI is not needed, and a release without one is refused), and replaced in place; an APT or
other copy is told what to run. The Homebrew tap can lag a release until its formula pull
request is merged.

Debian and Ubuntu (a signed APT repository) and every other way are on
[docs.inorbit.hr/docs/command-line](https://docs.inorbit.hr/docs/command-line/). winget
is not one yet: its manifests are built with every release and submitted with the first
stable one ([releasing](../docs/releasing.md)). The
installers ([install.sh](install/install.sh), [install.ps1](install/install.ps1)) are short
enough to read first: each downloads the archive for your machine from the GitHub release,
refuses it unless its SHA-256 matches the release's `SHA256SUMS` (and, with the GitHub CLI
signed in, unless its build attestation checks), and copies one binary into your user
directory. Every release also has archives, `.deb` and `.msi` files with provenance and
SBOMs ([verifying releases](../docs/security/verifying-releases.md)). From source:

```sh
cargo install --locked --path cli/crates/iohr
```

## Sign in

```sh
iohr login
iohr whoami
```

`iohr login` signs you in on InOrbit's own sign-in pages. On a machine with a browser it
opens one and waits up to 5 minutes for it to come back to `http://127.0.0.1:<port>`.
Over SSH, in a container or without a display it prints a link and a code instead:
open the link in any browser, on any device, and enter the code. `--web` and `--device`
choose one. Only enter a code you started yourself. `IOHR_NO_BROWSER=1` (any non-empty
value) makes every command behave as if no browser can be opened and print its link
instead; the test suites set it.

The sign-in always starts on your own account, so `iohr login` then picks the account the
profile acts for, first match wins: `--account <id|slug>` (a team you belong to), the
account this profile was already pointed at (`--account` or `iohr profile account`, kept
across every sign-in after), your default account (the `default_account_id` preference,
`iohr api PUT /v1/accounts/me/preferences/default -f account_id=<id>`), your own account.
It prints the account it chose and how to change it; `iohr profile show` says whether the
choice is kept. A default that you no longer belong to is skipped with a note.

A signed-in session holds a 15-minute access token and a 30-day refresh token that is
replaced on every use; `iohr` refreshes on its own. `iohr logout` revokes the session at
the sign-in service before removing it from this machine. The console's sign-in devices
list shows the session and can end it from anywhere.

An API token works too. Create one in the console (API tokens and keys) or with `iohr
token create`, then pipe it in; a token is never accepted as an argument, where shell
history and the process list would keep it.

```sh
iohr login --with-token --profile ci < token.txt
```

For CI and one-off use, `IOHR_TOKEN` is used in memory and nothing is written. It wins
over the default profile; a profile named with `--profile` or `IOHR_PROFILE` wins over it
(a named profile reads only `IOHR_TOKEN_<PROFILE>` from the environment):

```sh
IOHR_TOKEN="$(cat token.txt)" iohr api GET /v1/radar/digests -f page_size=5
# every page of a list as one answer (at most 100 pages; --max-pages N)
iohr api GET /v1/webhooks/endpoints --all
```

## Commands

| Command | What it does |
|---|---|
| `iohr login [--web \| --device] [--account ID\|SLUG] [--insecure-storage] [--scope S,...]` | Sign in and add a profile; `--account` acts for one of your teams instead of your default account (checked against your accounts, kept across later sign-ins); `--insecure-storage` keeps the token in a 0600 file on a machine without a credential store; `--scope` asks for scopes besides the defaults, such as `coordinator:runner,coordinator:read` for `iohr runner` (the sign-in service refuses a scope the client may not ask for) |
| `iohr login --with-token` | Read an API token from stdin and add a profile |
| `iohr logout` | Forget the profile and its credential on this machine |
| `iohr profile list \| use \| show` | The profiles here; the default one |
| `iohr whoami` | Subject, account, plan, scopes and expiry of the active profile |
| `iohr auth token [--format text\|json]` | The profile's access token on stdout, refreshed first when less than a minute is left; for programs, such as the SDKs, that call with your login (below) |
| `iohr accounts list` | The accounts the credential can see |
| `iohr token create --name N --scope S,... [--days 30] \| list [--status S] [--query Q] \| revoke ID` | API tokens for an account (a signed-in person only) |
| `iohr api <METHOD> <PATH>` | One call; `-f k=v` string fields, `-F k=json` typed fields, `--input file`, `-i` the status and request id before the body; `--all` GETs every page of a list (`next_page_token`) as one answer, `--max-pages N` bounds it |
| `iohr openapi pull` | The OpenAPI document this credential sees, to `openapi.json` |
| `iohr memory remember TEXT [--why W] [--ref PATH]... [--preference] [--pin] [--scope S] \| recall QUERY \| list \| forget ID \| check [--strict] \| drift [--remote] [--strict] \| sync [--dry-run] [--pull]` | Memory kept as files, local first: one Markdown file per memory in `.inorbit/memory/` (the project, committed), `~/.inorbit/memory/notes/` (you) or, with `[memory] sync = true`, your InOrbit account. No sign-in and no network except `sync`, `drift --remote` and `--scope account`. `check` finds dangling refs, a missing why, stale and never-confirmed, duplicate and uncovered topics; `drift` compares with the code (git) and the service; `sync` never overwrites |
| `iohr atlas roles list \| import [--dry-run]` | The built-in roles in `atlas/roles/` and whether each is in your InOrbit account; `import` sends them (`--dry-run` prints the body and sends nothing) |
| `iohr atlas search QUERY [--site S] [--limit N]` | The best chunks of the text of the sites you crawled, each with its page, heading path and offsets (local; `--json` for data) |
| `iohr atlas facts [--site S] [--predicate P] [--grep TEXT]` | The facts of the site model, each with the chunk it came from |
| `iohr atlas index [--pages] [--site S]` | The crawled sites with pages, chunks, words, entities, facts and links; `--pages` lists the pages |
| `iohr sdk generate --lang rust\|typescript\|python\|go\|java\|csharp\|swift --for P... --out DIR` | A surface cut to what the profiles may call, into your repository, with `iohr.lock` beside the directory; `--from NAME=FILE` works offline, `--package` names the surface's package or namespace (Go, Java, C#), `--runtime` the runtime's name in your project, `--force` replaces a non-empty directory |
| `iohr sdk config [--profile NAME] [--for TYPE]` | The configuration an SDK client built with `load` would use here: each setting with its source, the credential chain, the pipeline, what was ignored; secrets redacted; offline (below) |
| `iohr sdk check [--lock FILE] [--files]` | Fetch every profile's document again and exit 1 with what moved when the cut changed; for CI, `IOHR_TOKEN_<PROFILE>` stands in for a profile |
| `iohr sdk add [rust\|typescript\|python\|go] [--version V] [--dry-run]` | Add the published SDK to the project here with the package manager it already uses; the language comes from the project's files when left out (below) |
| `iohr sdk examples --from [NAME=]FILE [--lang L]... [--out FILE]` | One short program per operation and language that calls it with the published runtime, as JSON keyed by operation id; what the API reference shows beside each operation |
| `iohr profile account NAME ID\|SLUG` | Point a signed-in profile at one of its teams, the account `sdk generate` cuts to; a later `iohr login` keeps it |
| `iohr coordinator jobs list \| create`, `asks list \| create` | The coordinator's jobs and queued asks (RFC 0120, RFC 0138): list jobs by state, file a job (`--brief -` reads standard input; `--ask` links a queued ask), list asks, queue an ask (`asks create --kind K --body -`; the kind is required so the planner can file it); `--account <id>` on `jobs list|create` and `asks list` acts on a team you belong to (default: your own account; needs the coordinator release that accepts `account_id`). `iohr coordinator brief JOB` prints the job's brief (task, acceptance, files, document excerpt, capped at the kind's token budget, RFC 0132) for a worker to start from; the size goes to stderr. Needs a person's `iohr login`, not an API key, or `--sign-in io` to file as Io (below) |
| `iohr coordinator plan list \| show \| create`, `item add \| claim \| release \| note \| done \| fail \| run` | The Planner's plans, items, claims and notes (RFC 0166, RFC 0211). `plan create TITLE --external-id project:NAME` makes a plan or returns the one with that id (the output says `created`); `plan show PLAN` lists its items and latest notes. `item PLAN ITEM` commands write the item: `add` (title, kind, body; an existing item keeps what you do not set), `claim` (default holder `iohr:<profile>`, 3600 s; another holder's live claim is refused, `--force` takes it), `note` (text or `-` for standard input, up to 2000 characters), `done` (`--note`), and `fail --reason TEXT`, which notes the failure, sets the item back to `proposed` and releases it (the Planner has no failed status). `item run PLAN ITEM -- COMMAND` is a build step as a Planner item: it adds the item when missing, claims it, runs the command, then notes and marks it done, or notes the failure and puts it back; an item already `done` is skipped (a build resumes), and the command's exit code is this command's. The note names the program, never its arguments. `--account <id>` acts on a team. Needs a person's `iohr login` |
| `iohr domains add \| verify \| confirm \| list \| rm` | Prove the account controls a domain with one DNS TXT record; `verify --wait` checks every 10 s for up to `--timeout` seconds (600) |
| `iohr connectors list [--category C] \| show ID` | The catalogue of apps a connection can be made from: sign-in modes and their fields, settings, actions, the hosts each may call, AI models labelled |
| `iohr connections list \| show \| add \| test \| history \| pause \| resume \| rename \| delete \| reconnect` | The account's connections (RFC 0044): connect an app with a key (asked for without echo) or by signing in at the provider in a browser; `delete` asks first unless `--yes` |
| `iohr connections grant \| grants \| revoke-grant` | Grant a product, an API key or an agent named actions of a connection until an expiry, list and revoke grants |
| `iohr ext search \| show` | The extensions catalogue (RFC 0073): `search [QUERY] [--kind K] [--all \| --page-size N]` lists name, kind, publisher, latest version and visibility; `show` with no name lists the same catalogue, listings not yet released marked coming; `show PUBLISHER/NAME` the description, publisher, signer, scopes, privileges in plain words, evidence, versions and the install line. API host only, `extensions:read` |
| `iohr ext install [--lock FILE] [--no-service] \| service \| list \| upgrade \| remove \| verify \| sync` | Extensions: install (`NAME` from the registry, or `PUBLISHER/NAME` through the catalogue), verify and pin them; one that declares privileges for its system service asks first unless `--yes`; one with a system service of its own (`inorbit/capture`) offers to set it up with sudo after installing (it lists the interfaces it detects and suggests one; `--interface` repeatable, or `--all-interfaces`), and `iohr ext service NAME` does it later; `iohr <name> ...` runs one |
| `iohr ext init \| dev \| lint` | Build an extension locally (PRD 0084): `init PUBLISHER/NAME --lang rust\|python\|shell [--dir DIR]` scaffolds a folder (manifest with every field, entry point on the token channel, self-test, README, signing release workflow; a name holding `tbd` is refused); `dev [PATH] [--test]` runs it, or its tests, with the token channel for exactly the manifest's scopes, its secrets, `STATE` and `CACHE` under `~/.inorbit/ext-dev/NAME/` and its output timestamped, never touching installed extensions; `lint [PATH] [--preview] [--json]` checks `extension.toml` and says what it may do in plain words. Offline, except the token channel's sign-in |
| `iohr config set \| get \| unset` | `ext.registry` (a mirror) and `ext.trusted_keys` (keys a mirror re-signs with) |
| `iohr lab check [PATH...] [--config FILE] [--strict FILE]` | Check RFCs and studies as the InOrbit site checks its own: file names, front matter (with the review's `reviewed: YYYY-MM-DD` and `reviewer`), status logs, classified span markers, and redaction on public documents as an uncleared reader sees them (text inside `[[classified:LEVEL reason="..."]]...[[/classified]]` or a ```` ```classified level=LEVEL reason="..." ```` block is exempt, its reason is not); `--strict` adds a rules file such as the platform's `docs/lab/strict.json`, whose hits name the rule and never what matched (its `pending-sweep.json` exemptions are not read); offline, exit 1 with every finding ([Lab documents](https://docs.inorbit.hr/docs/lab)) |
| `iohr decisions list \| show \| create \| save \| status \| comment \| link-pr \| review \| diagram` | RFCs, PRDs and ADRs in the Decisions product (platform RFC 0065, RFC 0081; `iohr rfc` until core ADR 0056, removed with no alias) through the SDK's `decisions` operations (`/v1/decisions/...`), scopes `decisions:read` and `decisions:write`: a document by its number (`0065`, a part `0065.1`; `PRD 0001` or `ADR 0003` names its kind, else `--kind rfc|prd|adr|study|all`, default `rfc`) across the account's spaces (`--space` narrows), `create --kind` for a PRD or an ADR, `create` with `--file` saves the text straight away, `save` a new version from a file (refused when someone saved since), `status`, comments that name a pull request first (`owner/repo#N "title": open`, `merged`, `live`), review requests (`--reviewer` or `IOHR_DECISIONS_REVIEWER`; `--reason` as a comment), diagrams from a model file; `--json` on each |
| `iohr runner register [--name N] [--capacity N] [--always-on] [--holder SUBJECT] [--refresh] [--account A] \| run [--name N \| --runner ID] [--interval 20] [--capacity N] [--kinds K,...] [--max-beats N] [--no-auto-upgrade] [--session DIR] [--account A] \| config show [--capacity N] [--kinds K,...] [--no-auto-upgrade] \| status [--account A] \| gate-report --job J --lease L --token N [--repo DIR] [--command C] [--timeout S] [--name N \| --runner ID] [--account A] \| run-key [--proof FILE] [--label L] [--wait MIN] [--account A]` | This machine as a coordinator runner (platform RFC 0120), in the profile's account or `--account A`; every call names it, because the sign-in token's own account is the personal one: `register` records its name, system, `--capacity` (1 to 32) and the one `claude` on your PATH; `run` heartbeats with the capacity, version and kinds in effect, accepts turns the coordinator offers, up to that capacity at once, and runs each on your installed, unmodified `claude -p`, on your subscription only (below), and after a job turn that replied sends its gate report when the pack names the checkout (below); `status` lists your runners and what this machine can run; `gate-report` runs the check for a job whose lease you hold and sends the signed result (below); `run-key` makes and enrols the key `run` uses to carry out approved run asks (below) |
| `iohr session [DIR] [--json]` | Open a session on a project (platform RFC 0213): starts the project's one session host if none runs (this same `iohr` as `runner run --session DIR`, a process of its own that hosts the project's Roles and the runner's turn loop), joins it, and stays open until Ctrl-C or SIGTERM. The host stops when the last session has left (a shell that is killed counts as left), on `iohr stop`, or on SIGTERM; a host nobody joins within 30 s ends by itself. With `--json` it prints `{project, host_pid, roles}`; notes go to stderr. Needs a runner of its own: the host signs in as the profile `session`, whatever `--profile` the shell has, with its own runner key, because sharing the live runner's key would make the two overwrite one heartbeat row and take each other's leases. One time: `iohr login --profile session`, then `iohr --profile session runner register --name HOST-session`; without them the host stops at once and says which one is missing. Every `?`/`??` ask asked in the project's folder is answered by its Roles too (see Session mode). Unix only |
| `iohr stop [DIR] [--wait S] [--json]` | Stop the project's session host and wait up to `--wait` seconds (0 to 600, default 15) for it to end; an answer a Role is still writing is cut short and goes back to the queue for the next host. Prints `stopped` only when the host's lock is free, `stopping` while a Role still holds a task; says on stderr that no host runs when none does, and exits 0 either way; `--json` prints `{project, state, host_pid}` with `state` one of `not running`, `stopped`, `stopping`. Makes no API call; Unix only |
| `iohr browser install [--print]` | Open the console's install page for InOrbit Trails, the browser extension, in the default browser: it adds Trails from that browser's store, or shows the load-unpacked steps while the store has no listing yet; `--print` prints the address for a browser on another machine (or where none can open) |
| `iohr tunnel --host H [--env production\|local] [--port P] [--dry-run]` | A loopback pipe from this machine into an environment's Envoy edge: `ssh -N -L` to `H` (or `IOHR_TUNNEL_HOST`), bound to 127.0.0.1 on both ends, so the real site opens at `http://www.localhost:<port>/` (also `console`, `docs`, `auth`) with no DNS or certificate; `--port` moves the listening side, `--no-engine` skips the engine LB the local environment also forwards, `--dry-run` prints the addresses and the ssh command. It ends at Envoy, so every gate stays. Known gap: sign-in cookies belong to the production domain, so signed-in pages do not work on localhost |
| `iohr upgrade [--check]` | Update iohr the way it was installed (Homebrew, the install script, or the command to run); `--check` only reports; needs no sign-in |
| `iohr ext packages` | Every package you have in one table: published by your account, private ones shared with it, ones in progress on this machine, and what is installed (`iohr packages` still works, hidden from the help) |
| `iohr ask [--headless \| --card \| --interactive] [--agent A] [--model M] [--role NAME [--job ID]] [--mode manual\|plan\|auto] [--allow-edits] [--permissions prompt\|rules\|deny] [--thinking show\|collapse] [--dry-run] [--cwd DIR] [--last-command C] [--exit-code N] [--output-file F [--output-kind attached\|captured\|pasted]] [--read FOLDER]... [--prior-question Q --prior-answer A] QUESTION...` | Ask your own AI agent (Claude Code or Codex, local, your subscription) a question about this folder. By default it streams one print-mode run to the terminal: what it thinks (dim), each tool it uses (one row each), the answer as it arrives, a footer with time, turns and cost; with no terminal, `NO_COLOR` or `TERM=dumb` only the answer is printed, `--json` passes the agent's own events through as JSON lines, and nothing is changed unless `--allow-edits` (files only). `--headless`, `--card` or a context flag selects the other path, the one the `?` prompt uses (Claude Code only): one capped `claude -p` run (model `haiku`, 4 turns, 90 s, no session saved, read-only file tools, no write access, a scrubbed environment, the question on stdin; a Bash tool only through your permission rules, and at a terminal Io asks with a small Io card (numbered bar, arrows or 1-9, `y` once, `a` always in this project, `n` no, starting on No; one plain line under `NO_COLOR` or without a terminal) before it runs a command the rules do not cover, with `--mode manual\|plan\|auto` (or `IOHR_ASK_MODE` or `[ask] mode`) choosing how: manual asks before each command, plan is read only and answers with a plan, auto runs what your rules do not forbid without asking, deny rules and secrets still winning; `--permissions rules` or `deny` or `IOHR_ASK_PERMISSIONS` or `[ask] permissions` still work), the answer printed after one AI label line on stderr, or, with `--card` at a terminal, drawn as a card on stderr (plain on a pipe, under `NO_COLOR` or `--json`); with `--json` it prints `{answer, command, questions, agent, via, label, model, exit_code, duration_ms, permissions, context}` (`context` is what of the shell's context was sent, as data: `{folder, command, exit_code, output, output_bytes, can_attach}`; `output` is `not_sent`, `attached`, `captured` or `pasted`, never the text itself) (`command` is the one command the answer suggested, or null; `questions` are the at most two questions the model asked back, each `{about, ask, choices}`, or `[]`). Inside a project the prompt carries iohr's own git facts (branch, last 5 commits, change counts) and the project's notes; at a terminal the card asks the questions, keeps the answer in `.inorbit/memory/` and `inorbit.toml` and refreshes the answer once, and a dim line says when asks are waiting `--read FOLDER` (alias `--add-dir`, repeatable), `IOHR_ASK_READ` (a path list) and `[ask] read` name more folders the run may read, each checked like a yes (it must exist, hold no secrets and not be broad: the root, your home folder and every folder above it are refused, bar an explicit `--read ~`; the secret deny rules stay on), each written to `permissions.log` by source and path and listed in `--json` `context.folders`; `--interactive` forces the streamed run even with those flags; `--via io` says "not built yet" |

| `iohr ask --menu --command C [--link L] [--undo RULES] [--asked Q --access FOLDER --step KIND:VALUE --failed-command C --failed-exit N --output-to FILE]` | The action menu under a `?` card (Run, Copy, Follow up, Explain, Open, Undo for a permission the run saved, Add folder to keep one more folder for Io to read, Allow for a folder Io asked to read (kept in `trusted.toml`, then the question is put on the prompt again), the answer's next steps as numbered entries, Asks when some wait, and, after a failed command with a "why did that fail" question, Re-run with output and Paste output: the re-run shows the exact command and asks first, runs once for at most 10 s with the end of its output kept, and the paste reads to Ctrl-D; either writes the redacted output to FILE and prints the question again with `captured` or `pasted` after a 0x1e character) on the terminal, letter keys; prints only the one line for the hook, asks no model and runs nothing (Run prints the command and `run` after a 0x1e character; the hook executes it at once, after a yes when it is risky, never when a rule denies it, and a command with a placeholder gets Copy only) |
| `iohr asks count [--cache] [--json] [--account A]` | How many asks wait for you: "3 asks", nothing for none; only asks whose turn is yours (`pending`) count, one you already answered with a note (`question`) is the agent's turn and is left out (`iohr asks list` still shows it), `{"count": N}` with `--json`; `--cache` keeps just the number in `~/.inorbit/term/asks-count` and does nothing while that file is under 30 seconds old (the shell hook's prompt segment uses it). It reads the sign-in through a quiet credential store: no Keychain dialog (macOS interaction is switched off) and no wait past 1.5 s; when the store would ask, `--cache` keeps the last number and prints nothing, without `--cache` it exits 1 saying so |
| `iohr asks list \| show ID \| approve ID [--once] \| deny ID \| question ID \| decide ID \| key enroll [--label L] [--proof FILE] \| key show \| key remove` | The asks agents file for you (RFC 0204, ADR 0215). `approve` needs a device key's proof (RFC 0113). A Mac signs it with its own Secure Enclave key after Touch ID (a P-256 key made on first `key enroll`, kept as a 0600 blob only that chip can open, bound to the current fingerprints: change them and enrol again; the first `enroll` compiles a small Swift helper with the system `swiftc`, so Xcode's command line tools must be installed). `enroll` registers it as a `macos` device key; `--proof FILE` takes the countersignature JSON (`{key_id, signed_at, nonce, signature}`) an enrolled phone key made over the new public key, which activates it at once, otherwise the service holds a new key for 24 hours (the first key is active at once); `key show` reads it back and `key remove` revokes it and deletes the blob. Before Touch ID it prints the ask's title, kind and payload hash, and the prompt names the same title. Only a review, a document, a decision and a run may be signed here; a merge, a roll, a migration and an auto-approve rule (found in the kind, title, summary or what approve means) always say "Approve on your phone", and so does a Mac without a key, and every Windows or Linux machine (they never sign: `key enroll` refuses there). A refused Touch ID signs nothing and changes nothing. `deny` and `question` need no key |
| `iohr permissions list\|allow RULE\|deny RULE\|remove RULE\|check TOOL INPUT [--project\|--user] [--json]` | The rules for what Io may run or open when you ask it something (Claude Code's rule grammar) in `.inorbit/permissions.toml` and the user file in the iohr config folder; `allow` refuses rules that could open a secret file and a blanket `Bash(*)` (exit 2); `check` prints `allow`, `deny` or `ask` and the rule; every ask decision is logged to `.inorbit/permissions.log` (no output, no contents); see `docs/config.md`, section 4.5 |
| `iohr shell hook zsh` | A zsh snippet for `eval "$(iohr shell hook zsh)"` (a Down arrow on an empty line, or `[ask] menu_key`, opens that menu; Shift+Tab on a `?` line cycles manual, plan and auto): a line that starts with `? ` (or `?? ` for a follow-up) is answered by `iohr ask` and never run; `bash` and `fish` say "not built yet" (below); the right prompt shows "3 asks" when asks wait for you, read from `~/.inorbit/term/asks-count` and refreshed in the background (`INORBIT_ASKS_PROMPT=0` turns it off) |
| `iohr` (no subcommand) | On a terminal: enters the session. It joins this folder's session host (starting it if none runs, as `iohr session` does) and gives your shell with the floating panel over it; if the host cannot start it says so on stderr and the panel still runs without one. Under `NO_COLOR`, with `--json`, or off a terminal it prints the usage report and exits 2, as before; `iohr --help` and `iohr help` print the help. `iohr stop` ends the host, which closes the panel and leaves your shell open. Unix only |
| `iohr panel [--demo] [-- COMMAND...]` | Runs your shell (or COMMAND) with a floating panel at the top right of the terminal, drawn over the output and taking no rows (see "The floating panel"); `--demo` fills it with labelled stub asks; with `--demo --variant rail\|accordion\|band` it prints the older static design drawings instead |
| `iohr completion <shell>` | A completion script for bash, zsh, fish, elvish or PowerShell |

Every command takes `--profile` (or `IOHR_PROFILE`), `--json`, `--verbose`, `--env production|local`
(or `IOHR_ENV`) and `--base-url` (or `IOHR_BASE_URL`, which wins over `--env`).
`--verbose` prints method, path, status, time and request id on stderr; never a header,
a query value or a body.

### The help, and exit codes

`iohr --help` groups the commands the way `gh` does (Sign-in, Account, Build, Agents, Operate);
`-h` prints one line per command and `--help` the long text. The help is coloured with the
palette the interactive screens use, and plain under `NO_COLOR`, `--json` or a pipe.
Commands that kept an older name still run (`iohr packages`, `iohr project templates`); they are
hidden from the help.

```
0   ok
1   a finding or a failure: a failed call, a lint finding, a check that did not pass
2   usage: a flag or argument the command does not take
3   not signed in, or the token was refused
4   forbidden by scope, role or plan
75  EX_TEMPFAIL: `iohr runner run` stopped after installing a newer `iohr` so its launchd or
    systemd supervisor starts it (`upgrade_restart = "exit"`); that is not a failure
```

A usage error (exit 2) helps instead of only refusing. It says in plain words what was missing
or wrong, prints the usage line, shows two to four real examples for that command (the same
`Examples:` that end its `--help`, from one table in `src/examples.rs`; a test parses every
one and checks the `iohr api` routes against `spec/openapi.json`), points at `--help`, and
offers one line to get AI help: `iohr ask '...'` with the command name and what was wrong filled
in. That line is an offer. Nothing runs and nothing is sent unless you run it. A value you typed
is never repeated in that line or in the JSON (a value may be a secret): an error `iohr` finds in
the arguments names the argument or flag (`invalid value for --days <DAYS>`), never the value.
A usage error a command raises while it runs may quote what you typed on the terminal, as `iohr
profile use NAME` does; that quote is left out of the AI line and the JSON, which say only that
the arguments were refused. A bare `iohr token` is a usage error too: it lists the subcommands
with the examples, exits 2 and prints nothing on stdout. Everything goes to stderr; stdout stays
data only.
Under `--json` it is one line in the platform's error envelope, for example
`{"code":"usage","error":"`iohr api` is missing <METHOD> and <PATH>","details":[{"type":"usage","command":"iohr api","usage":"...","notes":[...],"examples":[{"command":"iohr api GET /v1/me","what":"..."}],"help":"iohr api --help","ask_ai":"iohr ask '...'"}]}`.
With no terminal, `NO_COLOR` or `TERM=dumb` it is plain text.

## Your login in other programs

`iohr auth token` prints the active profile's access token, refreshing a signed-in
session first when less than a minute of it is left. The SDKs' `cli` credential source
runs it ([docs/config.md](../docs/config.md) section 5.4), so `iohr login` is enough for a
program on your machine to call the API as you:

```sh
iohr auth token --profile work --format json
# {"access_token":"eyJ...","expires_at":"2026-10-04T10:15:00Z","profile":"work","account":"acc_8d2e"}
```

`expires_at` is RFC 3339 in UTC, or `null` for a token without an expiry; `profile` is
`null` when the token came from `IOHR_TOKEN`. Without `--format json` the token is printed
alone on one line. The refresh token never leaves the credential store. Exit codes are the
usual ones: 3 when there is no such profile, no credential, or the session has ended
(`iohr login` again), 1 when the sign-in service cannot be reached.

## Add the SDK to a project

```sh
iohr sdk add                 # the language and package manager from the project's files
iohr sdk add go --version 0.2.1
iohr sdk add --dry-run       # print the command, run nothing
```

The language comes from the nearest project file between the current directory and the
repository root: `Cargo.toml`, `package.json` or `deno.json`, `pyproject.toml`,
`requirements.txt` or a Python lock file, `go.mod`. When one directory holds several,
name the language (`ts` and `js` stand for `typescript`, `py` for `python`). The package
manager is the one the project already uses:

| Project | Command |
|---|---|
| Rust | `cargo add inorbithr` |
| `pnpm-lock.yaml`, `yarn.lock`, `bun.lock` | `pnpm add`, `yarn add`, `bun add @inorbithr/sdk` |
| `deno.json` without `package.json` | `deno add jsr:@inorbithr/sdk` |
| other JavaScript and TypeScript | `npm install @inorbithr/sdk` (or the `packageManager` field's) |
| `uv.lock`, `poetry.lock`, `pdm.lock` | `uv add`, `poetry add`, `pdm add inorbithr` |
| other Python, in an active virtualenv | `<venv>/bin/python -m pip install inorbithr` |
| Go | `go get github.com/inorbithr/sdk/go@latest` |

Lock files are looked for up to the repository root, so a workspace member uses its
workspace's manager. Without a virtualenv and a uv, poetry or pdm lock, `iohr` installs
nothing into a system Python and prints the uv and venv commands instead. `--version`
installs that release (`inorbithr@0.2.1`, `inorbithr==0.2.1`, `@v0.2.1`); without it the
manager picks the newest and records it as it always does. C# and Java are not on NuGet
and Maven Central yet and are refused with a pointer to the source.

`iohr` prints the command, then runs it as one program with its arguments (no shell,
never sudo) in the project directory; the exit code is the package manager's. Only the
package manager contacts a registry. After it succeeds, `iohr` shows a first call in that
language and points at `iohr sdk config`. With `--json`, the manager's output goes to
stderr and stdout holds one JSON object: `lang`, `manager`, `dir`, `command` (the argument
vector), `dry_run`.

## What will my service see?

`iohr sdk config` resolves the SDK configuration the way `load` does in every SDK
([docs/config.md](../docs/config.md) sections 2 to 5) and prints the same JSON as the
SDKs' `describe()`: the profile and where it was chosen, the config file, every setting
with its value and source, the credential chain with each source tried, the pipeline,
and what was read but ignored (unknown keys, scopes a token does not use).

```sh
INORBIT_TIMEOUT=5s iohr sdk config --profile ci
```

It reads `INORBIT_*`, the standard proxy variables and the config file, contacts no
host and prints no secret: a key secret, a token and a proxy password show as
`<redacted>`. `--profile NAME` is the client's `profile` option; `IOHR_PROFILE` is the
command line's switch and is not read, as the SDKs do not read it. `--for TYPE` resolves
for a generated profile type (`Client<AcmeCi>` is `--for acme-ci`) instead of the
public client. A configuration `load` would refuse exits 1 with every problem on
stderr, each with the setting and where its value came from.

## An SDK for your account

```sh
iohr login                                   # a person, or `iohr login --with-token --profile ci`
iohr profile account default acme            # a team's slug or id (a person's profile)
iohr sdk generate --lang rust --for default --for ci --out src/iohr
git add src/iohr src/iohr.lock
```

The surface holds exactly the operations each profile's credential may call, on the
language's runtime (`inorbithr` in Rust and Python, `@inorbithr/sdk` in TypeScript,
`github.com/inorbithr/sdk/go` in Go, `hr.inorbit:inorbit-sdk` in Java, `InOrbit.Sdk` in C#); a
call a profile may not make does not compile (in Python, pyright and mypy refuse it). In Go
each profile is a package, and the surface's import path is read from the enclosing `go.mod`
(or given with `--package`); in Java `--package` names the surface's package, and in C# its
namespace (default `InOrbit.Generated`). `--lang` takes all seven: `rust`, `typescript`,
`python`, `go`, `java`, `csharp` and `swift` (ADR 0013). In CI, with a token
per profile in `IOHR_TOKEN_<PROFILE>` (`IOHR_TOKEN_CI`, `IOHR_TOKEN_DEFAULT`), `iohr sdk
check` fails with a diff when the API's cut has moved, so a plan change or a revoked
scope is a failing check, not a surprise in production. A person profile cannot sign in
on CI: give it a token there, or leave it out of the lock.

## Examples for a reference

```sh
iohr sdk examples --from openapi.public.json --out examples.json
```

For every operation of the document, one program per language that builds the client,
makes the call with its path and required query parameters filled (the document's
`example`, or a `<name>` placeholder), walks every page of a list, reads a stream in a
loop, and reports the API's error code and request id. The output is JSON:
`operations.<operationId>.code.<lang>`, with the generator's version, the document's
API version and its cut hash. Each snippet uses the published runtime's public surface,
and the compile tests build every one against each runtime (`compile_examples.rs`). How
a snippet builds its client is one template per language
(`crates/iohr-codegen/src/examples/client.rs`), so a change to the client configuration
changes every example in one place. The docs site (docs.inorbit.hr) shows these beside
each operation of its API reference.

## Domains

```sh
iohr domains add acme.hr                 # prints the TXT record to add
iohr domains verify acme.hr --wait       # looks it up from several resolvers until seen
iohr domains confirm acme.hr             # marks it verified for the account
```

`--subdomain` proves only a subdomain's subtree (`staging.acme.hr`). The token expires
after 7 days unless the domain is verified, and a verified domain is checked again every
day. The calls need the `domains:read` and `domains:write` scopes.

## Connections

```sh
iohr connectors list --category incident        # what can be connected
iohr connectors show pagerduty                  # its modes, fields, settings and actions
iohr connections add incident-io                # asks for the API key without echo
iohr connections add pagerduty --mode api_key --config region=eu.pagerduty.com \
  --secret-file api_key=./pd.key                # or --secret-stdin api_key < pd.key
iohr connections add slack                      # opens Slack in a browser, waits until done
iohr connections grant incident-io --to product:reliability --actions create_incident --expires 90d
iohr connections history incident-io --status failed
iohr connections delete incident-io             # asks first; --yes in a script
```

A secret field (an API key, a bot token, a password) is never an argument: `iohr` asks for
it without echo on a terminal, or reads it from `--secret-file FIELD=PATH` or
`--secret-stdin FIELD`. It is held in memory that is wiped when dropped, sent once in the
body of the call that makes or reconnects the connection, and never printed, logged or
written anywhere. The platform tests the key before it stores anything: a refused key
exits 1 with the reason and nothing is kept, a passing one prints who the account is at
the provider ("Connected incident-io as … on incident.io"). Settings that are not secret
go in `--config KEY=VALUE`.

A mode that signs in (OAuth) starts a connect session on the platform and opens the
provider's page in your browser, or prints the link over SSH and without a display.
You finish on the provider's page and the console page it returns to; `iohr` asks the
session every 2 seconds until it completes, fails or expires (10 minutes), then prints the
connection. `iohr connections reconnect NAME` gives a connection a new credential the same
way. Connecting and deleting need an owner or admin of the account and the
`connections:write` scope; listing needs `connections:read`; grants are made by a
signed-in person, not by a token. An AI model's connector says so: what you send in
prompts goes to that provider under your own agreement with them.

## Extensions

An extension is a separate program that `iohr` installs from an OCI registry, verifies,
pins and runs. The first is the InOrbit agent:

```sh
iohr ext search agent                    # the catalogue: what you may install
iohr ext show                            # no name: the whole catalogue, coming ones marked
iohr ext show inorbit/agent              # publisher, signer, scopes, privileges, versions
iohr ext install inorbit/agent           # or inorbit/agent@0.1.0, or just: agent
iohr agent status                        # runs the installed program
```

An extension gets tokens over a private channel, for the scopes its manifest declares and
no others. Signed in as a person, each token is narrowed to the scopes asked for: `iohr`
makes an account key with them, exchanges it for a 15-minute token and revokes the key at
once (an admin or owner of the account is needed; anyone else is refused with that
reason). A key or token profile hands out its own token when it holds the scopes, and is
refused when it does not. Minting is bounded at 30 s, then the program gets a `503` that
says why (ADR 0012, point 6).

`iohr project init` looks at the template and the `--describe` text, finds the catalogue
listings that fit (a rule table in `commands/init_extensions.rs`) and, at a terminal, asks
whether to install each one (default no) through the same path as `iohr ext install`. With
`--yes`, `--json` or no terminal it prints one `iohr ext install ...` line and installs
nothing. A listing marked coming is never offered for install; init says it is not released
yet and that there is nothing to install, as a plain line, not as a skipped step. The site
crawler is not a listing of its own: it is released inside `inorbit/agent` as
`iohr agent atlas crawl <URL>`, so init offers `inorbit/agent` for it (and offers nothing for
an extension that is already installed).

`iohr ext search` and `iohr ext show` read the platform's extensions catalogue over the
API host (`extensions:read`, or a signed-in person); they never contact the registry. A
private listing the account may not see is "not found", like one that does not exist.
`iohr ext install PUBLISHER/NAME` asks the catalogue which version to install (the
newest release, or `@VERSION`), its digest and its signer, then fetches that digest from
the registry and runs every check below. The artifact must then be the one listed: the
same version, signed by exactly the signer the catalogue names, which must be the
listing's signing identity, with the same scopes and privileges. Any difference fails
the install and nothing is written, and so does an installed extension of the same name
from another signer (`iohr ext remove` it first). The catalogue never replaces the trust root: an
artifact the root refuses is refused whatever the catalogue says. The short form
`iohr ext install agent`, `upgrade` and `sync` read only the registry, as before. Another publisher's
extension is not in InOrbit's registry, so it needs `ext.registry` set to the mirror it
is published in.

Before anything is used, `iohr` checks every digest, a signature and SLSA provenance
from InOrbit's release workflow (Sigstore, offline against the root built into `iohr`)
and the manifest's rules; there is no flag that skips this. Install prints who signed it
and the API scopes it may ask for. What is installed is pinned in `iohr-ext.lock`; with
`--lock iohr-ext.lock` the same file goes into your repository, and `iohr ext sync`
installs exactly that elsewhere. Nothing updates by itself: `iohr ext upgrade` does,
when you run it.

An extension that controls a system service, such as eBPF traffic capture, declares
the Linux capabilities that service holds in its manifest (`privileges`). `iohr` grants
none of them: it runs as you, and the service gets them from its own package or unit.
Install shows each one in plain words and asks you to type `yes`; in a pipeline, pass
`--yes`, and without a terminal and without `--yes` nothing is installed:

```text
capture 0.1.0 declares privileges: its system service holds these Linux capabilities.
  CAP_BPF        load eBPF programs into the kernel
  CAP_PERFMON    observe performance and kernel state (perf events, tracing)
  CAP_NET_ADMIN  configure the network: interfaces, routes, firewall, traffic control
iohr runs as you and grants none of them; the service gets them from its own package or unit.
Type yes to confirm these privileges and install it:
```

`iohr ext upgrade` names privileges a new release adds and asks again. `iohr ext list`
and `verify` show them (`--json` too), and `iohr-ext.lock` records what was confirmed:
`iohr ext sync` refuses an extension whose privileges grew beyond its lock entry
(SR-32).

A running extension never sees your refresh token or the credential store. It asks
`iohr` for an access token over a private socket (`IOHR_EXT_TOKEN_SOCKET`, mode 0600),
only for scopes its manifest declares; `IOHR_EXT_API` is the API's address. Until the
platform can mint narrower tokens, the token handed out is the profile's own 15-minute
access token, and only when it holds every scope asked for.

### Building one

From an empty folder to a running extension:

```sh
iohr ext init inorbit/hello --lang shell   # or rust, or python
cd hello
iohr ext lint --preview                    # the manifest, checked, in plain words
iohr ext dev                               # run it: live log, token channel, heartbeat
iohr ext dev --test                        # its self-test, same channel and folders
```

`extension.toml` is the source manifest: the artifact's config (name, version,
entrypoint, scopes, privileges) plus how it runs, what it may reach, read, write, start
and send up, its resources, health, secrets by reference, targets and upgrade. Unknown
keys are refused. `iohr ext lint` exits 1 on an unknown key, a value that looks like a
secret, a `permissions.data.leaves` kind outside ADR 0063's non-sensitive list, a
`targets.kinds` the kind cannot be installed on, or a name holding `tbd`; `--preview`
prints what it may do as an installer will show it, `--json` the same structured, with
the artifact's `config`. A program may name `cluster` in `targets.kinds` (it needs
`delivery = "program"` and a `linux/...` platform), serve a panel (`[panel]`: `title`,
`entry`) and read `$REPOS`, the repository folders the user adds in its host (RFC 0143).

`iohr ext verify` checks what is installed; `iohr ext verify ARTIFACT|sha256:DIGEST --bundle
FILE --publisher-key PEM` checks one artifact that is not, with the install's trust check,
offline and without a sign-in: it prints the signer and exits 1 when it is not trusted
(`--json`: `ok`, `digest`, `signer`, `provenance`, `error`).

`iohr ext dev` lints first, then runs the program with the token channel for exactly
the manifest's scopes: every request is logged, a refused one in red. `STATE` and
`CACHE` are `~/.inorbit/ext-dev/NAME/state` and `cache`, and `REPOS` the roots in
`~/.inorbit/ext-dev/NAME/config.json` (`{"repos": ["/abs/path"]}`) joined with `:`, empty
without one; heartbeats are marked
`"dev": true`; installed extensions and their lock are never read or written. The child
gets no `IOHR_TOKEN*`, `INORBIT_TOKEN*` or `INORBIT_KEY_*`, and no folder holding an
`iohr` stays on its `PATH`, so the channel is its only credential. A secret
`{ name, ref, env }` comes from `IOHR_EXT_SECRET_<NAME>` or the Keychain (service
`hr.inorbit.iohr.ext`, account the `ref`). `--test` runs `tests/` (`*.sh`,
`python3 -m unittest`, or `cargo test`) the same way, with a fresh `HOME` and `TMPDIR`,
and fails when anything was written outside `STATE` and `CACHE`. The whole loop:
[developing extensions](../docs/extensions/developing.md).

From a company's mirror: `iohr config set ext.registry registry.acme.hr/inorbit/iohr-ext`,
plus `iohr config set ext.trusted_keys mirror.pub` if the mirror re-signs. A mirror that
needs a login reads `IOHR_EXT_REGISTRY_AUTH=user:password` from the environment. How to
check an extension by hand: [verifying extensions](../docs/security/verifying-extensions.md).

## A project of your own

`iohr project init NAME` (also `iohr init NAME` and `iohr project create NAME`, the same command) sets up a project you build yourself (PRD 0088) with one command,
and tells you what you now have. The local half is real: `~/development/NAME` from a template
(by default `coordinated`: a private macOS service that heartbeats and serves a tiny panel, with the
manifest of `iohr ext init`, plus a `coordination.toml` that names the project's Planner plan and an
`ask` example you can file with `iohr coordinator asks create`; `--template demo` is the same
without the `coordination.toml`), a git repository with one commit, the project's own state under
`~/.inorbit/projects/NAME/` (never in the repository), a Keychain slot reserved by name, and a
smoke test that runs the project once and reads its heartbeat back, with no token channel, so
its token ask is refused: the proof the channel is its only credential.

Run it again in the folder it made and it succeeds: a file that is already there is left
alone (reported as unchanged, or as left alone when you changed it), nothing is written
twice and nothing of yours is committed for you. A folder that is not that project is
still refused. The built-in roles go to your account only with the account step, so
`--no-sync`, or no sign-in, leaves them in `atlas/roles/` with a plain line saying so
(`iohr sync` imports them later). The template's `rust-version` is the toolchain you have
installed, and an agent started with `--describe` is told to keep it as the minimum.

At a terminal, `init` is guided: it asks for the name when it is missing (the folder's name is
the default), offers the templates as a menu (templates that are coming are listed dimmed and
cannot be chosen), confirms the folder, shows each step as a spinner that ends ok, warn or
fail, folds what waits on an API into one line (`--verbose` lists it), and ends with the next
commands. `--template` and `--yes` skip the menu and the confirmation. Without a terminal, or
with `--json`, `NO_COLOR` or `TERM=dumb`, nothing is drawn or asked: the plain `done:` and
`not yet:` lines are printed as before (the editor's own ask, Code asking before it opens the
folder, is said once in the ending, not as a line), on stderr, and stdout stays data only.

#### Reading your public site (PRD 0190.1)

`iohr project init --crawl https://example.com` reads that site on this machine, with the
`inorbit/agent` extension (`iohr agent atlas crawl`), and keeps what it found in the project.
At a terminal, a web address in the `--describe` text is asked about first (default no);
`--crawl URL` is the consent, and with `--json`, `--yes` or no terminal nothing is read unless
you pass it. `--no-crawl` never reads. Before it starts, init says what it will do: the host
you gave only, robots.txt honoured, at most `--crawl-pages` pages (default 200) and
`--crawl-minutes` minutes (default 5, hard caps 20000 and 240), 10 hosts, 50 MB in all and 2 MB
a page, and that nothing is sent to InOrbit. `--dry-run` says the same and reads nothing.

The crawl stays on that host. When a page links to another name in the same domain
(`blog.example.com` from `example.com`; from `www.example.com` that also means the apex and its
siblings), init asks once per name at a terminal: yes, no, or yes to all, default no. A yes
runs one more pass on that name alone, inside what is left of the same pages, minutes, hosts
and megabytes, and its raw record is in `.inorbit/crawl/<host>/more/<name>/`; everything lands
in the one study and the one memory entry. With `--yes`, `--json` or no terminal init follows
none, and the summary lists the names it left out. `--crawl-subdomains` is the yes to all given
in advance: the first pass is allowed the host and `*.host`, and nothing is asked.

```sh
iohr project init my-shop --crawl https://example.com --crawl-pages 50
iohr project init my-shop --crawl https://example.com --crawl-subdomains
iohr project init my-shop --no-crawl
```

Files land in `.inorbit/crawl/<host>/` (`crawl.json`, the raw record; `facts.jsonl`, one
fact per page with the day it was crawled; `policy.toml`, the policy it ran under); a
`.gitignore` there keeps them out of the project's git. The same facts become one local study,
`atlas/studies/NNNN-site-crawl-<host>.md` (a second crawl rewrites it in place), which
`iohr sync --dry-run` lists and `iohr sync` carries; and one project memory entry that points
at the study. The summary says how many pages were read, how many robots.txt skipped and how
many failed, whether a limit stopped it (and the flag that raises it), and ends with "Nothing
was sent to InOrbit. Run iohr sync when you want it there." Without the `agent` extension,
init says so in one line and offers to install it at a terminal. A crawl that fails does not
fail init: it is a failed step with the extension's own error. The crawl day is the evidence
time; the crawler records no per-page fetch time.

#### The site knowledge base (PRD 0192)

After the crawl, init reads the text of each HTML page it found, once, in a second pass that
is separate from the crawl and does not change its rate: one request at a time, one a second,
no redirect followed, 2 MB a page, within the same `--crawl-minutes`, and only pages the
crawler read with a 200 answer. The HTML is kept in `.inorbit/crawl/<host>/pages/` (a copy
under a week old is not asked for again, so a second init over the same site makes no request
for it). The text is cleaned (main text, title, headings, links, language; blocks that repeat
across the site are left out), cut along its headings into chunks of about 180 words with a
word count and a stable id (`ch_...`) and its URL and character offsets, and indexed in one
DuckDB file, `.inorbit/crawl/kb.duckdb`. Search is lexical (BM25 over a term table, heading and
title words weigh more); there is no embedding model on this machine, and the crawl box and `--json` say
`embeddings: stub` rather than pass a lexical score for a vector one.

The site model, `.inorbit/crawl/<host>/model.jsonl`, holds entities (organizations, products,
people, places, events, technologies, emails, phones, prices, profiles; from the page's own
JSON-LD first, then patterns), sections with an extractive summary, and facts in the same shape
as the page facts (`kind`, `site`, `url`, `crawled`) plus the chunk each came from. The crawl
box adds the counts. `iohr atlas search|facts|index` read it, and `?` and `??` add the best five
chunks for the question to what the model is given, as `<sources>` with each page URL and
chunk id, so an answer about the site can name where it came from. All of it stays on this
machine, gitignored, until you run `iohr sync`. A build that fails does not fail the crawl.

```sh
iohr project init my-shop         # the coordinated template: a Planner plan comes with it
iohr project init demo --template demo
iohr project create my-shop --template coordinated --dir ~/work/my-shop --json
iohr project template list        # name, status, origin, the default (*) and what it is
iohr project template list --json
iohr project template show demo   # what it asks, its languages, which files it writes, its steps
cd "$(iohr project template path my-sensor)"   # the folder of one of yours
```

The coordinated template adds one step, "The project's Planner plan" (RFC 0211): init makes the
plan, or finds the one it made before (its `external_id` is `project:NAME`, so a second run makes no
second plan), and writes its `plan_...` id under `[plan]` in `coordination.toml`.

| Flag | What it does |
|---|---|
| `--plan-id PLAN` | Use this plan instead of making one. Signed in, init checks you can see it first; a plan it cannot find fails that step, and the message says the files are already written and how to put the id in by hand. Signed out, the id is written as given and said to be unchecked. |
| `--no-plan` | Skip the Planner step; `[plan] id` stays empty. Cannot be combined with `--plan-id`. |

Signed out (no profile and no `IOHR_TOKEN`), offline or refused, init still works: the Planner
step is skipped, it prints how to set the id later (`iohr login`, then `iohr coordinator plan
create NAME --external-id project:NAME`, and put the id under `[plan]`), and the exit code is 0.
A session that exists but cannot be used (a token for another audience, an expired one) is
skipped the same way, and the message names the real reason instead of "not signed in".

`iohr project template list` lists every template (`iohr project templates` is the same command,
kept for older scripts); `macos-app` and `ui-panel` are `coming` and
refuse until they exist. The template is `--template`, else `template` under `[project]` in
`config.toml` (`[project]` / `template = "demo"`; you edit it, `iohr config` does not
write it), else `coordinated` (RFC 0211: the demo plus a `coordination.toml` and a Planner plan; `--template demo` is the plain one) (a described project uses `blank`, whatever `[project]` says). A configured
template that is coming refuses too.

### Templates are data

A template is a folder with a `template.toml` and the files of the project (ADR 0213). Nothing
about one is compiled in except the built-ins, which are the same folders embedded.

```toml
id = "my-sensor"
description = "what it is"
status = "live"          # or "coming"
extends = "demo"         # optional: its files, questions and steps come first; yours replace by path
agent_prompt = "Make @@NAME@@ read @@KIND@@."   # optional, used by the agent step and --describe

[[questions]]
key = "kind"             # lowercase snake_case; the file text gets @@KIND@@
prompt = "Which kind?"
type = "choice"          # string, bool, choice, multi
choices = ["alpha", "beta"]
default = "alpha"
regex = "^[a-z]+$"       # string answers only
help = "shown above the question"
when = "kind == beta"    # also: `key`, `!key`, `key != value`; asked only when it holds

[[steps]]
name = "git"             # git, register, smoke, or agent (labelled AI)
```

Files are filled by the same `@@KEY@@` renderer as the built-ins (`@@NAME@@`, `@@ID@@`,
`@@MODULE@@`, `@@CREATED_AT@@` plus your question keys), and path segments are filled too
(`notes-@@KIND@@.md`). The shape follows copier's `copier.yml` (questions with a type, choices,
default and `when`; layering) and cargo-generate's `cargo-generate.toml` (placeholders with a
type, prompt, choices and regex).

The template comes from, in order: `--template PATH` (a folder), `~/.inorbit/templates/<id>/`,
then the built-ins. `--template gh:owner/repo[//subdir][@ref]` (or `git:URL`) shallow-clones a
repository to read it (never in a dry run; `@ref` is a branch or tag).

```sh
iohr project template new my-sensor [--extends demo]   # a working template in ~/.inorbit/templates
iohr project template lint ./my-sensor                 # checks keys, `when`, regexes, placeholders, a sample render
iohr project template schema                           # the JSON Schema of template.toml
iohr project template list                             # with an ORIGIN column
iohr project template show my-sensor [--json]          # questions, `when`, files, the agent step
iohr project template path my-sensor                   # only the folder, for `cd $(...)`
```

Answers come from `--set key=value` (repeatable), `--answers file.json|toml`, then, at a
terminal, the questions for what is still missing, then the defaults; `--yes` never asks and
takes the defaults, and a missing answer with no default is an error that names the key.

### Describe it, your agent builds it

When `claude` or `codex` is on your PATH (or `[ask]` `agent` in `config.toml` names one), the
menu starts with "Describe it, your agent builds it", and `--describe "text"` does the same.
The agent works in the background: `init` draws the progress itself, runs to the end and exits.
There is no session to open or resume.

### Is the toolchain behind?

While you answer, `init` reads the installed Rust, Go, Node or Python version and asks that
language's own upstream for the latest stable: Rust `static.rust-lang.org/dist/channel-rust-stable.toml`,
Go `go.dev/dl/?mode=json`, Node `nodejs.org/dist/index.json` (the newest LTS line) and Python
`www.python.org/api/v2/downloads/release/` (the highest stable 3.x.y). The language menu shows
it (`cargo found · rustc 1.90.0, 1.93.0 is out`), the dry run says `would: report the toolchain
check: ...`, and the report carries it with the command that upgrades it (`rustup update
stable`, `brew upgrade go` when Homebrew installed it, `mise upgrade`, a download link, ...).

- At a terminal, `init` asks `Upgrade Rust now with ...?` (default no) and runs the command
  only on yes. With `--yes`, without a terminal and in a dry run it prints the command and
  never runs it.
- The whole lookup is bounded to about 3 seconds and runs beside the other steps. Offline, on
  an error, on a timeout or under `--json` it is skipped and the line says `could not check`;
  a version is never guessed and `init` never fails on it.
- `IOHR_TOOLCHAIN_UPSTREAM=off` turns the lookup off; any other value is a base URL for the
  routes `/rust`, `/go`, `/node` and `/python` (the tests use a local server).

The order is fixed: scaffold, then the agent, then the smoke test, then the report.

1. The scaffold is the `blank` template (an empty project in your language; `--template` names
   another on purpose). Its `agent_prompt` says only "a fresh project in <language>; build what
   is described below; keep it free of secrets; keep `<test>` passing".
2. The agent is your own `claude` or `codex` in print mode, in the new folder, with a prompt
   from your words, the answers and the template. It may edit files and run only the
   language's own build and test commands (`cargo build|test|fmt|clippy`, `go build|test|vet`,
   the template's Python and Node tools), this project's own `atlas new`, `atlas sync
   --dry-run` and `memory remember|recall|list|check` (under the running binary's name),
   read-only `git status|log|diff` and `ls`, and fetch and search the web with WebFetch and
   WebSearch; any other shell command (curl included) is denied and drawn dim with its reason.
   The prompt tells it to run one simple command per call and names the binary, the atlas
   and memory conventions and that you send the result with `<bin> sync`. The `docs`
   language ("None, documents only") makes a project of atlas and memory with no program and
   no smoke test; its prompt has no code in it. Its prompt carries the installed toolchain version
   and today's date, and tells it to use the latest stable, secure release and check it at the
   upstream repository and its advisories. It is bounded by `--agent-turns` (default 150, or
   `[project] agent_turns`), `--agent-timeout` (seconds, default 1800, or `[project]
   agent_timeout`) and runs on `--agent-model` (default `sonnet`, env
   `IOHR_INIT_AGENT_MODEL`, or `[project] agent_model`); the output says which values were used. The
   stream is the same as `iohr ask`: thinking dim, one row per tool, a footer labelled
   "AI-built" (AI Act Art. 50).
3. The smoke test then runs on what the agent built, so the report is true for the result.
   When it passes, what the agent built is committed on its own, a second commit whose message
   says it is AI-built (`git log` shows both). When the smoke test fails or does not run, the
   tree is left as the agent left it, uncommitted, and the report says so.
4. If the agent fails or runs out of turns or time, `init` says so, keeps the scaffold, exits
   non-zero and prints one line to continue (`<bin> ask --allow-edits "continue the work"`).
   It also writes `.inorbit/handoff.md` (what was asked, the files written, the agent's last
   words); the next `ask --allow-edits` in that folder reads it into its prompt and removes it
   when that run succeeds. It is a first, small handoff; a fuller one builds on it.

`--json` passes the agent's own events through, as `iohr ask --json` does, then an
`iohr_summary` line (the agent, the bounds, how it ended) and the `iohr_report` line.
Prompts and answers are never logged or stored. With no agent it says how to configure one.

### The `blank` template

`iohr project init my-tool --template blank --lang rust` writes the smallest project in your
language and nothing about InOrbit services: `README.md` (name and a one-line description from
your words), `.gitignore`, `inorbit.toml`, `.inorbit/space`, and a skeleton with one trivial
test (Rust `Cargo.toml` and `src/main.rs`; Python `pyproject.toml` and `src/<module>/__init__.py`;
Go `go.mod` and `main.go`; TypeScript `package.json` and `src/index.ts`). The smoke test is the
language's own test command. Its kind in `inorbit.toml` is `project`.

### The project record and the space link

Every template writes two committed files, neither holding a secret:

- `inorbit.toml` at the root is the InOrbit project record `iohr` and InOrbit Desktop read. Its
  keys are grouped as the project (`name`, `id`, `kind`: `project`, `macos-service`,
  `macos-app` or `ui-panel`), where it came from (`template`, `template_version`,
  `created_at`) and the InOrbit links (`space_id`, `listing`, `memory_namespace`, each empty
  until the step that fills it exists; `iohr init` fills `space_id`). It was `inorbit-project.toml`; nothing in the CLI reads
  the record, so there is no fallback for the old name.
- `.inorbit/space` is the link to an InOrbit space (platform RFC 0103, "The link"): `id` and
  `name` as `key = "value"` lines. `init` writes it from the shared scaffold step for every
  template. Until the space exists it holds empty values and a comment naming the plugin
  command `/inorbit:link <space>`; `iohr init` and `iohr project sync` fill it. `.inorbit/` is
  committed, except `.inorbit/space.*` (the local environment's link), which its own
  `.gitignore` keeps out.

### The atlas, and `iohr sync`

`iohr project init` also writes `atlas/` in the project root, a plain folder you can read and
edit: `prd/`, `rfc/`, `adr/` and `studies/` with one starter each (numbered 0001, filled from
the project name, template, language and `--describe` text, and labelled stubs), `canvas/` with
a starter canvas (`iohr.canvas` version 3, the format InOrbit Desktop reads), `models/` for 3D
files (glTF 2.0; a README, no fake model) and an `atlas/README.md`. A second run never
overwrites a file you have. `iohr atlas init` adds the folder to a project made without it.

```sh
iohr atlas new rfc "Cache the catalogue"      # atlas/rfc/0002-cache-the-catalogue.md
iohr sync --dry-run                            # what would be created or saved; sends nothing
iohr sync                                      # send it to the space in .inorbit/space
```

A document is Markdown with front matter (`title`, `status`, `public`, `summary` and the rest)
that maps one to one onto the Decisions document. `iohr sync` uses the SDK's Decisions client:
it creates what is new, saves a new version of what changed on the version it read, and on a
conflict (the document changed in Decisions) re-reads, says so and overwrites nothing. It writes
`remote_id`, `remote_version`, `remote_number` and `remote_sha` back into the front matter; those
keys stay local. The canvas and the 3D folder are never sent, and the output says so. No document
text reaches a log. The `[sync]` `background` key in `config.toml` is read (default `false`); no
background loop exists yet, so only `iohr sync` sends anything.

### Roles and memory

`iohr project init` and `iohr atlas init` also write the built-in roles into
`atlas/roles/`: `security`, which every project gets whatever its language, and one
engineer for the project's language (`rust-engineer`, `python-engineer`, `typescript-engineer`
or `go-engineer`; `atlas init` reads the language from `inorbit.toml` or the build file). Each
file has front matter (`name`, `parent`, `domain`, the catalogue it came from) and its lines, a
core of at most 700 tokens and recall lines found by topic, each with the primary source it
comes from (NIST SSDF, OWASP ASVS, the Rust API guidelines, the PEPs and the like). The
catalogue is data in the binary, labelled origin `doctrine`, and `inorbit.toml` records the
names in `[roles]`.

Signed in, init imports them through `POST /v1/coordinator/personalities/import` and prints one
line each (`written v1`, `upgraded v1 → v2`, `unchanged`, `left alone`). A role already in your
account that an earlier project put there is read from its history (`ListPersonalityVersions`):
when every version of it was an import and its lines are an older catalogue's, nobody changed
it, and it is upgraded (`v1 → v2`). A copy you changed (a version that is not an import, or
lines of your own), or one you changed in InOrbit since this project's import, is left alone;
the line says the catalogue's version differs and names the command that takes it,
`iohr atlas roles import --upgrade`, which is your go to replace the changed copy. Provenance is
read from that history; there is no built-in marker on the server, so a role whose history
cannot be read stays `left alone`. The catalogue never replaces your changes on its own. When the account has no root role, `base-engineer`
comes with them. Only the account owner or an admin may import. Signed out, offline or refused,
init still succeeds, says so once and keeps the files; `iohr sync` (or
`iohr atlas roles import`) imports later. `--json` reports each role.

The folder was called `atlas/personalities/` before. `iohr atlas init`, `roles list`, `roles import`
and `iohr sync` move it to `atlas/roles/` once and say so in one line; a file that `atlas/roles/`
already has is never overwritten. `iohr atlas personalities` still works as a hidden alias of
`iohr atlas roles`. The wire names stay as the service has them: the paths
`/v1/coordinator/personalities` and `/v1/coordinator/personalities/import` and the `personalities`
key in their bodies.

Every built-in fact carries `checked: 2026-11-05`, the day it was last read against its primary
source. Every prompt that asks a model to research or write facts (`project init`, `--describe`,
the agent step) ends with the system clock's date: "Today is YYYY-MM-DD. Use the newest edition or
version published as of today; name it with its publication date; if you are unsure a newer one
exists, say so."

### Use a Role in a run: `--role`

`iohr ask --role NAME` and `iohr init --role NAME` (the same as `iohr project init`) run the agent
in a Role (RFC 0211). The Role is looked up in this order: the project's `atlas/roles/NAME.md`,
then your account (`GET /v1/coordinator/personalities/NAME`, signed in), then the built-in
catalogue in the binary, so `--role rust-engineer` works offline. A dry run never asks the
account. A name that is nowhere ends the run before anything starts (exit 2).

What the Role changes:

- **Its text.** Claude Code gets the Role's core (the "Core" section of the pack, or the
  account's compiled core, at most 8000 characters, cut with a note when longer) through
  `--append-system-prompt`; the streamed `?` run puts it first in the prompt in a `<role>` block.
  Codex has no system-prompt flag, so the text leads the question. The Role guides how the agent
  works; it never widens the run's permissions, edits or limits.
- **Its model.** The order is the `--model` flag, then `[ask] model` in the config, then the
  Role's own model, then the default (`haiku` for `ask`, the project agent's default for `init`).
  Codex ignores the Role's model. A Role model that is not a plain model name is ignored, not
  an error.
- **Its brief for a job.** `iohr ask --role NAME --job JOB_ID` (signed in) asks the account for
  the Role's brief for that job (`POST /v1/coordinator/personalities/NAME/brief`, the job id
  in the body, never the URL) and uses it in place of the Role's own text. When the brief is
  not available the run goes on with the Role's text and says so in one line (the status only).

With `--json` the Role is named: `ask --dry-run` has `role` beside `args`, the `iohr_summary`
line and the headless answer carry it, and `init`'s agent step and report carry it, each as
`{name, source: project|account|catalogue, brief: role|job, model, job}`. The Role's text is
never in that object.

### Memory

`iohr memory` is local first (RFC 0205, ADR 0216: files, no database). Each memory is one
Markdown file with front matter (`id`, `kind`, `scope`, `created`, `updated`, `checked`, `why`, `pinned`,
`refs`) and the text as the body. There are three scopes; `--scope` picks one per command and
`[memory] scope` in the config sets the default (else `project`):

| Scope | Where | Notes |
|---|---|---|
| `project` | `<repo root>/.inorbit/memory/` | Committed with the code. The repo root is the nearest folder up with `.inorbit/`, `atlas/` or `.git` (the home folder's own `.inorbit/` is not one). Outside a project the default falls back to `user`, said on stderr; `--scope project` there is an error |
| `user` | `~/.inorbit/memory/notes/` | Yours, on this machine |
| `account` | InOrbit's service | Needs `[memory] sync = true`; the only scope that is not a file |

```sh
iohr memory remember "We use sqlx" --why "typed SQL" --ref src/db.rs   # offline; --preference, --pin, --scope
iohr memory recall "database"          # ranked by word match, recency (preferences never decay) and pin
iohr memory list
iohr memory forget <id>
iohr memory check [--strict]           # --strict exits 1 on any finding
iohr memory drift [--remote] [--strict]
iohr memory sync [--dry-run] [--pull]  # off until [memory] sync = true or you run it
```

`check` reports `dangling-ref` (a path that is gone, or a `cmd:` program not on PATH),
`bad-ref` (absolute or `..`), `missing-why`, `stale` (not confirmed in 180 days, `--stale-days`;
pinned ones are exempt), `unchecked` (no `checked:` date in the front matter, or one older than
`--checked-days`, else `[memory] check_days`, else 180; pinned ones are exempt), `never-confirmed`, `duplicate` and `near-duplicate`, and `gap` (topics
no memory covers: the language, the build and test commands, the owners). `drift` only reads: it
compares each `--ref` with `git log` and `git status` (`ref-changed` when the path changed after
the memory was last updated, `ref-uncommitted` when it has edits not committed), and says it
skipped when the folder is not a git repository. It asks the service only when sync is on or
`--remote` is given. A memory is never printed to a log; `--json` prints it to stdout only, and
text is never in a URL. A text that looks like a secret is refused.

`sync` (also run by `iohr sync` when `[memory] sync = true`) shares the `project` or `user`
memories with your account, tagged `project:<slug>` or `user`. The service's `MemoryService` has
create, list, recall and delete, and no update or version, so the check for a change on the
other side is made here: `remote_sha` (a hash of kind and text) and `remote_version` are kept in
the file's front matter, local only. Per memory the plan is `push` (new), `link` (the service
already has the same text), `replace` (changed here: the new copy is created, the ids are written
back, then the old one is deleted; this is create-then-delete, not atomic, and a failed delete
is reported), `remote newer` and `remote gone` (reported, nothing overwritten) or `conflict`
(changed on both sides: reported, exit 1, nothing overwritten). Entries only on the service are
written as files only with `--pull`. `--dry-run` sends nothing. Signed out, `sync` is a sign-in
error and the files stay as they are.

`iohr project init` and `iohr atlas init` write `.inorbit/memory/README.md` and the built-in
roles' recall lines as seed memories, locally, with no network; a seed you forget stays
forgotten. The agent-instructions file (`AGENTS.md` or `CLAUDE.md`) gets a short section that
tells agents to run `iohr memory recall` and `iohr memory remember`.

### Try it without changing anything

```sh
iohr project init my-shop --template demo --dry-run --yes
```

`--dry-run` (alias `--plan`) runs the whole flow, questions, defaults and `when` skips
included, and does nothing: no files or folders, no git, registration or smoke test, no
sign-in, no network call, and the agent is not started; its exact command and full prompt are
printed instead, still labelled AI. Every step reads `would: ...`, and it ends with
`dry run: nothing was changed`. With `--json` the plan is data: `files` (path and bytes),
`steps`, and `agent` (command and prompt). Every action goes through one gate that checks it.

### The project in InOrbit: `iohr init` and `iohr project sync`

`iohr init NAME` is `iohr project init NAME`. Signed in, it makes the project's space in
InOrbit before it writes anything, so the first commit holds the link: a Decisions space of
kind `project` (RFC 0127.1), named like the folder. The team and who can read it:

```sh
iohr init my-shop --team acme --visibility managers   # under the team acme; owners and admins read it
iohr init my-shop --visibility team                   # everyone in the account (the default)
iohr --env local init my-shop                         # the local environment, a link of its own
iohr project sync                                     # make or find the space later, from the folder
iohr init my-shop --no-sync                           # make nothing in InOrbit
```

At a terminal, a missing `--team` or `--visibility` is asked as a menu (a personal account
is "personal"; a team you are not in is refused before anything is made). The sync is
idempotent: the space is found by account and name, so a second run, or a teammate's clone,
never makes a second one; a clone whose sign-in cannot open the linked space is told to ask a
team owner or admin. The link is `.inorbit/space` for production (committed, and `space_id` in
`inorbit.toml` is filled) and `.inorbit/space.local` for `--env local`, kept out of git by
`.inorbit/.gitignore`. The project then shows under Projects in the app and the console, and
the team opens it from the link.

Init never fails for the sync. Not signed in, offline, or a server that is down or refuses the
call (401, 403, 408, 425, 429, 5xx) is a `queued` step with the answers kept in
`~/.inorbit/projects/NAME/space-sync.<env>.toml`; `iohr project sync` completes it and clears
it. Any other refusal is a failed step with the reason. `--dry-run` says `would:` and calls
nothing.

Still `not yet` (the platform has no API for them): a space's members, per-project scopes and
tokens, and the project summary (`POST /v1/decisions/projects`), so the chosen visibility is
the space's access floor (the whole account or its managers), not a member list.

The other steps that need InOrbit (its private listing, its own
scopes and tokens, a memory namespace, monitors and notifications) are reported as `not yet`,
each with the API it waits on; nothing is skipped in silence, and a step that needs your or an
admin's say becomes an ask. Every step is re-runnable, and nothing is overwritten. Then
`iohr ext lint --preview`, `iohr ext dev` and `iohr ext dev --test` work in the folder as for
any extension, and InOrbit Desktop's Code opens it after asking you.

## The `?` prompt

`iohr ask` and `iohr shell hook zsh` put a question prompt in your terminal. Add this to `~/.zshrc`:

```sh
eval "$(iohr shell hook zsh)"
```

Then type `? why did that fail` at the prompt. The line is never run by the shell and is not
kept in its history; the answer prints under it, after the label `Io -- Io running
on Claude Code (local), an AI model, under your own subscription; ...` and a line that says
what was read. `?? and the second one` asks a follow-up with the last question and answer as
context. A line such as `?.txt` or `? ` alone runs as it always did. An accept-line widget you
already have is kept and called for every other line. `INORBIT_ASK=0` turns it off for the
session. Shift+Tab on a `?` line switches the mode of the answers between manual (asks before each
command), plan (read only, a plan and nothing run) and auto (runs what your rules do not forbid);
it says which under the prompt and every question carries it (`--mode`; `[ask] mode` and
`IOHR_ASK_MODE` set where it starts). Inside the InOrbit app (`INORBIT_TERM`) the hook asks the app to draw the answer
(an OSC `7727;ask` escape) instead of printing it.

What goes to your provider: the question, the folder's path, the names at the top of that folder
(at most 60 names and 2 KiB; dotfiles left out except `.inorbit`, `.github`, `.gitignore` and
`.claude`; names the folder's own `.gitignore` hides left out; never file contents), the last
command and its exit status, and, when output is attached (`--output-file`, or the menu's re-run or paste), the last 8 KiB (200 lines) of it with secrets redacted (passwords, tokens, bearer values, URLs, hosts, IP addresses);
the whole message is capped at 16 KiB. The card's `Read:` line says exactly this, including "output not sent" when none went. A plain zsh hook cannot read a command's output, so
it sends none; after a failed command and a question such as "why did that fail", the menu (Down on an empty line) offers Re-run with output (asked first, never automatic, 10 s and 16 KiB at most) and Paste output, and what you attach goes with the next ask only. Off a terminal the card says the output was not sent and how to attach it (`--output-file FILE`). Io runs on
Claude Code only: it signs in with your own `claude` login, never an API key, and `iohr`
stores neither question nor answer. Only zsh is built; `iohr shell hook bash` and `fish`
exit 1 with "not built yet", and so do `--via io` and `--headless` with `--agent codex`. An answer is an AI
answer: check it before you rely on it.

## The floating panel

`iohr panel` runs your shell inside a small terminal proxy and paints a panel over the top right
corner after every chunk of output, with save and restore of the cursor. It takes no rows: the shell
keeps its full height, what you half-typed stays where it is, and the panel moves when the window
is resized. Nothing is drawn under `NO_COLOR`, off a terminal, or on the alternate screen (vim,
less); it comes back when the program ends. Colours are truecolor when `COLORTERM=truecolor`, else
the nearest of the 256.

Bare `iohr` on a terminal runs this same panel inside the project's session (see `iohr session`);
`iohr panel` runs it on its own, without a host.

The panel has these views. At rest it is a pill, `auto ● on   N waiting   ⌥i menu` (44 columns or
as wide as its text needs). The pill names the key that works in your terminal: `⌥i` where Option
is Meta, and `^G` (Ctrl-G) in Terminal.app, iTerm2, VS Code, Ghostty, WezTerm and Hyper, which type
a dead key `ˆ` for Option+i by default (found through `TERM_PROGRAM`). Ctrl-G is a letter, so it
is the same on every keyboard layout; it has no meaning for the terminal driver, and its only
shell binding is "abort" in readline and zsh. Both keys always work. The key opens the menu
(42 columns): `a` auto mode, `p` approvals, `w` workloads, `r` run, `s` status,
`?` help, `x` stop the panel; arrows and Enter move and open, Esc closes. Keys are taken only while
the panel is open; every other byte goes to your shell untouched.

The mouse works on the menu and the views while they are open: a click on a row selects it and
opens it, as Enter does; a click outside closes the view; the wheel moves the selection. The
panel asks the terminal for mouse reports (SGR, `?1000h` with `?1006h`, never drag tracking) only
while a view is open, and turns them off when it closes, when the program on the screen enters
the alternate screen, and when the panel ends (or stops, or the process panics). Reports are
never sent to your shell. At rest it asks for none, so the shell keeps normal drag-to-select. The
`[panel] mouse` setting (or `IOHR_PANEL_MOUSE`) changes that: `menu` (the default), `always` (a
click on the pill opens the menu and every other click goes through to the program) or `off` (no
mouse at all).

Bare `iohr` inside the panel prints one line, that the panel is on and which key
opens the menu, and exits 0; after `iohr stop` it brings the panel back first. `iohr stop` inside the panel stops the session host and closes
the panel with it; with no host running it says the panel is closed. If something cannot be read, the
panel says "unavailable", never a number. Stub data appears only with `--demo` and is labelled "stub".

Auto mode (`a`, 56 columns) is the real auto mode: the account's auto-approve policy (ADR 0142),
the one the Mac app and the phone show. The view reads it (`GET /v1/approvals/auto-approve`): on
until a time, how many asks it decided, and the kinds it never decides. Enter turns it off
(`DELETE`, the same call as the Mac app). Turning it on files an ask for 8 hours that goes to your
phone (`POST`); only the phone sets the policy, so the panel says "asked your phone" and shows the
new state at the next read. A second Enter before the phone answers asks once more.

Approvals (50 columns, `p`) are full approvals through the CLI's own approvals client: `↑↓` pick an
ask, `o` or Enter opens its details (a reader, `↑↓` scroll, Esc back), `q` types a question that is
sent to the asker, `d` denies after a y/n confirmation, and `a` approves. Approving needs the
device-key proof (RFC 0113), so from the CLI it is phone-only: `a` says "approve on your phone".
Merge, roll, migration and auto-approve always stay on the phone. Touch ID signing (PR 450,
`feat/mac-touchid-sign`) plugs in at `panel::approve::attempt`.

Workloads (60 columns, `w`) is the Mac app's workload tree, flattened: a Coordinator root for this
machine, its children with `├` and `└`, then one "Coordinator service" row for the queue. A dot
shows the state: green working, amber needs you, grey idle, red stuck. The header counts
`N running · M need you`; the count is of sessions and jobs, not of the root and service rows, and
the menu shows the same number. The data is real and read every 20 seconds from the same places the
Mac app reads: the sessions registry `~/.inorbit/agents/sessions/*.json` (a session is listed only
while its process is alive; working means seen in the last 10 minutes, else idle; the model family
and the latest assistant line come from the end of its transcript, memory and CPU from `ps`) and
the coordinator picture (`GET /v1/coordinator/picture`: jobs and the queue). A session "needs you"
when an open ask names it (its name, its id, or an id prefix of 6 or more characters), the Mac app's
rule. If the coordinator cannot be reached the service row says "unavailable" and the sessions
still show; the public picture tier is marked redacted. `m` messages the highlighted session and
`t` opens its terminal from the list; `↑↓` move, Enter opens a row in place (72 columns: Why,
Latest, Since, Uses, Waits, PR, then the actions), Esc goes back. In an opened workload the
actions are `m` message (typed in the panel; one JSON line `{at, from, to, text}` appended to the
session's `~/.inorbit/agents/inbox/<id>.jsonl` and to `~/.inorbit/agents/log.jsonl`, what
`iohr-agents send` writes), `t` terminal (brings the Terminal.app or iTerm2 window with that tty
forward through `osascript`; macOS only, and it says so elsewhere), `l` transcript (the end of its
transcript in a scrolling reader), `p` PR (opens the pull request in the browser: a job's own, else the one `gh pr view`
finds for the session's branch; says so when there is none), `a` its approval, and `x` stop (a y/n confirmation, then SIGINT, a second after 600 ms, never
SIGKILL, as the Mac app stops a terminal). Not available yet and shown as "not available": a
session's checks, worker nodes under a session, and the stuck and failed states of a session (only
jobs can be stuck). Under `--demo` the list is the design's stub tree, labelled "stub" in the
title, and every action says "stub: nothing was ..." and does nothing.

Not built yet, and shown as "coming next": the Run and Status views. Only zsh is tried; bash and
fish are untested (the pty test uses `/bin/sh`). `⌥i` needs the terminal to send Option as Esc or
Meta (iTerm2: Profiles, Keys, "Esc+"; Terminal.app: "Use Option as Meta key"); without that setting
use `^G`. If `[ask] menu_key = "ctrl-g"` is set, the panel leaves Ctrl-G to the `?` menu and shows
`⌥i`. The panel saves and
restores the cursor with DECSC and DECRC, which can overwrite a program's own saved cursor. The
shell runs with `IOHR_PANEL` set to the pid of the panel around it, which counts only while that
process lives: a nested `iohr panel` runs plainly and a bare `iohr` says it is already in the
session, and when the panel was stopped (`iohr stop`, "Stop panel", or its host ending) the bare
`iohr` brings it back first, so running `iohr` again gives a live panel. A shell that outlived
its panel starts a new one. While the panel is stopped `^G` is not sent to the shell (it would
echo as `^G` under a command). Arrows count in the menu with any modifier (Shift, Option, Ctrl,
or the kitty keyboard form); a release event is ignored. If the session host stops at once, the
message carries the last line of its `host.log`. `IOHR_PANEL_LEADER` is internal (the proxy re-executes itself to become the shell's session
leader).

## Runner

```sh
claude                                  # once: /login with your subscription
iohr runner register                    # this machine, by its host name
iohr runner run                         # until Ctrl-C
iohr runner status
```

To run as Io instead of a person (platform RFC 0065.2, so jobs Io files run on runners Io
owns), add `--sign-in io` (or `IOHR_SIGN_IN=io`) to `iohr runner`. The runner then signs
in as the OAuth2 client `iohr-io` with the client credentials grant, asks for
`coordinator:runner coordinator:read coordinator:write approvals:read approvals:write`,
keeps the 15-minute token in memory and mints a new one shortly before it ends. The secret
is never an argument or an environment value: it comes from the file `IOHR_CLIENT_SECRET_FILE`
names (it must be yours and mode 0600, or the runner stops) or, on a Mac, from the
keychain item the Mac bridge uses (`iohr-mcp-bridge-io`). A profile (`--profile`, else the
default) still holds the runner's keys and settings, and `--account` names the account.
Nothing else changes: without the flag the runner signs in as your profile does.
`--sign-in io` also works on `iohr coordinator ...` (every other command refuses it), so an
agent's jobs and asks are Io's, not the person's who runs it. To say which role made
the call, add `--via-role NAME` (`IOHR_VIA_ROLE`; the old `--via-personality` and `IOHR_VIA_PERSONALITY` still work), and optionally
`--via-session ID` (`IOHR_VIA_SESSION`) and `--via-job ID` (`IOHR_VIA_JOB`): they set the
`x-iohr-via` header (`personality=improver; session=...; job=...`), which is attribution only and
grants nothing. Each value is 1 to 64 of `A-Z a-z 0-9 . _ : -`, else the call is refused. (`--via` is
taken: `iohr ask --via` says where an answer comes from.) These flags need `--sign-in io`.
Example: `iohr --sign-in io --via-role improver coordinator jobs create --kind chore --brief -`.
A runner of Io's that holds a person's provider sign-in (the person signed in to `claude`
on that machine) names that person at registration:
`iohr --sign-in io runner register --holder <subject>`. The person must be Io's seat
holder and already hold the same sign-in on a runner of their own; the coordinator then
gives that runner only their verified work.

The runner is this machine's half of a coordinator turn. It signs in to InOrbit as your
`iohr` profile does, which needs the scopes `coordinator:runner coordinator:read`
(`iohr login --scope coordinator:runner,coordinator:read`), and calls the coordinator's
runner calls: register (it makes an Ed25519 key, kept in the credential store your
profile uses, the macOS Keychain on a Mac, and signs the service's challenge with it),
heartbeat (every `heartbeat_interval_seconds` the service sends, 20 s; `--interval`
overrides), accept a lease, get its turn pack, keep the lease alive while the turn
runs (the turn stops when the service says so), and report the signed result. A report
the service does not take is kept in memory and retried; it is lost if the runner exits
first. A turn is one
`claude -p` run of the `claude` first on your PATH, as you installed it:
`--output-format json`, the turn's `--model` and `--max-turns`, its `--allowedTools`, the
pack as the system prompt, and `--resume <session>` only when this runner ran the
previous turn of the thread and the session is on its disk (otherwise
`--session-id <new>`). The prompt goes in on stdin. To keep the turn's context small it
also passes `--setting-sources project,local` and `--strict-mcp-config`, so your own
user-level hooks, plugins, skills and MCP servers are not loaded into a turn (your
sign-in is). On `claude` 2.1.307 one short turn went from 33,220 to 15,054 context tokens.

It runs on your subscription and nothing else:

- Before every turn it asks the binary (`claude auth status`) and refuses unless it says
  `claude.ai` and `firstParty`. It never reads Claude Code's own credential files and
  never signs in for you.
- The child gets an allowlist of your environment, nothing else: `PATH`, `HOME`, `USER`,
  `LOGNAME`, `SHELL`, `TERM`, `TMPDIR`, `TZ`, `LANG`, `LC_*`, `CLAUDE_CONFIG_DIR` and the
  XDG directories (and the Windows equivalents), so `claude` finds its own sign-in.
  Anything whose name holds `TOKEN`, `KEY`, `SECRET` or `PASSWORD` is dropped even if
  listed, and so are `ANTHROPIC_*`, `AWS_*`, `GOOGLE_*`, `OPENAI_*`, `AZURE_*`, `GH_TOKEN`,
  `GITHUB_TOKEN` and `IOHR_TOKEN*`. There is no option to pass one through.
- When the runner stops (Ctrl-C or revocation), the running turn is aborted and
  `claude` and everything it started are killed.
- It never passes `--bare`, `--settings` or an endpoint, and never edits the binary or
  its requests. Both the arguments and the environment are table-tested, and a `--bare`
  or an `ANTHROPIC_BASE_URL` in their output fails the build.
- A turn is refused when its user is not the person the runner is registered to.
- `gh` and git are signed out in the child: `GH_CONFIG_DIR` points at a directory that
  does not exist, git's credential helpers are cleared (`GIT_CONFIG_COUNT`) and its
  prompts are off (`GIT_TERMINAL_PROMPT=0`). The check a job runs gets the same.

### GitHub access

A job whose repository is on GitHub is checked out, pushed to `job/<job_id>` and opened
as a pull request by the runner itself, never by `claude`. For each lease it asks the
platform for a GitHub App installation token (platform RFC 0120): that one repository,
`contents` and `pull_requests` write, about an hour; it asks again when fewer than
10 minutes are left before a push. The token is kept in memory only. git gets it per
command through a credential helper given with `-c` (every helper from your own git
configuration is dropped for that command, and hooks are off), only in the runner's own
mirror under its working directory after the mirror's configuration is checked; `gh`
gets it as `GH_TOKEN` for that one call. It is never written to a file or a git
configuration and never logged.

Your own `gh` sign-in is used instead only when the platform says its App is not
installed for the account, or the platform is older than the call; the log says which.
Any other refusal (the App lacks a permission, the lease was not confirmed) leaves the
lease unaccepted, so another runner can take it.

### Session mode

`iohr session` is the way into a project's Roles (platform RFC 0213). Opening it starts
one host process for the project: `iohr runner run --session DIR`, which is the same runner
(heartbeats, the 90 s server-side lease, turns on your `claude -p`, no self-upgrade while it
hosts) plus the project's Roles at work on one local queue. The host stops when the last
session has left, when a shell that was killed has left, on `iohr stop`, and on SIGTERM
(exit 0). It keeps no more than one host per project: a lock the kernel frees when the
process dies, so a crash leaves nothing to clean up.

The queue is a DuckDB file, `queue.duckdb`, in `session/<project hash>/` below the data
directory (`IOHR_DATA_DIR`, else the platform's own), beside the host's socket and log
(`host.log`, the host's own output). The host is the only process that opens it; shells talk
to the host over its socket. The folder must be yours and mode 0700 (the host refuses one that
is not, or that is a link) and the socket is mode 0600; a socket path over 103 bytes is refused
with a message naming the fix, and a host holds at most 64 connections (more are answered
`error busy`). The queue holds `roles`, `tasks` and an append-only `task_events`. A task is
`queued`, `claimed`, `running`, then `done` or `failed`; a Role takes the next task in one
statement, with a lease of 90 s that its heartbeat renews. A statement that meets another
Role's edit of the same row is tried again, not failed.

Leases are timed on a monotonic clock of the host process, never the wall clock, so a laptop
that sleeps or a clock that is set back cannot make a live Role's task look abandoned. While a
host runs, a task is taken from its Role only when that Role's thread has ended and its lease
has run out; a check every few seconds does that and also fails a task that was tried 3 times.
A host that starts holds the lock alone, so every claim left in the file is an orphan: each
goes back to `queued` with the attempt counted, and after 3 attempts becomes `failed` with the
reason. A task is meant to be worked once at a time; a host killed in the middle of one runs
it again on the next start (at least once), so a handler must be safe to repeat.

`iohr stop` makes the host drain: a Role takes no more tasks, an answer it is still writing is
cut short (its child process group is killed) and the task goes back to `queued` with the try not
counted, and until the last Role has stopped the host still answers `status` with `stopping` and
holds its lock. `--wait` polls that lock, so "stopped" is said only when the host has really
ended, and "stopping" when the wait ran out first.

Asks (slice S2 of RFC 0213). When a host runs for the folder an ask is made in (or one above it),
each `?`/`??` ask is put in the queue once per Role (`external_id` `ask:<id>:<role>`, so a repeat
adds nothing), for every Role the host runs, or only the ones `--role a,b` names. Each Role
answers as a child of the host, `iohr --json ask --headless --role <Role> --cwd <folder>` (the
way `--role` answers today; read-only, `plan` when the ask was `plan`), at the same time as
Io's own answer. When Io's card is drawn, the Roles' answers follow, one card each as it lands
(the stored answer is the task's `result_ref`: JSON `{text, model, duration_ms}`, the text cut
at 48K characters). A Role that fails shows its reason in its card. Ctrl-C leaves the wait; the
Roles keep working and `iohr stop` ends them. A task still queued or cut short when the host
stops is answered by the next host. With no host, with `--json`, `--menu`, `--dry-run`,
`--allow-edits` or `--mode auto`, with a `--role` the host does not run, or with an older host,
the ask goes the way it always did. The socket's `roles` and `results <ask>` requests carry
this. Nothing yet re-attaches to an ask's answers after the wait ended, and finished tasks are
not pruned. No launchd or systemd unit starts a session host at login.

Each Role keeps two things in the project between its turns (RFC 0213 S2). When a Role's turn
ends the host writes its handoff note, `.inorbit/handoff/<role>.md`: what it was asked and what
it answered (cut to 800 and 2000 characters, replaced by each turn, written through a temporary
file and a rename). Its memory area is `.inorbit/memory/<role>/`, the same memory files as
`iohr memory` (the project's own store skips the sub-folder, so a Role's lessons are not the
project's). Every `iohr ask --role <Role>` turn, routed or not, is told the note and the newest
eight memories of the area (about 2000 characters, pinned first) after the Role's text. A Role
name that is not lower-case letters, digits, `-` and `_` (at most 64) keeps and reads nothing.
Both are plain files in the project folder; nothing is sent anywhere. Nothing writes the memory
area yet: a person adds a memory file there by hand, in the format of `.inorbit/memory/`.

### Staying current

`run` looks for a newer `iohr` release about hourly, and when there is one it takes no new
work, waits for the running turns and approved runs to finish, installs it with the
verified install of `iohr upgrade`, and restarts itself (`exec` of the same path, or exit
75 for a launchd or systemd supervisor with `upgrade_restart = "exit"`). A failure keeps
the old version running and backs off from 5 minutes. `--no-auto-upgrade` or
`auto_upgrade = false` turns it off; a copy installed by apt is reported and skipped.
On macOS, `upgrade_sign_identity = "auto"` (or `IOHR_SIGN_IDENTITY`) signs each new binary
with a code-signing identity from your keychain, so a keychain "Always Allow" grant holds
across upgrades; it is off by default. Keys, back-off and limits are in `docs/config.md`
section 4.3.

### Capacity

How many turns the runner takes at once, and which classes, come from the `[runner]`
table of `config.toml` (or `[profiles.<name>.runner]`), and `--capacity` and `--kinds`
override it:

```toml
[runner]
capacity = 4                      # 1 to 32
kinds = ["coordinate", "build"]
```

`run` sends them on every heartbeat, so a change reaches the coordinator within one beat;
`kill -HUP <pid>` makes a running `run` read the file again. Lowering the capacity never
stops a running turn. Below 100 GB free on `/mnt/build` the runner takes no new build
turn until there is room. `iohr runner config show` prints what is in effect and where
each value came from (`docs/config.md`, section 4.3).

### Gate report

`iohr runner gate-report --job J --lease L --token N` is the runner's half of the Checked
gate (platform RFC 0120). In the job's checkout (`--repo`, default the current directory)
it fetches `origin/main`, refuses a checkout with uncommitted changes or one that does not
contain `origin/main`, runs `--command` (default `mise run ci:changed`) through the shell
with the scrubbed environment above and a `--timeout` (default 1800 s, at most 21600 s), and
sends the coordinator the command, its exit code, the head, main's commit and the SHA-256
of the output, signed with the key this runner registered. The output itself stays on this
machine; when the check fails, its last 4 KB (whole lines, token-shaped words and URL
credentials redacted) is also printed here (`output_tail` with `--json`) and written to the
runner's log, so a job held at Checked says why. The tail is never signed or sent. A non-zero exit is sent as it is: the job holds at Checked, which is the honest
answer. A coordinator that does not have the call yet (an older release) is reported as
such and is not an error. `iohr runner run` does the same by itself: when a job turn ends
with a reply, it runs this check (the same command, timeout and report) in the checkout the
turn's pack names (`checkout_path`, `checkouts/<job_id>`, relative to the runner's working
directory for turns; a path that is absolute or climbs out of it is refused), keeps the lease beating while it runs, and sends the
report before the turn's own report. It sends nothing, and logs why, when the turn did not
reply, the pack names no checkout (a job with no repository, or an older coordinator), the
check cannot run (dirty or unmerged checkout, time cap) or the lease is gone. Call the
command yourself only for a job the loop did not check.

What the sign-in service sees of the provider is the SHA-256 of the account's salt, a
newline and the identity line, and the method's name. The identity line is built from
`claude auth status` alone, from the fields that name the seat:
`authMethod=claude.ai apiProvider=firstParty orgId=<orgId> email=<email>`, with `orgId`
and `email` trimmed and lowercased. Fields that change on a token refresh or with the host
(`loginExpiresAt`, `configDirectory`, `projectsDirectory`) and those that change without
changing the seat (`orgName`, `subscriptionType`) stay out, so the hash is stable across
token refresh. A sign-in whose status names no organisation or no email is refused at
registration. `iohr runner register --refresh` sends the registration again with the key
this machine holds, after signing in to another account or to update the runner; the
runner keeps its id.

### Approved run asks

`iohr runner run` also carries out run asks the owner approved and pinned to this runner
(platform RFC 0172 and RFC 0153). It needs two things set up once, and stays off with a
note saying which is missing:

- A run key. `iohr runner run-key` makes a P-256 key, kept in the credential store your
  profile uses (like the coordinator key `register` makes, and never printed). Not enrolled
  yet, it sends an enrolment ask to your phone (showing the key's label and fingerprint),
  waits up to `--wait` minutes (15) for your approval and enrols the key; run it again to keep
  waiting on the same ask. To enrol from a file instead, have your device sign the enrolment
  message (`iohr-approval-runner-v1`, the public key, the label, the time, a nonce and the
  device key id), save that proof as JSON `{key_id, signed_at, nonce, signature}`, and run
  `iohr runner run-key --proof FILE`: the runner adds its own signature of possession and
  enrols the key. `run-key` needs no
  coordinator scope.
- The device keys it trusts, in `runner/approvals/<account>/device-keys` in the config directory:
  one `key_id public_key` per line (the key base64url of the 65-byte P-256 point), `#`
  for a comment. The service does not list a person's public keys, so they are pinned
  here. The file, like the record below, must belong to you and not be writable by group
  or world (`chmod go-w`), or the runner stays off.
- The cluster, `kube_context` and/or `kubeconfig` under `[runner]` (or a profile's
  runner table). Every `kubectl` call gets `--context` and `--kubeconfig` from them, so an
  approved action reaches only that cluster; with neither set, every action is refused.

Every 20 seconds (up to 5 minutes after errors) the runner lists the account's approved
`run` asks and takes those pinned to its own runner id. Before anything runs it checks,
in the order the platform checks them, that the ask is approved and pinned, that the
payload hashes to what was approved, that a person decided it, that the action fits the
`runner/host-action/v0` schema, that it names this runner and its key, that it has not
expired, and that a pinned device key signed the decision. An ask whose decision this
machine has already carried out (the record is `runner/approvals/<account>/seen`, owner-only, one line per
decision, a malformed line is an error) is refused as a replay. A refused ask is reported `failed` with the reason and runs nothing.
Otherwise the runner records the decision on disk (under a file lock), reports `started`,
runs the action and reports `done` or `failed` with at most 500 characters of what
happened, with addresses, URLs and tokens redacted; each report is signed with the run key.

The catalogue has one action, `k8s.delete-failed-pods`: `kubectl delete pods
--field-selector=status.phase=Failed` in the namespace the ask names, or in every
namespace. It runs `kubectl` from an absolute `PATH` entry with an argument vector (never
a shell), the scrubbed environment above (no `KUBECONFIG`: the configured cluster is used), and a 2-minute limit. A
namespace must be a DNS label, and `kube-system`, `kube-public` and `kube-node-lease` are
refused. Any other action is reported `failed` as unknown.

## Where things are kept

- Profiles: `config.toml` in the platform's config directory (`~/.config/iohr` on
  Linux, `~/Library/Application Support/hr.InOrbit.iohr` on macOS,
  `%APPDATA%\InOrbit\iohr\config` on Windows). No secret is ever written there. The
  SDKs read the same file ([docs/config.md](../docs/config.md) section 4): their `[sdk]`
  table, their keys inside `[profiles.<name>]`, and profiles that hold only SDK keys.
  `iohr` changes only its own keys (`default`, a profile's `kind`, `account`, `storage`,
  `issuer`, `client_id`, and `[ext]`) and keeps every other key and comment; `iohr
  logout` removes a profile's own keys and leaves the SDK's.
- Secrets: the operating system's credential store (macOS Keychain, Windows Credential
  Manager, the Secret Service on Linux), one entry per profile and account. On a machine
  without one, `--insecure-storage` keeps the token in a file with mode 0600 instead.
- Extensions: the platform's data directory (`~/.local/share/iohr/extensions` on
  Linux), owner-only, with the machine's `iohr-ext.lock` and, per version, the program,
  the Sigstore bundles checked at install and a record of the checks.
- Extensions you are building: `iohr ext dev` keeps their `STATE` and `CACHE` in
  `~/.inorbit/ext-dev/NAME/`, owner-only, apart from the installed ones.
- The runner's one working directory, `runner/coordinate` in the config directory, is
  empty: Claude Code keeps the session itself, in its own directory.
- Approved run asks: the pinned device keys and the record of decisions carried out, in
  `runner/approvals/<account>` in the config directory, owner-only. The run key is in the
  credential store.
- `iohr` talks to `api.inorbit.hr` and `auth.inorbit.hr` (the extensions catalogue is
  on the API host), and to the extension registry only in `iohr ext install`, `upgrade`
  and `sync` (ADR 0012). No telemetry, no update
  check. `iohr sdk add` runs your project's package manager, which reaches its own
  registry (SR-31).

The rules are SR-10 to SR-34 in [docs/security/requirements.md](../docs/security/requirements.md);
the layout is [ADR 0009](../docs/adr/0009-the-command-line.md).

## Dependencies

Each runtime dependency, and why (SR-20):

| Crate | Why |
|---|---|
| clap, clap_complete | Arguments, help and shell completions |
| tokio | The runtime for HTTP; current-thread only |
| inorbithr | The Rust SDK's runtime: the client every command calls through, with its retries, errors and user agent (ADR 0009, ADR 0011) |
| reqwest (rustls) | HTTPS with the platform's trust store; no OpenSSL (the sign-in flows in iohr-auth) |
| serde, serde_json, toml | The config file and the API's JSON |
| toml_edit | Rewriting `config.toml` without dropping the SDKs' keys or anyone's comments; already in the lock through proc-macro-crate |
| thiserror | Typed errors in the libraries |
| time, url | RFC 3339 timestamps; URL checks (HTTPS only, the path stays on the API host) |
| keyring-core and one store per OS | The OS credential store: apple-native-keyring-store, windows-native-keyring-store, zbus-secret-service-keyring-store (pure Rust D-Bus, no libdbus) |
| zeroize | Secrets are wiped from memory when dropped |
| rpassword | `iohr connections add` and `reconnect`: a secret field typed on the terminal without echo (Unix and Windows consoles); with rtoolbox, two small crates and no new transitive dependency |
| etcetera | The platform's config directory |
| duckdb (bundled, Unix only) | The session host's durable task queue (RFC 0213): tasks and Roles survive a restart of the host or the machine, and Roles take work at the same time on connections of one process. Bundled so no system library is needed; it builds DuckDB from C++ source (bigger binary, longer first build) |
| rustix (`process`, `fs`; Unix only) | The current user's id (`process`) and the session host's one-per-project lock, `flock` (`fs`): safe calls, no `unsafe` code |
| base64, sha2 | Reading a token's claims to name its account; the PKCE S256 challenge |
| getrandom | Request ids and retry jitter |
| typify, schemars, syn, quote, prettyplease | `iohr sdk generate`: the models of a generated surface as Rust types, formatted (schemars reads the schemas, syn and quote add each model's constructor and setters, prettyplease prints them) |
| minijinja | The templates the generated surface's operations, profiles and markers are rendered from, embedded in the binary |
| heck | Case changes for generated names (`AcmeCi`, `get_me`) |
| sigstore-verify | Extensions: verifies Sigstore bundles (Fulcio chain, SCT, Rekor inclusion, DSSE) offline against the trusted root it embeds; from the sigstore project |
| flate2 (pure Rust backend) | Extensions: the gzip of an extension's layer; the tar inside is read by `iohr` itself, one regular file only |
| semver | Extensions: the newest release among a registry's tags |
| sha2 | Also the digests of OCI manifests and blobs |
| regex (no Unicode tables but the Perl classes) | `iohr lab check`: the redaction rules are regular expressions published as data (`spec/lab/rules.json`); already in the lock through two other crates |

The OCI registry client is written here over reqwest (ADR 0012): the published crates
added a licence outside `deny.toml` and about 30 crates for four read-only calls.
