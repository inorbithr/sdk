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
- `mise run cli:hydra`: browser and device sign-in, refresh and revocation against the
  Ory Hydra the platform runs (Docker, image pinned by digest).
- `mise run cli:fuzz` (nightly, `FUZZ_SECONDS` each): the config, error, token and
  loopback request readers.
- Single test: `cargo nextest run -p iohr -E 'test(name)'` in `cli/`.

## Layout

```
cli/
  Cargo.toml          workspace, lints, dependency versions
  crates/iohr-auth/   Redacted, Credential, Claims, Config and profiles, Store,
                      Provider (discovery), Authorization<Browser|Device|Granted>,
                      loopback listener, PKCE, Session (refreshing person)
  crates/iohr/        lib: cli (clap), commands, context (the runtime's Client with the
                      profile's credential as its TokenProvider, the --verbose hook), lock,
                      ext (OCI client, Sigstore trust, layer, lock, install store, token
                      channel, run)
                      main.rs: argv guard, parse, run, exit code
  crates/iohr-openapi/ a document normalised (N1 to N6, equal to tools/spec-sync.py),
                      hashed (the cut hash) and modelled as operations per profile
  crates/iohr-codegen/ one Target per language; rust/ renders models (typify) and the
                      surface (minijinja templates in src/rust/templates/)
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
- Sign-in: PKCE S256 only, a fresh `state` compared in constant time, the issuer checked
  in discovery and in the ID token, every endpoint and link on the issuer's origin, the
  loopback listener on `127.0.0.1` (or `[::1]`) only, one `GET /callback`, 5 minutes.
  Only `Authorization<Granted>` becomes a stored `Session`; keep it that way.
- A refused refresh ends the session only after re-reading the store: another `iohr`
  process may have rotated the token.
- Only the API host: paths are checked to stay on it, base URLs are HTTPS (loopback
  excepted), redirects are not followed. No telemetry, no update check.
- The HTTP client is the Rust SDK (`inorbithr`, by path): retries, the error envelope,
  the user agent and the body cap are its; the command line adds the credential, the
  `--verbose` hook and the exit codes. Do not grow a transport layer here again.
- `--verbose` prints method, path, status, time and request id; never a header, a query
  value or a body. The `verbose_output_never_shows_the_token` test covers every command:
  add each new command to it.
- `sdk generate` and `sdk check` resolve each profile through `context::session_for`:
  `IOHR_TOKEN_<PROFILE>` stands in for a named profile (CI), never the bare `IOHR_TOKEN`.
  A person profile sends `?account=` and the answer's stamp must name that account.
- Output: results on stdout (tables, or JSON with `--json`), notes on stderr, so pipes
  get only data. `token create` prints the new token alone on stdout.
- Dependencies: the table in `README.md`. A new one needs a reason there and in the PR.
- The generator's output is deterministic and golden-tested (`iohr-codegen/tests/golden/`,
  `IOHR_UPDATE_GOLDEN=1` rewrites, review the diff); `IOHR_TEST_COMPILE=1` builds a
  generated surface against `rust/` and proves the wrong profile does not compile.
  `rust/src/generated/` is this generator's output; never edit it, run `mise run rust:gen`.
- `unwrap`/`expect` only in tests; `unsafe` is forbidden; clippy pedantic stays on.
- Extensions (ADR 0012, SR-25 to SR-28): the registry is reached only from `ext install`,
  `upgrade` and `sync` (`ext::oci`, `GET` only, bounded sizes, digests checked). Nothing
  from a registry is used before `ext::trust::verify` passes, and no option skips it.
  `ext::layer` takes the entrypoint only, as a regular file. An extension gets tokens
  only through `ext::socket` for its manifest's scopes; never pass it `IOHR_TOKEN*`, the
  store or a refresh token. `tests/ext.rs` runs a registry on loopback: add a case for
  every new refusal.
- Commit scope `cli`.
