# Releasing

Releases are automated by release-please ([ADR 0003](adr/0003-versioning-and-releases.md)).
A maintainer's job is to merge the release PR and approve the publish.

## Launch checklist (when the repository goes public)

The repository went public on 2026-10-01. These items waited for that.

- [x] JSR: link `inorbithr/sdk` on `@inorbithr/sdk` (Settings, GitHub repository). Done (checked 2026-10-03). The
      scope and package exist since 2026-10-01; the release job publishes to JSR only
      once the repository is public.
- [x] Environments `release-npm`, `release-pypi`, `release-crates`, `release-go` (2026-10-03): add a required
      reviewer (needs a public repository on the Team plan). Done 2026-10-01.
- [x] Turn on private vulnerability reporting (Settings, Code security). Done 2026-10-01.
- [x] Fork pull requests from any outside contributor wait for approval before their
      workflows run. Done 2026-10-01.
- [ ] Check that `security.yml` (CodeQL, zizmor upload, dependency review) and
      `scorecard.yml` run; they skip themselves while the repository is private.
- [x] README: drop "design phase" once a release exists. Done 2026-10-03.

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
| Go | `go/vX.Y.Z` | warms `proxy.golang.org`, attests the source archive | `release-go` |
| Go OpenTelemetry | `go/otel/vX.Y.Z` | none: release-please tags it, and the module proxy fetches the tag on first request. It requires `go/vX.Y.Z`, so merge its release PR after the `go` one is tagged | none |
| Rust | `rust/vX.Y.Z` | `cargo publish` via `crates-io-auth-action` | `release-crates` |
| TypeScript | `typescript/vX.Y.Z` | `npm stage publish` (provenance automatic), then a maintainer approves it; `jsr publish` | `release-npm` |
| Python | `python/vX.Y.Z` | `uv build`, `pypa/gh-action-pypi-publish` | `release-pypi` |
| C# | `csharp/vX.Y.Z` | none yet (M4b): release-please only versions it, the `.csproj` `Version` | none |
| Java | `java/vX.Y.Z` | none yet (M4b): release-please only versions it, `pom.xml` and `Client.SDK_VERSION` | none |
| Command line | `iohr/vX.Y.Z` | builds six targets (`cli-build.yml`), attests every file, attaches archives, `.deb`, `.msi`, SBOMs and `SHA256SUMS` to the release; then runs the installers against it and opens the formula PR on `inorbithr/homebrew-tap` (release App token limited to the tap) ([ADR 0010](adr/0010-releasing-the-command-line.md)) | `release-cli` |

C# and Java follow every change and carry the same version as the other four, so their
registry release is one step once M4b lands: `IsPackable` stays `false` and the POM has no
`distributionManagement` until then, and no job uploads them.

## Versions

Every component follows SemVer, and release-please derives each version from the
Conventional Commits that touched it; nobody sets one by hand (no `release-as`, no
`Release-As:` footer).

- **Before 1.0** (`bump-minor-pre-major`, `bump-patch-for-minor-pre-major`): a breaking
  change bumps the minor (0.2.0 to 0.3.0), a `feat` or `fix` the patch (0.2.0 to 0.2.1).
- **From 1.0**, which comes when the API is stable: breaking is major, `feat` minor, `fix`
  patch.
- A breaking change says so in its commit, `feat(go)!: ...` or a `BREAKING CHANGE:` footer,
  and the squash-merge subject and body keep it. `rust:api` and `py:api` accept a break
  only when such a commit since the last tag declares it (`tools/declared-break.py`): Rust
  is then checked as a breaking release, Python's changes are listed without failing.
- The command line is the same rule on its `alpha` pre-release line (`0.1.0-alpha.N`)
  until its first stable release.
- Before merging a release PR, check its title and the manifest line carry the version
  the commits call for.

## The command line's channels## The command line's channels

