# Command line

The `iohr` binary, a Cargo workspace of its own (ADR 0009). Edition 2024, MSRV 1.94.
Read the root `AGENTS.md`, `docs/design.md` sections 4 to 6 and
`docs/security/requirements.md` first; this file adds what is specific to the CLI.

## Commands

- `mise run cli:check`: `cargo fmt --check`, clippy (pedantic, warnings are errors),
  `cargo nextest run`, doctests, `cargo deny check`.
- `mise run cli:fmt`, `mise run cli:audit`.
- `mise run cli:keyring` (Linux): the credential-store test against a throwaway Secret
  Service in a private D-Bus session.
- `mise run cli:fuzz` (nightly, `FUZZ_SECONDS` each): the config, error and token readers.
- Single test: `cargo nextest run -p iohr -E 'test(name)'` in `cli/`.

## Layout

```
cli/
  Cargo.toml          workspace, lints, dependency versions
  crates/iohr-auth/   Redacted, Credential, Claims, Config and profiles, Store
  crates/iohr/        lib: api (private HTTP client), cli (clap), commands, context
                      main.rs: argv guard, parse, run, exit code
  fuzz/               cargo-fuzz targets; excluded from the workspace
```

## Rules

- `main.rs` holds no logic. Libraries return typed errors; only `Error::exit_code` maps
  them to 0, 1, 2, 3 or 4, and those codes are a contract.
- Every secret is a `Redacted<String>` from the moment it is read until the one place it
  is sent or stored. Never `Debug`-print, log or put a secret in an error message.
- A token never comes from argv (`refuse_secrets_in_args` runs before parsing). New
  input of a secret goes through stdin or the environment.
- Secrets go only through `Store`: `KeyringStore` by default, `FileStore` only with
  `--insecure-storage`. The config file never holds one. Entries are keyed by profile and
  account (`EntryKey`).
- Credential-store calls block: run them through `context::blocking`.
- Only the API host: paths are checked to stay on it, base URLs are HTTPS (loopback
  excepted), redirects are not followed. No telemetry, no update check.
- `--verbose` prints method, path, status, time and request id; never a header, a query
  value or a body. The `verbose_output_never_shows_the_token` test covers every command:
  add each new command to it.
- Output: results on stdout (tables, or JSON with `--json`), notes on stderr, so pipes
  get only data. `token create` prints the new token alone on stdout.
- Dependencies: the table in `README.md`. A new one needs a reason there and in the PR.
- `unwrap`/`expect` only in tests; `unsafe` is forbidden; clippy pedantic stays on.
- Commit scope `cli`.
