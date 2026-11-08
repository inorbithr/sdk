# Files, network and dependencies

Where `iohr` keeps what it writes, which hosts it talks to and when, and every runtime
dependency with the reason it is there. Back to the [iohr overview](../README.md).

The rules behind this page are SR-10 to SR-34 in
[docs/security/requirements.md](../../docs/security/requirements.md); the layout is
[ADR 0009](../../docs/adr/0009-the-command-line.md).

## Where things are kept

| What | Where |
|---|---|
| Profiles and settings | `config.toml` in the platform's config directory: `~/.config/iohr` on Linux, `~/Library/Application Support/hr.InOrbit.iohr` on macOS, `%APPDATA%\InOrbit\iohr\config` on Windows. No secret is ever written there |
| Secrets | The operating system's credential store (macOS Keychain, Windows Credential Manager, the Secret Service on Linux), one entry per profile and account. On a machine without one, `--insecure-storage` keeps the token in a file with mode 0600 instead |
| Installed extensions | The platform's data directory (`~/.local/share/iohr/extensions` on Linux), owner-only: the machine's `iohr-ext.lock` and, per version, the program, the Sigstore bundles checked at install and a record of the checks |
| Extensions you are building | `~/.inorbit/ext-dev/NAME/` (`STATE` and `CACHE`), owner-only, apart from the installed ones |
| The runner's turns | `runner/coordinate` in the config directory, with the job checkouts in `checkouts/<job_id>`; Claude Code keeps the session itself, in its own directory. Mirrors are in `runner/mirrors` and the shared build directories in `runner/target` ([Runner](runner.md#github-access)) |
| Approved run asks | `runner/approvals/<account>` in the config directory, owner-only: the pinned device keys and the record of decisions carried out. The run key is in the credential store |
| Session hosts | `session/<project hash>/` below the data directory: the queue, the socket and the log ([Sessions and the panel](session-and-panel.md#the-queue)) |
| A project's own files | `.inorbit/` and `atlas/` in the project: memory, permissions, handoff notes ([Projects](projects.md), [Roles and memory](roles-and-memory.md)) |

The SDKs read the same `config.toml` ([docs/config.md](../../docs/config.md), section 4):
their `[sdk]` table, their keys inside `[profiles.<name>]`, and profiles that hold only SDK
keys. `iohr` changes only its own keys (`default`, a profile's `kind`, `account`,
`storage`, `issuer`, `client_id`, and `[ext]`) and keeps every other key and comment;
`iohr logout` removes a profile's own keys and leaves the SDK's.

## The hosts it talks to

`iohr` sends no telemetry. What it reaches, and when:

| Host | When |
|---|---|
| `api.inorbit.hr`, `auth.inorbit.hr` | Every signed-in command and the sign-in itself. The extensions catalogue is on the API host |
| The extension registry | Only `iohr ext install`, `upgrade` and `sync` ([ADR 0012](../../docs/adr/0012-extensions.md)) |
| GitHub (`api.github.com`, `github.com`) | `iohr upgrade`, and `iohr runner run` when it looks for a newer release ([staying current](runner.md#staying-current)); the runner's pushes and pull requests for a job ([GitHub access](runner.md#github-access)) |
| `static.rust-lang.org`, `go.dev`, `nodejs.org` | `iohr project init`, to say whether the installed toolchain is behind ([Projects](projects.md#is-the-toolchain-behind)) |
| A template's repository | `iohr project init --template gh:...` or `git:...`, a shallow clone ([Projects](projects.md)) |
| Your package manager's registry | `iohr sdk add`, which runs your project's package manager (SR-31) |
| Your AI provider | `iohr ask` and `iohr runner`, through your own `claude` or `codex` and your own sign-in ([Ask](ask.md#what-goes-to-your-provider)) |

## Dependencies

Each runtime dependency, and why (SR-20). The crate that pulls it in is named when it is
not `iohr` itself.

| Crate | Why |
|---|---|
| clap, clap_complete | Arguments, help and shell completions |
| tokio | The runtime for HTTP; current-thread only |
| inorbithr | The Rust SDK's runtime: the client every command calls through, with its retries, errors and user agent ([ADR 0009](../../docs/adr/0009-the-command-line.md), [ADR 0011](../../docs/adr/0011-runtime-and-surface.md)) |
| reqwest (rustls) | HTTPS with the platform's trust store, no OpenSSL; the sign-in flows in iohr-auth |
| serde, serde_json, toml | The config file and the API's JSON |
| toml_edit | Rewriting `config.toml` without dropping the SDKs' keys or anyone's comments |
| thiserror | Typed errors in the libraries |
| time, url | RFC 3339 timestamps; URL checks (HTTPS only, the path stays on the API host) |
| keyring-core and one store per OS (iohr-auth) | The OS credential store: apple-native-keyring-store, windows-native-keyring-store, zbus-secret-service-keyring-store (pure Rust D-Bus, no libdbus) |
| security-framework (iohr-auth, macOS) | The version apple-native-keyring-store already uses; the quiet store turns the Keychain's dialogs off through it |
| zeroize | Secrets are wiped from memory when dropped |
| rpassword | `iohr connections add` and `reconnect`: a secret typed on the terminal without echo (Unix and Windows consoles) |
| etcetera | The platform's config and data directories |
| aws-lc-rs | The runner's Ed25519 key and P-256 run key, and their signatures; also the crypto backend sigstore-verify is built with |
| cliclack, console | The terminal look: spinners, menus, questions and notes on stderr, text widths and colours; plain lines when there is no terminal, `--json`, `NO_COLOR` or `TERM=dumb` |
| vt100 (no default features) | The panel's model of your shell's screen: what lies under the floating box and whether a program is on the alternate screen |
| duckdb (bundled, Unix only) | The session host's durable task queue: tasks and roles survive a restart of the host or the machine. Bundled so no system library is needed; it builds DuckDB from C++ source (a bigger binary, a longer first build) |
| rustix (Unix only) | The current user's id, the session host's one-per-project lock (`flock`) and the panel's pseudo-terminal: safe calls, no `unsafe` code |
| base64, sha2 | Reading a token's claims to name its account; the PKCE S256 challenge; the digests of OCI manifests and blobs |
| getrandom | Request ids and retry jitter |
| typify, schemars, syn, quote, prettyplease (iohr-codegen) | `iohr sdk generate`: the models of a generated surface as Rust types, formatted (schemars reads the schemas, syn and quote add each model's constructor and setters, prettyplease prints them) |
| minijinja (iohr-codegen) | The templates the generated surface's operations, profiles and markers are rendered from, embedded in the binary |
| heck (iohr-codegen, iohr-openapi) | Case changes for generated names (`AcmeCi`, `get_me`) |
| sigstore-verify | Extensions: verifies Sigstore bundles (Fulcio chain, SCT, Rekor inclusion, DSSE) offline against the trusted root it embeds; from the Sigstore project |
| flate2 (pure Rust backend) | Extensions: the gzip of an extension's layer; the tar inside is read by `iohr` itself, one regular file only |
| semver | Extensions: the newest release among a registry's tags; the version checks of `iohr upgrade`, the runner and the toolchain check |
| regex (no Unicode tables but the Perl classes; iohr and iohr-lab) | `iohr lab check`: the redaction rules are regular expressions published as data (`spec/lab/rules.json`); also template answers and the memory text checks |

The OCI registry client is written here over reqwest ([ADR 0012](../../docs/adr/0012-extensions.md)):
the published crates added a licence outside `deny.toml` and about 30 crates for four
read-only calls.
