# 0010. Releasing the command line

Status: accepted, 2026-10-02. Implements platform RFC 0021 for `iohr`.

## Context

`iohr` (ADR 0009) needs signed, attested binaries for Linux, macOS and Windows, a Debian
package, a Windows installer and the channels RFC 0021 decided: our APT repository, a
Homebrew tap, winget and short installers. This repository already releases each package
with release-please and attests what it publishes (ADR 0003, ADR 0005).

## Decision

1. **A workflow of our own, not dist.** dist generates and regenerates its own release
   workflow and expects to create the GitHub release; here release-please owns tags and
   releases, every job runs a `mise` task, permissions are per job and zizmor checks
   everything (ADR 0005). Keeping dist's output in line would mean editing a generated
   file dist then flags as dirty, and the two custom jobs (the `.deb`, the installer)
   would remain. `cli-build.yml` builds; `release.yml` attests and publishes.
2. **Targets**, each built natively on its own runner: x86-64 and arm64 Linux (statically
   linked against musl in a pinned `rust:alpine` image, so one binary runs on every
   distribution), Apple silicon and Intel macOS, x86-64 and arm64 Windows.
3. **Artifacts per release**: an archive per target (`.tar.gz`, `.zip` on Windows) with the
   binary, licence, notice, README and shell completions; a `.deb` per Linux architecture
   (cargo-deb); an `.msi` per Windows architecture; `SHA256SUMS`; a CycloneDX SBOM per
   target; a build provenance attestation for every file and an SBOM attestation binding
   each target's SBOM to its files.
4. **The installer uses WiX 5.0.2**, the last WiX release under its original MS-RL licence.
   WiX 6 and later add a maintenance-fee EULA for revenue-making organisations; msitools'
   `wixl` cannot add a program to `PATH`. Moving past WiX 5 is a decision of its own.
5. **Versions**: release-please component `iohr` (path `cli`, tag `iohr/vX.Y.Z`), the
   `simple` strategy with the workspace version and the two `Cargo.lock` entries updated
   by TOML path, because release-please's Rust workspace support assumes a workspace at
   the repository root and replaces inherited versions. Pre-1.0 versions are alpha
   pre-releases and every GitHub release before 1.0 is marked pre-release (RFC 0021, the
   Cyber Resilience Act's reporting duty applies to them too).
6. **One approval**: the publish job runs in the `release-cli` environment (main only, a
   required reviewer). The channel jobs (APT, Homebrew, winget) follow it without a second
   gate, and each does nothing until its credentials exist.
7. **crates.io later**: `cargo install iohr` needs `iohr` and `iohr-auth` published, and
   trusted publishing needs each name claimed by hand first; the crates stay
   `publish = false` until then.

## Consequences

- A pull request that changes the build runs the whole six-target matrix (`cli-build.yml`),
  so a release never discovers a broken target.
- Linux release builds pull Alpine packages (musl headers, cmake, a C compiler) at build
  time; the image is pinned by digest, the packages are not.
- macOS binaries are unsigned until the Apple Developer ID exists; Homebrew and the shell
  installer do not quarantine them. Windows binaries are unsigned until an Authenticode
  certificate exists; winget and the PowerShell installer avoid SmartScreen's prompt.

## Sources

- Platform RFC 0021, "Packages" (inorbit.hr/lab, decided 2026-10-02).
- dist configuration reference, axodotdev.github.io/cargo-dist/book/reference/config.html.
- release-please `cargo-workspace` and a workspace outside the root,
  googleapis/release-please#2589.
- WiX Open Source Maintenance Fee, docs.firegiant.com/wix/osmf/.
- cargo-deb, github.com/kornelski/cargo-deb.
