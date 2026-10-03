# iohr, the InOrbit command line

Pre-release (0.x): commands and output can still change between versions.

```sh
brew install inorbithr/tap/iohr                                   # macOS, Linux
curl -fsSL https://packages.inorbit.hr/install.sh | sh            # Linux, macOS, no sudo
powershell -c "irm https://packages.inorbit.hr/install.ps1 | iex" # Windows
```

Debian and Ubuntu (a signed APT repository), winget and every other way are on
[docs.inorbit.hr/docs/command-line](https://docs.inorbit.hr/docs/command-line/). The
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

For CI and one-off use, `IOHR_TOKEN` is used in memory and nothing is written:

```sh
IOHR_TOKEN="$(cat token.txt)" iohr api GET /v1/radar/digests -f limit=5
```

## Commands

| Command | What it does |
|---|---|
| `iohr login [--web \| --device]` | Sign in and add a profile |
| `iohr login --with-token` | Read an API token from stdin and add a profile |
| `iohr logout` | Forget the profile and its credential on this machine |
| `iohr profile list \| use \| show` | The profiles here; the default one |
| `iohr whoami` | Subject, account, plan, scopes and expiry of the active profile |
| `iohr accounts list` | The accounts the credential can see |
| `iohr token create \| list \| revoke` | API tokens for an account (a signed-in person only) |
| `iohr api <METHOD> <PATH>` | One call; `-f k=v` string fields, `-F k=json` typed fields, `--input file` |
| `iohr openapi pull` | The OpenAPI document this credential sees, to `openapi.json` |
| `iohr sdk generate --lang rust\|typescript\|python\|go\|csharp --for P... --out DIR` | A surface cut to what the profiles may call, into your repository, with `iohr.lock` beside the directory; `--from NAME=FILE` works offline |
| `iohr sdk check [--files]` | Fetch every profile's document again and exit 1 with what moved when the cut changed; for CI, `IOHR_TOKEN_<PROFILE>` stands in for a profile |
| `iohr profile account NAME ID\|SLUG` | Point a signed-in profile at one of its teams, the account `sdk generate` cuts to |
| `iohr domains add \| verify \| confirm \| list \| rm` | Prove the account controls a domain with one DNS TXT record; `verify --wait` checks every 10 s |
| `iohr ext install \| list \| upgrade \| remove \| verify \| sync` | Extensions: install, verify and pin them; `iohr <name> ...` runs one |
| `iohr config set \| get \| unset` | `ext.registry` (a mirror) and `ext.trusted_keys` (keys a mirror re-signs with) |
| `iohr lab check [PATH...] [--config FILE]` | Check RFCs and studies as the InOrbit site checks its own: file names, front matter, status logs, redaction on public documents; offline, exit 1 with every finding ([Lab documents](https://docs.inorbit.hr/docs/lab)) |
| `iohr completion <shell>` | A completion script for bash, zsh, fish, elvish or PowerShell |

Every command takes `--profile` (or `IOHR_PROFILE`), `--json` and `--verbose`.
`--verbose` prints method, path, status, time and request id on stderr; never a header,
a query value or a body.

Exit codes: 0 success, 1 a failed call, 2 a usage error, 3 not signed in or the token
was refused, 4 forbidden by scope, role or plan.

## An SDK for your account

```sh
iohr login                                   # a person, or `iohr login --with-token --profile ci`
iohr profile account default acme            # a team's slug or id (a person's profile)
iohr sdk generate --lang rust --for default --for ci --out src/iohr
git add src/iohr src/iohr.lock
```

The surface holds exactly the operations each profile's credential may call, on the
language's runtime (`inorbithr` in Rust and Python, `@inorbithr/sdk` in TypeScript,
`github.com/inorbithr/sdk/go` in Go, `InOrbit.Sdk` in C#); a call a profile may not make
does not compile (in Python, pyright and mypy refuse it). In Go each profile is a package,
and the surface's import path is read from the enclosing `go.mod` (or given with
`--package`); in C#, `--package` names the surface's namespace (default
`InOrbit.Generated`). `--lang` takes `rust`, `typescript`, `python`, `go` and `csharp`
today; Java follows (ADR 0013). In CI, with a token
per profile in `IOHR_TOKEN_<PROFILE>` (`IOHR_TOKEN_CI`, `IOHR_TOKEN_DEFAULT`), `iohr sdk
check` fails with a diff when the API's cut has moved, so a plan change or a revoked
scope is a failing check, not a surprise in production. A person profile cannot sign in
on CI: give it a token there, or leave it out of the lock.

## Domains

```sh
iohr domains add acme.hr                 # prints the TXT record to add
iohr domains verify acme.hr --wait       # looks it up from several resolvers until seen
iohr domains confirm acme.hr             # marks it verified for the account
```

`--subdomain` proves only a subdomain's subtree (`staging.acme.hr`). The token expires
after 7 days unless the domain is verified, and a verified domain is checked again every
day. The calls need the `domains:read` and `domains:write` scopes.

## Extensions

An extension is a separate program that `iohr` installs from an OCI registry, verifies,
pins and runs. The first is the InOrbit agent:

```sh
iohr ext install agent                   # or agent@0.1.0
iohr agent status                        # runs the installed program
```

Before anything is used, `iohr` checks every digest, a signature and SLSA provenance
from InOrbit's release workflow (Sigstore, offline against the root built into `iohr`)
and the manifest's rules; there is no flag that skips this. Install prints who signed it
and the API scopes it may ask for. What is installed is pinned in `iohr-ext.lock`; with
`--lock iohr-ext.lock` the same file goes into your repository, and `iohr ext sync`
installs exactly that elsewhere. Nothing updates by itself: `iohr ext upgrade` does,
when you run it.

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
  `%APPDATA%\InOrbit\iohr\config` on Windows). No secret is ever written there.
- Secrets: the operating system's credential store (macOS Keychain, Windows Credential
  Manager, the Secret Service on Linux), one entry per profile and account. On a machine
  without one, `--insecure-storage` keeps the token in a file with mode 0600 instead.
- Extensions: the platform's data directory (`~/.local/share/iohr/extensions` on
  Linux), owner-only, with the machine's `iohr-ext.lock` and, per version, the program,
  the Sigstore bundles checked at install and a record of the checks.
- `iohr` talks to `api.inorbit.hr` and `auth.inorbit.hr`, and to the extension registry
  only in `iohr ext install`, `upgrade` and `sync` (ADR 0012). No telemetry, no update
  check.

The rules are SR-10 to SR-28 in [docs/security/requirements.md](../docs/security/requirements.md);
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
| thiserror | Typed errors in the libraries |
| time, url | RFC 3339 timestamps; URL checks (HTTPS only, the path stays on the API host) |
| keyring-core and one store per OS | The OS credential store: apple-native-keyring-store, windows-native-keyring-store, zbus-secret-service-keyring-store (pure Rust D-Bus, no libdbus) |
| zeroize | Secrets are wiped from memory when dropped |
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
