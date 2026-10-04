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
| Rust | `rust/vX.Y.Z` | `cargo publish` via `crates-io-auth-action` | `release-crates` |
| TypeScript | `typescript/vX.Y.Z` | `npm stage publish` (provenance automatic), then a maintainer approves it; `jsr publish` | `release-npm` |
| Python | `python/vX.Y.Z` | `uv build`, `pypa/gh-action-pypi-publish` | `release-pypi` |
| C# | `csharp/vX.Y.Z` | none yet (M4b): release-please only versions it, the `.csproj` `Version` | none |
| Java | `java/vX.Y.Z` | none yet (M4b): release-please only versions it, `pom.xml` and `Client.SDK_VERSION` | none |
| Command line | `iohr/vX.Y.Z` | builds six targets (`cli-build.yml`), attests every file, attaches archives, `.deb`, `.msi`, SBOMs and `SHA256SUMS` to the release; then runs the installers against it and opens the formula PR on `inorbithr/homebrew-tap` (release App token limited to the tap) ([ADR 0010](adr/0010-releasing-the-command-line.md)) | `release-cli` |

C# and Java follow every change and carry the same version as the other four, so their
registry release is one step once M4b lands: `IsPackable` stays `false` and the POM has no
`distributionManagement` until then, and no job uploads them. A version the four take
together is set once in `release-please-config.json` (`release-as` on each package,
removed in the next pull request after the release); a `Release-As:` footer would also
move every other component the commit touches, the command line included.

## The command line's channels

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
