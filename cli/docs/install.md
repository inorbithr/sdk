# Install and upgrade

How to install `iohr`, keep it current and check that what you installed is what InOrbit
released. Back to the [iohr overview](../README.md).

## Homebrew (macOS, Linux)

```sh
brew trust inorbithr/tap            # once per machine: Homebrew refuses untrusted taps
brew install inorbithr/tap/iohr
```

The tap can lag a release by the time its formula pull request takes to merge.

## Install script (Linux, macOS, Windows)

```sh
curl -fsSL https://packages.inorbit.hr/install.sh | sh             # Linux, macOS
powershell -c "irm https://packages.inorbit.hr/install.ps1 | iex"  # Windows
```

The scripts ([install.sh](../install/install.sh), [install.ps1](../install/install.ps1))
are short enough to read before you run them. Each one:

1. downloads the archive for your machine from the GitHub release;
2. refuses it unless its SHA-256 matches the release's `SHA256SUMS`, and, when the GitHub
   CLI is installed and signed in, unless its build attestation checks;
3. copies one binary into your user directory, with no sudo or administrator rights:
   `~/.local/bin` on Linux and macOS, `%LOCALAPPDATA%\Programs\iohr` on Windows (which it
   adds to your user `PATH`, saying so). `install.sh` changes no shell profile.

`IOHR_VERSION=0.1.0-alpha.38` installs one version instead of the newest, and
`IOHR_INSTALL_DIR` puts the binary somewhere else.

## APT (Debian, Ubuntu)

A signed APT repository for `amd64` and `arm64`:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://packages.inorbit.hr/iohr.gpg | sudo tee /etc/apt/keyrings/iohr.gpg >/dev/null
echo "deb [signed-by=/etc/apt/keyrings/iohr.gpg] https://packages.inorbit.hr/apt stable main" \
  | sudo tee /etc/apt/sources.list.d/iohr.list
sudo apt update && sudo apt install iohr
```

The key's fingerprints, and how to check them, are in
[verifying releases](../../docs/security/verifying-releases.md).

## From source

From a clone of this repository, with Rust 1.94 or newer:

```sh
cargo install --locked --path cli/crates/iohr
```

`iohr` is not published on crates.io.

## Not available yet

winget: its manifests are built with every release and will be submitted with the first
stable one ([releasing](../../docs/releasing.md)). Until then, use the PowerShell script.

## Upgrade

```sh
iohr upgrade            # update the way this copy was installed
iohr upgrade --check    # only say whether a newer release exists and what would run
```

`iohr upgrade` needs no sign-in and follows how `iohr` was installed:

| Installed with | What `iohr upgrade` does |
|---|---|
| Homebrew | Runs `brew upgrade inorbithr/tap/iohr`, and tells you to run `brew trust inorbithr/tap` when Homebrew refuses the tap |
| The install script | Downloads the new archive, checks it against `SHA256SUMS` and its GitHub build attestation, and replaces the binary in place. The attestation is checked inside `iohr`, so the GitHub CLI is not needed; a release without one is refused |
| APT, or anything else | Prints the command to run |

A machine that runs `iohr runner run` can upgrade itself; see
[Runner, staying current](runner.md#staying-current).

## Check a release by hand

Every release has archives for macOS, Linux and Windows on x86_64 and aarch64, `.deb` and
`.msi` packages, a `SHA256SUMS` file, a CycloneDX SBOM per archive (`.cdx.json`) and GitHub
build attestations. How to check a
download, and the APT key, yourself: [verifying releases](../../docs/security/verifying-releases.md).
Release notes are in [CHANGELOG.md](../CHANGELOG.md).
