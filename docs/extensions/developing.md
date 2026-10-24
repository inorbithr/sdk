# Developing an extension

An extension is a program `iohr` runs as its own process (ADR 0012, platform RFC 0028).
It never holds your sign-in: it asks `iohr` for short-lived tokens over a private channel,
and gets them only for the API scopes its manifest declares. This page is the local loop
of PRD 0084 slice 1: scaffold, check, run, test. Packing, signing with a local key and
installing to this Mac or a host come in later slices; publishing to the catalogue is
the release workflow the scaffold already contains.

## From an empty folder to a heartbeat

```sh
iohr ext init inorbit/hello --lang shell
cd hello
iohr ext lint --preview
iohr ext dev
```

`init` takes `PUBLISHER/NAME` (the catalogue's form), `--lang rust`, `python` or `shell`,
and `--dir` to write somewhere other than `./NAME`. It refuses a folder that already has
files, and any name holding `tbd`: name the extension for what it does before building
it. It writes:

| File | What it is |
|---|---|
| `extension.toml` | The source manifest, every field present and commented |
| `bin/NAME` (shell), `src/MODULE/` (Python), `src/main.rs` (Rust) | The program: asks the token channel for a token, writes `$STATE/heartbeat.json` every 60 s |
| `tests/` | A self-test for `iohr ext dev --test` |
| `README.md` | What it does and what it may do |
| `.github/workflows/release.yml` | Builds, derives the artifact's config with `iohr ext lint --json`, pushes the OCI artifact, signs it keylessly and attaches the signature and provenance with `oras attach` |
| `tools/ext_artifact.py` | Builds the OCI layout from the lint's config, deterministically |

The shell scaffold needs nothing but `sh` and `curl`. The Python one needs `python3` and
nothing from PyPI: `channel.py` speaks the token channel with the standard library. The
Rust one uses the `inorbithr` crate's `ExtSocketToken`.

## The manifest

`extension.toml` is TOML. Unknown keys are errors, so a typo never silently drops a
permission.

```toml
id = "inorbit/hello"             # publisher/name
version = "0.1.0"
kind = "program"                 # RFC 0073 kind
delivery = "program"             # RFC 0073.1
entrypoint = "hello"             # the program's path inside the artifact
args = []
description = "One line about what it does."
api = ">=2026.10"
secrets = []                     # [{ name, ref, env }]: references, never values

[runs]                           # as = service | timer | oneshot; every; restart; shutdown_grace
[permissions]                    # scopes, privileges, network, exec
[permissions.filesystem]         # read, write: $STATE and $CACHE are its own; $REPOS the user's repository folders
[permissions.data]               # leaves: what it may send up
[resources]                      # cpu, memory, disk, nice
[health]                         # heartbeat, heartbeat_every, stale_after, check
[targets]                        # platforms, kinds (desktop, host, agent, cluster), needs
[panel]                          # title, entry: a program that serves a panel its host shows
[upgrade]                        # strategy, keep_previous, migrate, rollback
[dev]                            # lang: how iohr ext dev runs the folder
```

`permissions.scopes` and `permissions.privileges` mean what `scopes` and `privileges`
have always meant in an artifact's config, and `iohr ext lint --json` writes them at the
config's top level, so installed artifacts and the catalogue read them as before.
Scopes are enforced by the token channel. Network hosts, files and programs are declared
and shown now; on Linux the installer will enforce them through the systemd unit it
writes (a later slice), and on macOS they stay declared and audited.

`permissions.data.leaves` names what the extension may send up. Each must be one of ADR
0063's non-sensitive kinds: `ids`, `measurements`, `hashes`, `declared.names`,
`evidence.claims`, `check.result`, `monitor.result`, `extension.health`. Anything else
leaves only through a per-item share, never from an extension.

## `iohr ext lint`

```sh
iohr ext lint [PATH]             # exit 0, or 1 with every problem
iohr ext lint --preview          # what it may do, in plain words
iohr ext lint --json             # findings, the preview structured, the artifact's config
```

It fails on:

- an unknown key (`unknown_key`);
- a value that looks like a secret: a token, a key, a PEM block, a URL with a user name or
  password, a connection string with a password, a long random string (`secret_value`);
- a `permissions.data.leaves` kind outside the list above (`data_class`);
- a `targets.kinds` the kind cannot be installed on: a program on `desktop`, `host`,
  `agent` or `cluster`, an `agent-plugin` or a `web` on `cluster` (`target_kind`); a
  `cluster` target runs the program artifact as a pod, so it needs `delivery = "program"`
  and a `linux/...` platform when `targets.platforms` is given (`target_kind`);
- a `[panel]` that is not a `title` (1 to 64 characters, one line) and an `entry` (a path
  starting with one `/`, no `..`, no spaces), or on anything but a program (`panel`);
- a name holding `tbd` (`tbd_name`);
- anything else the artifact's config refuses, such as a version that is not semver
  (`invalid`).

The preview of the scaffold:

```text
inorbit/hello 0.1.0: Writes a heartbeat every minute and asks iohr for a token with its declared scope.
May ask iohr for API tokens with identity:read, and nothing else.
Holds no special privileges.
Reaches no network host.
May read and write its own state and cache, nowhere else.
Starts no other program.
Asks for no secrets.
Sends up: its health (state, heartbeat time, version).
Runs as a service, restarted when it fails.
May be installed on this computer and a host over SSH.
Needs sh and curl on the target.
Scopes are enforced by iohr's token channel. Network hosts, files and programs are declared and shown, not yet enforced; on macOS they stay declared and audited only.
```

`--json` carries the same sentences in `preview.lines`, so the desktop app shows exactly
what the terminal shows.

## `iohr ext dev`

```sh
iohr ext dev [PATH]              # run it until Ctrl-C
iohr ext dev --test              # run tests/ instead
```

What it does, in order:

1. Lints the manifest; any finding stops it.
2. Opens the token channel for exactly `permissions.scopes`. Every request is logged; one
   for a scope the manifest does not declare is refused and printed in red, which is how
   you learn what the extension really needs. Tokens come from your profile, as for an
   installed extension (`iohr login`, or `IOHR_TOKEN` in a pipeline). Signed in as a
   person, each token is narrowed to the scopes asked for through a short-lived account
   key that is revoked at once (also on a timeout or cancel once the key has an id), so you need to be an admin or owner of the account;
   a token profile must hold the scopes itself. The SDK waits up to 45 s for `iohr`, which
   answers with the reason when it cannot.
3. Resolves each secret: `IOHR_EXT_SECRET_<NAME>` first (`github_token` reads
   `IOHR_EXT_SECRET_GITHUB_TOKEN`), then the Keychain item with service
   `hr.inorbit.iohr.ext` and the `ref` as its account
   (`security add-generic-password -s hr.inorbit.iohr.ext -a github-token -w` on macOS).
   The program receives it as the secret's `env`, and never sees the
   `IOHR_EXT_SECRET_*` variables themselves.
4. Sets `STATE` and `CACHE` to `~/.inorbit/ext-dev/NAME/state` and `cache` (mode 0700),
   and `IOHR_EXT_DEV=1`.
5. Removes `IOHR_TOKEN`, `IOHR_TOKEN_*`, `INORBIT_TOKEN*` and `INORBIT_KEY_*` from the
   child's environment, and every folder holding an `iohr` program from its `PATH`
   (saying which): `iohr auth token` would hand out a full-scope token around the
   channel's check.
6. Runs the program: `bin/NAME` for shell, `python3 -m MODULE` with `src` on
   `PYTHONPATH` for Python, `cargo run` for Rust, from the folder, with `IOHR_EXT_API`
   and `IOHR_EXT_TOKEN_SOCKET` set.
7. Streams its output with the time (`out` and `err`), and logs each new heartbeat,
   marking it `"dev": true` if the program did not.

It never reads or writes the installed extensions, their lock or their data directory.

With `--test` it runs the folder's tests the same way: each `tests/*.sh` with `sh`,
`python3 -m unittest discover -s tests`, or `cargo test`. `HOME` is a fresh empty folder
and `TMPDIR` is under `CACHE`, and the run fails, naming each file, when the tests wrote
anything outside `STATE` and `CACHE`: in the extension's folder (build output such as
`target/` and `__pycache__/` aside) or in `HOME`.

