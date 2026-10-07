# iohr, the InOrbit command line

Pre-release (0.x): commands and output can still change between versions. This page
describes `main`; a change merged after the latest pre-release ships with the next one
([CHANGELOG.md](CHANGELOG.md), [releases](https://github.com/inorbithr/sdk/releases?q=iohr)).

```sh
brew install inorbithr/tap/iohr                                   # macOS, Linux
curl -fsSL https://packages.inorbit.hr/install.sh | sh            # Linux, macOS, no sudo
powershell -c "irm https://packages.inorbit.hr/install.ps1 | iex" # Windows
```

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
| `iohr login [--web \| --device] [--insecure-storage]` | Sign in and add a profile; `--insecure-storage` keeps the token in a 0600 file on a machine without a credential store |
| `iohr login --with-token` | Read an API token from stdin and add a profile |
| `iohr logout` | Forget the profile and its credential on this machine |
| `iohr profile list \| use \| show` | The profiles here; the default one |
| `iohr whoami` | Subject, account, plan, scopes and expiry of the active profile |
| `iohr auth token [--format text\|json]` | The profile's access token on stdout, refreshed first when less than a minute is left; for programs, such as the SDKs, that call with your login (below) |
| `iohr accounts list` | The accounts the credential can see |
| `iohr token create --name N --scope S,... [--days 30] \| list [--status S] [--query Q] \| revoke ID` | API tokens for an account (a signed-in person only) |
| `iohr api <METHOD> <PATH>` | One call; `-f k=v` string fields, `-F k=json` typed fields, `--input file`, `-i` the status and request id before the body; `--all` GETs every page of a list (`next_page_token`) as one answer, `--max-pages N` bounds it |
| `iohr openapi pull` | The OpenAPI document this credential sees, to `openapi.json` |
| `iohr sdk generate --lang rust\|typescript\|python\|go\|java\|csharp --for P... --out DIR` | A surface cut to what the profiles may call, into your repository, with `iohr.lock` beside the directory; `--from NAME=FILE` works offline, `--package` names the surface's package or namespace (Go, Java, C#), `--runtime` the runtime's name in your project, `--force` replaces a non-empty directory |
| `iohr sdk config [--profile NAME] [--for TYPE]` | The configuration an SDK client built with `load` would use here: each setting with its source, the credential chain, the pipeline, what was ignored; secrets redacted; offline (below) |
| `iohr sdk check [--lock FILE] [--files]` | Fetch every profile's document again and exit 1 with what moved when the cut changed; for CI, `IOHR_TOKEN_<PROFILE>` stands in for a profile |
| `iohr sdk add [rust\|typescript\|python\|go] [--version V] [--dry-run]` | Add the published SDK to the project here with the package manager it already uses; the language comes from the project's files when left out (below) |
| `iohr sdk examples --from [NAME=]FILE [--lang L]... [--out FILE]` | One short program per operation and language that calls it with the published runtime, as JSON keyed by operation id; what the API reference shows beside each operation |
| `iohr profile account NAME ID\|SLUG` | Point a signed-in profile at one of its teams, the account `sdk generate` cuts to |
| `iohr domains add \| verify \| confirm \| list \| rm` | Prove the account controls a domain with one DNS TXT record; `verify --wait` checks every 10 s for up to `--timeout` seconds (600) |
| `iohr connectors list [--category C] \| show ID` | The catalogue of apps a connection can be made from: sign-in modes and their fields, settings, actions, the hosts each may call, AI models labelled |
| `iohr connections list \| show \| add \| test \| history \| pause \| resume \| rename \| delete \| reconnect` | The account's connections (RFC 0044): connect an app with a key (asked for without echo) or by signing in at the provider in a browser; `delete` asks first unless `--yes` |
| `iohr connections grant \| grants \| revoke-grant` | Grant a product, an API key or an agent named actions of a connection until an expiry, list and revoke grants |
| `iohr ext search \| show` | The extensions catalogue (RFC 0073): `search [QUERY] [--kind K] [--all \| --page-size N]` lists name, kind, publisher, latest version and visibility; `show PUBLISHER/NAME` the description, publisher, signer, scopes, privileges in plain words, evidence, versions and the install line. API host only, `extensions:read` |
| `iohr ext install \| list \| upgrade \| remove \| verify \| sync` | Extensions: install (`NAME` from the registry, or `PUBLISHER/NAME` through the catalogue), verify and pin them; one that declares privileges for its system service asks first unless `--yes`; `iohr <name> ...` runs one |
| `iohr config set \| get \| unset` | `ext.registry` (a mirror) and `ext.trusted_keys` (keys a mirror re-signs with) |
| `iohr lab check [PATH...] [--config FILE] [--strict FILE]` | Check RFCs and studies as the InOrbit site checks its own: file names, front matter (with the review's `reviewed: YYYY-MM-DD` and `reviewer`), status logs, classified span markers, and redaction on public documents as an uncleared reader sees them (text inside `[[classified:LEVEL reason="..."]]...[[/classified]]` or a ```` ```classified level=LEVEL reason="..." ```` block is exempt, its reason is not); `--strict` adds a rules file such as the platform's `docs/lab/strict.json`, whose hits name the rule and never what matched (its `pending-sweep.json` exemptions are not read); offline, exit 1 with every finding ([Lab documents](https://docs.inorbit.hr/docs/lab)) |
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

From a company's mirror: `iohr config set ext.registry registry.acme.hr/inorbit/iohr-ext`,
plus `iohr config set ext.trusted_keys mirror.pub` if the mirror re-signs. A mirror that
needs a login reads `IOHR_EXT_REGISTRY_AUTH=user:password` from the environment. How to
check an extension by hand: [verifying extensions](../docs/security/verifying-extensions.md).

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
- `iohr` talks to `api.inorbit.hr` and `auth.inorbit.hr` (the extensions catalogue is
  on the API host), and to the extension registry only in `iohr ext install`, `upgrade`
  and `sync` (ADR 0012). No telemetry, no update
  check. `iohr sdk add` runs your project's package manager, which reaches its own
  registry (SR-31).

The rules are SR-10 to SR-32 in [docs/security/requirements.md](../docs/security/requirements.md);
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
| typify, schemars, syn, prettyplease | `iohr sdk generate`: the models of a generated surface as Rust types, formatted (schemars reads the schemas, syn and prettyplease print them) |
| minijinja | The templates the generated surface's operations, profiles and markers are rendered from, embedded in the binary |
| heck | Case changes for generated names (`AcmeCi`, `get_me`) |
| sigstore-verify | Extensions: verifies Sigstore bundles (Fulcio chain, SCT, Rekor inclusion, DSSE) offline against the trusted root it embeds; from the sigstore project |
| flate2 (pure Rust backend) | Extensions: the gzip of an extension's layer; the tar inside is read by `iohr` itself, one regular file only |
| semver | Extensions: the newest release among a registry's tags |
| sha2 | Also the digests of OCI manifests and blobs |
| regex (no Unicode tables but the Perl classes) | `iohr lab check`: the redaction rules are regular expressions published as data (`spec/lab/rules.json`); already in the lock through two other crates |

The OCI registry client is written here over reqwest (ADR 0012): the published crates
added a licence outside `deny.toml` and about 30 crates for four read-only calls.
