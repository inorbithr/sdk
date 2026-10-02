# iohr, the InOrbit command line

Not released yet: no package, installer or binary is published. Build it from this
repository to try it. Packages for APT, Homebrew and winget come with platform RFC 0021.

```sh
cargo install --locked --path cli/crates/iohr
```

## Sign in

This version signs in with an API token. Create one in the console (API tokens and
keys), then pipe it in; a token is never accepted as an argument, where shell history
and the process list would keep it.

```sh
iohr login --with-token --profile work < token.txt
iohr whoami
```

Signing in with a browser or a device code arrives in the next version.

For CI and one-off use, `IOHR_TOKEN` is used in memory and nothing is written:

```sh
IOHR_TOKEN="$(cat token.txt)" iohr api GET /v1/radar/digests -f limit=5
```

## Commands

| Command | What it does |
|---|---|
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
- `iohr` talks to `api.inorbit.hr` only (and, with browser sign-in, `auth.inorbit.hr`).
  No telemetry, no update check.

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
| base64 | Reading a token's claims to name its account |
| getrandom | Request ids and retry jitter |
