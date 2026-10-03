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
| `iohr completion <shell>` | A completion script for bash, zsh, fish, elvish or PowerShell |

Every command takes `--profile` (or `IOHR_PROFILE`), `--json` and `--verbose`.
`--verbose` prints method, path, status, time and request id on stderr; never a header,
a query value or a body.

Exit codes: 0 success, 1 a failed call, 2 a usage error, 3 not signed in or the token
was refused, 4 forbidden by scope, role or plan.

## Where things are kept

- Profiles: `config.toml` in the platform's config directory (`~/.config/iohr` on
  Linux, `~/Library/Application Support/hr.InOrbit.iohr` on macOS,
  `%APPDATA%\InOrbit\iohr\config` on Windows). No secret is ever written there.
- Secrets: the operating system's credential store (macOS Keychain, Windows Credential
  Manager, the Secret Service on Linux), one entry per profile and account. On a machine
  without one, `--insecure-storage` keeps the token in a file with mode 0600 instead.
- `iohr` talks to `api.inorbit.hr` and `auth.inorbit.hr` only. No telemetry, no update
  check.

The rules are SR-10 to SR-24 in [docs/security/requirements.md](../docs/security/requirements.md);
the layout is [ADR 0009](../docs/adr/0009-the-command-line.md).

## Dependencies

Each runtime dependency, and why (SR-20):

| Crate | Why |
|---|---|
| clap, clap_complete | Arguments, help and shell completions |
| tokio | The runtime for HTTP; current-thread only |
| reqwest (rustls) | HTTPS with the platform's trust store; no OpenSSL |
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