| Channel | How a release reaches it | Credential |
|---|---|---|
| GitHub release | `iohr` job: archives, `.deb`, `.msi`, SBOMs, `SHA256SUMS`, attestations | `GITHUB_TOKEN`, environment `release-cli` |
| APT (`https://packages.inorbit.hr/apt`) | `iohr-apt`: reprepro, signed by the repository subkey, uploaded to R2, then installed in Debian | `APT_SIGNING_*`, `R2_*` (repository secrets) |
| Installers (`https://packages.inorbit.hr/install.sh`, `.ps1`) | uploaded by `iohr-apt`; `iohr-install` runs them against the release | as above |
| Homebrew (`inorbithr/homebrew-tap`) | `iohr-tap`: a formula PR the tap's CI installs and tests; a maintainer merges it | release GitHub App, token limited to the tap |
| winget | `iohr-winget`: manifests generated and schema-checked, kept as an artifact; submitted to `microsoft/winget-pkgs` by hand from the first stable release | a maintainer's GitHub account with a fork of winget-pkgs |

## One-time setup (before the first release)

- [x] Create the npm organisation `inorbithr`; publish `@inorbithr/sdk@0.0.0` by hand to
      claim it (npm cannot trust-publish a package that does not exist yet). Done
      2026-10-01.
- [x] Publish `inorbithr 0.0.0` to crates.io by hand, then link the repository under the
      crate's trusted publishing settings (workflow `release.yml`, environment
      `release-crates`), trusted publishing only. Done 2026-10-01; owner `0x19`.
- [x] Publish `inorbithr 0.0.0` to PyPI by hand to hold the name (a pending publisher
      does not reserve it), then add the trusted publisher for `inorbithr/sdk` (workflow
      `release.yml`, environment `release-pypi`). Done 2026-10-01.
- [x] On npm, add the trusted publisher for `@inorbithr/sdk` (workflow `release.yml`,
      environment `release-npm`), stage-only, and require 2FA with no bypass tokens.
      Done 2026-10-01.
- [x] Create the three environments, deployable from `main` only (2026-10-01).
- [x] Add a required reviewer to each environment (`release-crates`, `release-npm`,
      `release-pypi`, `release-go`, `release-cli`, `release-nuget`): the maintainer, `main`
      only. Done 2026-10-03.
- [x] Create a GitHub App for release-please (contents and pull-requests write) and store
      `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`; the default token cannot trigger
      the CI run a release PR needs. Done (`inorbithr-release`; the key is also an
      organisation secret shared with inorbithr/dataplane).
- [ ] C# on NuGet: the trusted-publishing policy (owner organisation InOrbit, repository
      `inorbithr/sdk`, workflow `release.yml`, environment `release-nuget`), the `InOrbit.`
      ID prefix (mail to account@nuget.org), then lift `IsPackable` and add the publish job.
- [ ] Java on Maven Central: a Central user token and a dedicated signing key as secrets,
      then the publish job. The namespace `hr.inorbit` is verified (2026-10-03).

## Dependencies

Every dependency, toolchain and action stays on its latest version, majors included.
A major update adapts our code in the same pull request; it is never skipped or pinned
back.

- **The one exception is a minimum runtime.** A type package or toolchain that defines the
  oldest runtime a published library supports stays at that minimum (ADR 0006): today
  `@types/node` follows Node 22 (`engines`), `go` in `go.mod` stays 1.26, `rust-version`
  1.94, `requires-python` 3.11, Java `release` 17, C# `net8.0`. Raising one is a breaking
  change for users and is decided on its own, never as part of an update. `@types/node`
  majors are the only `ignore` rule in `.github/dependabot.yml`; an update that would
  raise another minimum (a crate needing a newer Rust, a module needing a newer Go) fails
  the oldest-version CI cell and waits.