## The token channel from your own code

Rust, on the `inorbithr` crate:

```rust
use inorbithr::{Client, ExtSocketToken, TokenProvider as _};

// Explicitly, for chosen scopes:
if let Some(p) = ExtSocketToken::from_env() {
    let token = p.scopes(["identity:read"]).token().await?;
}
// Or a client: with IOHR_EXT_TOKEN_SOCKET set, Client::load uses the channel and
// nothing else (no INORBIT_TOKEN, no key, no config file, no iohr login), and calls
// IOHR_EXT_API. A token or key set in code is refused.
let client: Client = Client::load()?;
```

Any language: connect to the Unix socket `IOHR_EXT_TOKEN_SOCKET` (a named pipe on
Windows), send

```text
POST /token HTTP/1.1
content-type: application/json
content-length: 28

{"scopes":["identity:read"]}
```

and read `{"access_token": "...", "expires_at": "<RFC 3339>"}`, or a 403 with
`{"error": "scope_not_declared"}` for a scope the manifest lacks. No scopes means every
scope the manifest declares. Keep the token in memory; never print or write it.

## Releasing

Tag `vVERSION` with the version in `extension.toml` and push the tag. The scaffold's
release workflow builds per platform, runs `iohr ext lint --json` for the config, builds
the OCI layout with `tools/ext_artifact.py`, pushes it to `ghcr.io/OWNER/iohr-ext/NAME`,
and from a job that runs no repository code attaches a keyless cosign signature and SLSA
provenance as OCI referrers. `iohr ext install` verifies both against
`.github/workflows/release.yml` at a tag of your repository; installing another
publisher's extension needs `ext.registry` set to its registry.

## A panel, `$REPOS` and the `cluster` target (RFC 0143)

A program can serve a panel: `[panel]` with `title` and `entry` (the path on its loopback
address the host opens); both are required and no other key is read. `$REPOS` joins `$STATE`
and `$CACHE` in `permissions.filesystem`: it stands for the repository folders the user
added in the extension's settings in its host, kept in the host's per-extension config
(`{"repos": ["/absolute/path"]}`), an empty list until one is added. `iohr ext dev` reads the
same file at `~/.inorbit/ext-dev/<name>/config.json` and gives the program `REPOS` as the
roots joined with `:`, empty when there is no file. `targets.kinds` may name `cluster` for a
program: it is the same signed artifact run as a pod by a deployment. iohr itself installs
on the desktop only; deploying to a cluster is the platform's, and it begins with
`iohr ext verify ARTIFACT --bundle FILE --publisher-key PEM`
([Verifying extensions](../security/verifying-extensions.md)).
