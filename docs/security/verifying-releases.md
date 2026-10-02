# Verifying releases

How to check that the package you installed is the one this repository built. Every
release is built and published by the `release` workflow in `inorbithr/sdk` from a
commit on `main`; the checks below confirm that.

Status: the commands apply from the first release (0.1.0). Items marked **planned** are
being added to the release workflow ([requirements.md](requirements.md) SR-23).

## npm: `@inorbithr/sdk`

```sh
npm install @inorbithr/sdk
npm audit signatures
```

`npm audit signatures` checks the registry signatures and the provenance attestation of
every installed package. The package page on npmjs.com shows "Built and signed on GitHub
Actions" with a link to the exact workflow run and commit.

## PyPI: `inorbithr`

PyPI stores a PEP 740 attestation for each file, shown on the file's page under
"Provenance". To verify a downloaded file:

```sh
pip download inorbithr==X.Y.Z --no-deps -d dist/
pipx run pypi-attestations verify pypi \
  --repository https://github.com/inorbithr/sdk dist/inorbithr-X.Y.Z-py3-none-any.whl
```

## crates.io: `inorbithr`

crates.io does not sign crates yet. The release workflow attests the exact `.crate` file
with GitHub artifact attestations (**planned**):

```sh
curl -sSLo inorbithr-X.Y.Z.crate https://static.crates.io/crates/inorbithr/inorbithr-X.Y.Z.crate
gh attestation verify inorbithr-X.Y.Z.crate --repo inorbithr/sdk
```

Cargo also checks every download against the checksum in the crates.io index.

## Go: `github.com/inorbithr/sdk/go`

The Go toolchain checks every module against the public checksum database
(`sum.golang.org`) on download; a module that differs from what the database recorded
fails to build. The release workflow also attests the source archive attached to the
GitHub release:

```sh
gh release download go/vX.Y.Z --repo inorbithr/sdk --pattern go-source.tar.gz
gh attestation verify go-source.tar.gz --repo inorbithr/sdk
```

## JSR: `@inorbithr/sdk`

JSR records provenance for packages published from GitHub Actions and shows it on the
package page with a link to the transparency log entry.

## Command line: `iohr`

Every file on an `iohr/vX.Y.Z` release (archives, `.deb`, `.msi`, `SHA256SUMS`) has a
build provenance attestation, and each target's CycloneDX SBOM is attested against that
target's files:

```sh
gh release download iohr/vX.Y.Z --repo inorbithr/sdk --pattern 'iohr-*-x86_64-unknown-linux-musl.tar.gz' --pattern SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify iohr-X.Y.Z-x86_64-unknown-linux-musl.tar.gz --repo inorbithr/sdk
```

The installers run the checksum check themselves and the attestation check when `gh` is
installed. Releases before 1.0 are marked pre-release.

The APT repository at `https://packages.inorbit.hr/apt` is signed with a key used for
nothing else ([`cli/install/iohr.gpg`](../../cli/install/iohr.gpg), also served at
`https://packages.inorbit.hr/iohr.gpg`). Check what you fetched with
`gpg --show-keys /etc/apt/keyrings/iohr.gpg`:

| Key | Fingerprint |
|---|---|
| Primary, `InOrbit packages <packages@inorbit.hr>` (certifies only, kept offline) | `5FF6 7AF3 D50A 6B06 FFE6  EA8A FFA5 11CF 585B F28D` |
| Signing subkey (signs the repository; expires 2028-10-01) | `1725 799F E6C8 0810 9671  0D42 E7B1 639B 4435 8981` |

A new signing subkey is announced in the release notes a release before it takes over,
with both in the keyring during the overlap.

## SBOMs

Each GitHub release carries a CycloneDX SBOM per package (**planned**), attested so you
can check it came from the same workflow:

```sh
gh release download typescript/vX.Y.Z --repo inorbithr/sdk --pattern '*.cdx.json'
gh attestation verify <artifact> --repo inorbithr/sdk \
  --predicate-type https://cyclonedx.org/bom
```

`<artifact>` is the package file the SBOM describes (for example the npm `.tgz`).

## The GitHub release itself

Releases are immutable once published (**planned**): their tag and assets cannot be
changed, and GitHub signs a release attestation.

```sh
gh release verify typescript/vX.Y.Z --repo inorbithr/sdk
gh release verify-asset typescript/vX.Y.Z ./inorbithr-sdk-X.Y.Z.tgz --repo inorbithr/sdk
```

## If a check fails

Do not use the package. Report it privately as described in
[SECURITY.md](../../SECURITY.md).