- **Dependabot** watches every manifest: Go, Rust (with `cli/` and the examples, so the
  lockfiles that build against `rust/` move together), npm, uv, Maven, NuGet and the
  workflows' actions. It groups updates into one pull request per ecosystem for patch
  and minor and one for majors, weekly on Mondays (actions daily), never younger than the
  7-day cooldown (ADR 0005). Security updates come at once.
- **Auto-merge** (`.github/workflows/dependabot-automerge.yml`): a Dependabot pull
  request whose updates are all patch, or minor at 1.0 or later, gets `gh pr merge --auto
  --squash`, so it lands once `ci-ok` passes. Majors and 0.x minors (breaking under
  SemVer) wait for a maintainer, who adapts the code on the branch. The job runs only for
  pull requests Dependabot opened, with a token limited to that merge. A merge made by
  that token starts no workflows, so release-please picks it up on the next push to
  `main`.
- **Toolchains and tools in `mise.toml`** are outside Dependabot: `mise outdated --bump`
  lists them, and a `chore(deps)` pull request moves them, with the CI matrix's newest
  cells (`ci.yml`) and the release build image (`cli/release/dist.sh`) in step.
- **Dependency changes are `build(<lang>)`, `ci` or `chore(deps)`**, so they release as
  nothing or a patch. An update that changes our public API (a re-exported type, a
  raised minimum) is breaking and is decided before it lands.
- **Known vulnerabilities**: `mise run audit` (OSV over every lockfile, `govulncheck`,
  `cargo deny check advisories`, `pnpm audit`, `pip-audit`) runs in CI; Dependabot
  alerts are fixed by updating, never by an ignore.

## Lock files in a release

release-please updates every lock file that records a package's own version, so the
release PR builds with `--locked`: `rust/Cargo.lock`, `cli/Cargo.lock` (the `iohr*` crates)
and `python/uv.lock` (`extra-files` in `release-please-config.json`).

## A Rust release and the command line

The command line depends on the `inorbithr` crate by path, and its `cli/Cargo.lock` pins
the crate's version. A Rust release PR that changes the version leaves that lock behind,
and `cli:check` (which builds with `--locked`) fails on it: add a `chore(cli)` commit to the
release PR with `cargo update -p inorbithr --manifest-path cli/Cargo.toml`.
- [x] Ruleset `main`: pull request with squash merge only, `ci-ok` required, resolved
      threads, linear history, no force push or deletion (2026-10-01). Required approvals
      are 0 while there is one maintainer.
- [x] Ruleset `release tags are immutable`: tags `*/v*` cannot be moved or deleted.
- [ ] Restrict creating `*/v*` tags to the release app, once the app exists.

## Approving an npm release

The npm trusted publisher may only **stage** a version. After the publish job runs, the
version waits on npmjs.com, invisible to installs, until a maintainer approves it:

- web: npmjs.com, `@inorbithr/sdk`, **Staged Packages**, **Approve** (asks for 2FA);
- CLI: `npm stage approve <stage-id>` (asks for 2FA).

The CI token cannot approve, so a compromised workflow cannot ship a version on its own.

## Fixing a bad release

- npm, PyPI, crates.io: publish a patch. Deprecate (npm), yank (PyPI, crates.io) the bad
  version; never unpublish.
- Go: add a `retract` directive for the bad version in `go/go.mod` and release a patch.
  Never delete or move a Go tag; the checksum database has already recorded it.

## When a tagged release did not reach its registry

The release workflow can publish an existing tag again by hand, through the same trusted
publisher and approval environment: Actions, `release`, "Run workflow", with
`python_tag` (for example `python/v0.1.0`) or `typescript_tag` (for example
`typescript/v0.1.0`), one per run. Python publishes the release's attached, attested
files; TypeScript rebuilds from the tag and stages on npm, where a maintainer approves it
with 2FA. release-please does not run on a manual run, so nothing new is tagged.
When the version is already staged on npm and a later step failed (the SBOM, the
attestation, the release files or JSR), add `typescript_npm_staged`: the run builds and
packs the same tag again but does not stage it a second time.
