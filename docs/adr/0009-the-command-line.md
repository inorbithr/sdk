# 0009. The command line lives here, in its own workspace

Status: accepted, 2026-10-02. Scopes SR-12 to the libraries and adds SR-24. Point 5 is
superseded by [ADR 0012](0012-extensions.md) (2026-10-03): the extension registry is the
one exception.

## Context

The platform decided on 2026-10-02 to ship a command line, `iohr`, that signs a person
in, keeps several accounts side by side as profiles, calls the API, and later generates
SDKs cut to what a credential may call (platform RFC 0019 and RFC 0020, both decided).
It is public code under this repository's licence, and it needs a home, a release path
and security rules.

Three facts shape where it goes:

- Each top-level language directory here is one package with its own release-please
  component (ADR 0003). `rust/` is the `inorbithr` crate; a workspace root there would
  make every command-line change look like an SDK change to release-please.
- The Rust SDK does not exist yet (roadmap M3), and every public SDK change lands in all
  four languages at once (`AGENTS.md`). The command line cannot wait for M3, and it must
  not add Rust-only SDK surface to get there.
- SR-12 says the SDK never writes a token to disk. A command line that signs a person
  in has to keep a refresh token between runs, or the person signs in on every command.

## Decision

1. **`cli/` is a Cargo workspace of its own**, beside `rust/`: `cli/crates/iohr` (the
   binary, with a library target the binary and the fuzz targets share) and
   `cli/crates/iohr-auth` (credentials, profiles, the credential store). The generator
   crates of RFC 0020 join it later. Edition 2024, MSRV 1.94 like the SDK. Crates are
   `publish = false` until the release work of RFC 0021 adds them to release-please.
2. **Until the Rust SDK exists, the command line carries its own small HTTP client**,
   private to the binary and written to `docs/design.md` sections 4 to 6 (errors,
   retries, user agent, redaction). When M3 ships, the command line depends on
   `inorbithr` by path and the private client goes; its `Credential` trait becomes the
   SDK's token provider.
3. **SR-12 applies to the libraries.** A program a person runs on their own machine may
   keep a credential between runs under SR-24.
4. **SR-24: command-line credentials.** Secrets go to the operating system's credential
   store only (macOS Keychain, Windows Credential Manager, the Secret Service on Linux),
   one entry per profile and account. The config file never holds a secret. A plain file
   store exists only behind `--insecure-storage`, created with mode 0600 in a 0700
   directory. A token given in `IOHR_TOKEN` is used in memory and nothing is written.
   A token is never accepted as a command-line argument, where it would reach shell
   history and the process list.
5. **No update check.** (Superseded by ADR 0012, which adds the extension registry
   for `iohr ext install`, `upgrade` and `sync` only.) SR-16 holds for the command line: it talks to the API host and
   the sign-in host and nothing else.
6. Commit and pull-request scope `cli`.

## Consequences

- `mise run cli:check` and a `cli` CI job (Linux on pull requests, macOS and Windows on
  `main`), with the same pins, cache and MSRV cell as the Rust job.
- The credential store is tested against a real store on each operating system in CI;
  where a CI image has none, the test is skipped with the reason printed, never faked.
- The private client duplicates some of what the Rust SDK will hold. That is the price
  of not shipping a Rust-only SDK surface early; the duplication ends with M3.
- Releasing the command line (packages, installers, signing) is a later ADR, with RFC
  0021.

## Sources

- Platform RFC 0019, "The command line" (inorbit.hr/lab, decided 2026-10-02).
- keyring-core 1.0 and its per-platform stores, open-source-cooperative/keyring-rs wiki
  "Keyring", 2026.
- GitHub CLI multi-account storage, cli/cli#12885 (an entry keyed by host alone returned
  the wrong account's token).
- Command Line Interface Guidelines, clig.dev, "Arguments and flags" (secrets do not go
  in flags).
