# 0003. Independent versions, release-please, trusted publishing

Status: accepted, 2026-10-01

## Context

The four packages ship on four registries with four sets of conventions. A Go major
version changes the import path, so it is expensive; a Python-only bug should not bump
the Rust crate.

## Decision

- **Independent SemVer per package**, starting at `0.1.0`. The API version the package
  was built against is recorded in each package (`spec/SOURCE`) and sent in the
  user agent.
- **release-please in manifest mode** (`release-please-config.json`), one release PR per
  package, driven by Conventional Commits scoped to the language. Tags:
  `go/vX.Y.Z`, `rust/vX.Y.Z`, `typescript/vX.Y.Z`, `python/vX.Y.Z`.
- **Trusted publishing (OIDC), no long-lived tokens**: npm (with provenance), PyPI
  (with PEP 740 attestations), crates.io (`rust-lang/crates-io-auth-action`). Go
  publishes by tag; the workflow warms `proxy.golang.org` afterwards.
- Each publish job runs in a GitHub environment (`release-npm`, `release-pypi`,
  `release-crates`) that requires a maintainer's approval.
- The first version on npm and crates.io is published by hand once to claim the name;
  trusted publishing is configured right after (`docs/releasing.md`).
- Stay on v0 until the public surface covers more than the account endpoints; go to v1
  deliberately, never by accident.

## Consequences

- A tag on a Go module is permanent: the Go checksum database keeps it forever. A bad Go
  release is fixed with a `retract` directive and a new patch, never by moving the tag.
- Changelogs and versions are written by release-please only.

## Sources

- https://github.com/googleapis/release-please/blob/main/docs/manifest-releaser.md
- https://docs.npmjs.com/trusted-publishers
- https://docs.pypi.org/trusted-publishers/
- https://blog.rust-lang.org/2025/07/11/crates-io-development-update-2025-07
- https://go.dev/ref/mod#go-mod-file-retract
