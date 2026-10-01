# Releasing

Releases are automated by release-please ([ADR 0003](adr/0003-versioning-and-releases.md)).
A maintainer's job is to merge the release PR and approve the publish.

## How a release happens

1. Conventional Commits land on `main` (`feat(go): ...`, `fix(py): ...`).
2. The `release-please` workflow opens or updates one PR per package
   ("chore(main): release go 0.2.0") with the version bump and the changelog.
3. A maintainer merges it. release-please tags the commit (`go/v0.2.0`) and creates the
   GitHub release.
4. The same `release` workflow run then starts the publish job for that package. It waits
   for approval in its environment, builds, and publishes with OIDC; npm and PyPI add
   provenance and attestations themselves, the Go job attests the source archive and
   records an SBOM.

| Package | Tag | Publish job | Environment |
|---|---|---|---|
| Go | `go/vX.Y.Z` | warms `proxy.golang.org`, attests the source archive | none |
| Rust | `rust/vX.Y.Z` | `cargo publish` via `crates-io-auth-action` | `release-crates` |
| TypeScript | `typescript/vX.Y.Z` | `npm publish` (provenance automatic) and `jsr publish` | `release-npm` |
| Python | `python/vX.Y.Z` | `uv build`, `pypa/gh-action-pypi-publish` | `release-pypi` |

## One-time setup (before the first release)

- [ ] Create the npm organisation `inorbithr`; publish `@inorbithr/sdk@0.0.0` by hand to
      claim it (npm cannot trust-publish a package that does not exist yet).
- [ ] Publish `inorbithr 0.0.0` to crates.io by hand, then link the repository under the
      crate's trusted publishing settings (workflow `release.yml`, environment
      `release-crates`).
- [ ] On PyPI, add a pending trusted publisher for `inorbithr` (workflow `release.yml`,
      environment `release-pypi`); no manual upload needed.
- [ ] On npm, add the trusted publisher for `@inorbithr/sdk` (workflow `release.yml`,
      environment `release-npm`).
- [x] Create the three environments, deployable from `main` only (2026-10-01).
- [ ] Add a required reviewer to each environment. GitHub allows required reviewers on a
      private repository only on Enterprise; on the Team plan this waits until the
      repository is public. Until then a publish starts as soon as release-please tags.
- [ ] Create a GitHub App for release-please (contents and pull-requests write) and store
      `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`; the default token cannot trigger
      the CI run a release PR needs.
- [x] Ruleset `main`: pull request with squash merge only, `ci-ok` required, resolved
      threads, linear history, no force push or deletion (2026-10-01). Required approvals
      are 0 while there is one maintainer.
- [x] Ruleset `release tags are immutable`: tags `*/v*` cannot be moved or deleted.
- [ ] Restrict creating `*/v*` tags to the release app, once the app exists.

## Fixing a bad release

- npm, PyPI, crates.io: publish a patch. Deprecate (npm), yank (PyPI, crates.io) the bad
  version; never unpublish.
- Go: add a `retract` directive for the bad version in `go/go.mod` and release a patch.
  Never delete or move a Go tag; the checksum database has already recorded it.
