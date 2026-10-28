# iohr, the InOrbit command line

Pre-release (0.x): commands and output can still change between versions. This page
describes `main`; a change merged after the latest pre-release ships with the next one
([CHANGELOG.md](CHANGELOG.md), [releases](https://github.com/inorbithr/sdk/releases?q=iohr)).

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
choose one. Only enter a code you started yourself.

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
| `iohr sdk generate --lang rust\|typescript\|python\|go\|java\|csharp\|swift --for P... --out DIR` | A surface cut to what the profiles may call, into your repository, with `iohr.lock` beside the directory; `--from NAME=FILE` works offline, `--package` names the surface's package or namespace (Go, Java, C#), `--runtime` the runtime's name in your project, `--force` replaces a non-empty directory |
| `iohr sdk config [--profile NAME] [--for TYPE]` | The configuration an SDK client built with `load` would use here: each setting with its source, the credential chain, the pipeline, what was ignored; secrets redacted; offline (below) |
| `iohr sdk check [--lock FILE] [--files]` | Fetch every profile's document again and exit 1 with what moved when the cut changed; for CI, `IOHR_TOKEN_<PROFILE>` stands in for a profile |
| `iohr sdk add [rust\|typescript\|python\|go] [--version V] [--dry-run]` | Add the published SDK to the project here with the package manager it already uses; the language comes from the project's files when left out (below) |
| `iohr sdk examples --from [NAME=]FILE [--lang L]... [--out FILE]` | One short program per operation and language that calls it with the published runtime, as JSON keyed by operation id; what the API reference shows beside each operation |
| `iohr profile account NAME ID\|SLUG` | Point a signed-in profile at one of its teams, the account `sdk generate` cuts to; a later `iohr login` keeps it |
| `iohr coordinator jobs list \| create`, `asks list \| create` | The coordinator's jobs and queued asks (RFC 0120, RFC 0138): list jobs by state, file a job (`--brief -` reads standard input; `--ask` links a queued ask), list asks, queue an ask (`asks create --kind K --body -`; the kind is required so the planner can file it); `--account <id>` on `jobs list|create` and `asks list` acts on a team you belong to (default: your own account; needs the coordinator release that accepts `account_id`). `iohr coordinator brief JOB` prints the job's brief (task, acceptance, files, document excerpt, capped at the kind's token budget, RFC 0132) for a worker to start from; the size goes to stderr. Needs a person's `iohr login`, not an API key |
| `iohr domains add \| verify \| confirm \| list \| rm` | Prove the account controls a domain with one DNS TXT record; `verify --wait` checks every 10 s for up to `--timeout` seconds (600) |
| `iohr connectors list [--category C] \| show ID` | The catalogue of apps a connection can be made from: sign-in modes and their fields, settings, actions, the hosts each may call, AI models labelled |
| `iohr connections list \| show \| add \| test \| history \| pause \| resume \| rename \| delete \| reconnect` | The account's connections (RFC 0044): connect an app with a key (asked for without echo) or by signing in at the provider in a browser; `delete` asks first unless `--yes` |
| `iohr connections grant \| grants \| revoke-grant` | Grant a product, an API key or an agent named actions of a connection until an expiry, list and revoke grants |
| `iohr ext search \| show` | The extensions catalogue (RFC 0073): `search [QUERY] [--kind K] [--all \| --page-size N]` lists name, kind, publisher, latest version and visibility; `show PUBLISHER/NAME` the description, publisher, signer, scopes, privileges in plain words, evidence, versions and the install line. API host only, `extensions:read` |
| `iohr ext install \| service \| list \| upgrade \| remove \| verify \| sync` | Extensions: install (`NAME` from the registry, or `PUBLISHER/NAME` through the catalogue), verify and pin them; one that declares privileges for its system service asks first unless `--yes`; one with a system service of its own (`inorbit/capture`) offers to set it up with sudo after installing (it lists the interfaces it detects and suggests one; `--interface` repeatable, or `--all-interfaces`), and `iohr ext service NAME` does it later; `iohr <name> ...` runs one |
| `iohr ext init \| dev \| lint` | Build an extension locally (PRD 0084): `init PUBLISHER/NAME --lang rust\|python\|shell [--dir DIR]` scaffolds a folder (manifest with every field, entry point on the token channel, self-test, README, signing release workflow; a name holding `tbd` is refused); `dev [PATH] [--test]` runs it, or its tests, with the token channel for exactly the manifest's scopes, its secrets, `STATE` and `CACHE` under `~/.inorbit/ext-dev/NAME/` and its output timestamped, never touching installed extensions; `lint [PATH] [--preview] [--json]` checks `extension.toml` and says what it may do in plain words. Offline, except the token channel's sign-in |
| `iohr config set \| get \| unset` | `ext.registry` (a mirror) and `ext.trusted_keys` (keys a mirror re-signs with) |
| `iohr lab check [PATH...] [--config FILE] [--strict FILE]` | Check RFCs and studies as the InOrbit site checks its own: file names, front matter (with the review's `reviewed: YYYY-MM-DD` and `reviewer`), status logs, classified span markers, and redaction on public documents as an uncleared reader sees them (text inside `[[classified:LEVEL reason="..."]]...[[/classified]]` or a ```` ```classified level=LEVEL reason="..." ```` block is exempt, its reason is not); `--strict` adds a rules file such as the platform's `docs/lab/strict.json`, whose hits name the rule and never what matched (its `pending-sweep.json` exemptions are not read); offline, exit 1 with every finding ([Lab documents](https://docs.inorbit.hr/docs/lab)) |
| `iohr decisions list \| show \| create \| save \| status \| comment \| link-pr \| review \| diagram` | RFCs, PRDs and ADRs in the Decisions product (platform RFC 0065, RFC 0081; `iohr rfc` until core ADR 0056, removed with no alias) through the SDK's `decisions` operations (`/v1/decisions/...`), scopes `decisions:read` and `decisions:write`: a document by its number (`0065`, a part `0065.1`; `PRD 0001` or `ADR 0003` names its kind, else `--kind rfc|prd|adr|study|all`, default `rfc`) across the account's spaces (`--space` narrows), `create --kind` for a PRD or an ADR, `create` with `--file` saves the text straight away, `save` a new version from a file (refused when someone saved since), `status`, comments that name a pull request first (`owner/repo#N "title": open`, `merged`, `live`), review requests (`--reviewer` or `IOHR_DECISIONS_REVIEWER`; `--reason` as a comment), diagrams from a model file; `--json` on each |
| `iohr runner register [--name N] [--capacity N] [--always-on] [--account A] \| run [--name N \| --runner ID] [--interval 20] [--capacity N] [--kinds K,...] [--account A] \| config show [--capacity N] [--kinds K,...] \| status [--account A] \| gate-report --job J --lease L --token N [--repo DIR] [--command C] [--timeout S] [--name N \| --runner ID] [--account A] \| run-key [--proof FILE] [--label L] [--account A]` | This machine as a coordinator runner (platform RFC 0120), in the profile's account or `--account A`; every call names it, because the sign-in token's own account is the personal one: `register` records its name, system, `--capacity` (1 to 32) and the one `claude` on your PATH; `run` heartbeats with the capacity, version and kinds in effect, accepts turns the coordinator offers, up to that capacity at once, and runs each on your installed, unmodified `claude -p`, on your subscription only (below), and after a job turn that replied sends its gate report when the pack names the checkout (below); `status` lists your runners and what this machine can run; `gate-report` runs the check for a job whose lease you hold and sends the signed result (below); `run-key` makes and enrols the key `run` uses to carry out approved run asks (below) |
| `iohr browser install [--print]` | Open the console's install page for InOrbit Trails, the browser extension, in the default browser: it adds Trails from that browser's store, or shows the load-unpacked steps while the store has no listing yet; `--print` prints the address for a browser on another machine (or where none can open) |
| `iohr tunnel --host H [--env production\|local] [--port P] [--dry-run]` | A loopback pipe from this machine into an environment's Envoy edge: `ssh -N -L` to `H` (or `IOHR_TUNNEL_HOST`), bound to 127.0.0.1 on both ends, so the real site opens at `http://www.localhost:<port>/` (also `console`, `docs`, `auth`) with no DNS or certificate; `--port` moves the listening side, `--no-engine` skips the engine LB the local environment also forwards, `--dry-run` prints the addresses and the ssh command. It ends at Envoy, so every gate stays. Known gap: sign-in cookies belong to the production domain, so signed-in pages do not work on localhost |
| `iohr completion <shell>` | A completion script for bash, zsh, fish, elvish or PowerShell |

Every command takes `--profile` (or `IOHR_PROFILE`), `--json` and `--verbose`.
`--verbose` prints method, path, status, time and request id on stderr; never a header,
a query value or a body.

Exit codes: 0 success, 1 a failed call, 2 a usage error, 3 not signed in or the token
was refused, 4 forbidden by scope, role or plan.

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
namespace (default `InOrbit.Generated`). `--lang` takes all six: `rust`, `typescript`,
`python`, `go`, `java` and `csharp` (ADR 0013). In CI, with a token
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

`iohr project init NAME` sets up a project you build yourself (PRD 0088) with one command,
and tells you what you now have. The local half is real: `~/development/NAME` from a template
(`--template demo`: a private macOS service that heartbeats and serves a tiny panel, with the
manifest of `iohr ext init`), a git repository with one commit, the project's own state under
`~/.inorbit/projects/NAME/` (never in the repository), a Keychain slot reserved by name, and a
smoke test that runs the project once and reads its heartbeat back, with no token channel, so
its token ask is refused: the proof the channel is its only credential.

```sh
iohr project init demo
iohr project init my-shop --template demo --dir ~/work/my-shop --json
```

The steps that need InOrbit (the project as a Decisions space, its private listing, its own
scopes and tokens, a memory namespace, monitors and notifications) are reported as `not yet`,
each with the API it waits on; nothing is skipped in silence, and a step that needs your or an
admin's say becomes an ask. Every step is re-runnable, and nothing is overwritten. Then
`iohr ext lint --preview`, `iohr ext dev` and `iohr ext dev --test` work in the folder as for
any extension, and InOrbit Desktop's Code opens it after asking you.

## Runner

```sh
claude                                  # once: /login with your subscription
iohr runner register                    # this machine, by its host name
iohr runner run                         # until Ctrl-C
iohr runner status
```

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
machine. A non-zero exit is sent as it is: the job holds at Checked, which is the honest
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
newline and the line `claude auth status` prints, and the method's name.

### Approved run asks

`iohr runner run` also carries out run asks the owner approved and pinned to this runner
(platform RFC 0172 and RFC 0153). It needs two things set up once, and stays off with a
note saying which is missing:

- A run key. `iohr runner run-key` makes a P-256 key, kept in the credential store your
  profile uses (like the coordinator key `register` makes, and never printed), and shows
  its public key. Have your device sign the enrolment message (`iohr-approval-runner-v1`,
  the public key, the label, the time, a nonce and the device key id), save that proof as
  JSON `{key_id, signed_at, nonce, signature}`, and run `iohr runner run-key --proof FILE`:
  the runner adds its own signature of possession and enrols the key. `run-key` needs no
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
