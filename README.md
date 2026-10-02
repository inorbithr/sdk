# InOrbit SDK

The `iohr` command line and the official client libraries for the
[InOrbit API](https://docs.inorbit.hr) in Go, Rust, TypeScript and Python.

The repository holds two things: the `iohr` command line, released as a pre-release,
and the four client libraries, which are not released yet. Status on 2026-10-02:

| Part | Package | Status |
|---|---|---|
| Command line | `iohr` (APT, Homebrew, installers) | **0.1.0-alpha.2, pre-release** |
| Go | `github.com/inorbithr/sdk/go`, Go 1.26 | not released |
| Rust | [`inorbithr`](https://crates.io/crates/inorbithr), Rust 1.94 | not released |
| TypeScript | [`@inorbithr/sdk`](https://www.npmjs.com/package/@inorbithr/sdk), Node 22.12, Bun, Deno, browsers | not released |
| Python | [`inorbithr`](https://pypi.org/project/inorbithr/), Python 3.11 | not released |

The contract the libraries are built against is synced from the platform into
[`spec/`](spec/) (23 operations on 2026-10-02), and the replay server in
[`conformance/`](conformance/) runs the shared behaviour cases. [docs/roadmap.md](docs/roadmap.md)
has the milestones.

## The command line

```sh
# macOS and Linux, Homebrew
brew install inorbithr/tap/iohr

# Debian and Ubuntu: the repository is signed by the key with fingerprint
# 5FF6 7AF3 D50A 6B06 FFE6  EA8A FFA5 11CF 585B F28D
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://packages.inorbit.hr/iohr.gpg | sudo tee /etc/apt/keyrings/iohr.gpg >/dev/null
printf 'Types: deb\nURIs: https://packages.inorbit.hr/apt\nSuites: stable\nComponents: main\nSigned-By: /etc/apt/keyrings/iohr.gpg\n' \
  | sudo tee /etc/apt/sources.list.d/iohr.sources >/dev/null
sudo apt update && sudo apt install iohr

# Any Linux or macOS, for your user only, no sudo
curl -fsSL https://packages.inorbit.hr/install.sh | sh
```

On Windows, `install.ps1` from the same host installs it for your user. Then:

```sh
iohr login        # signs you in through your browser, or with a device code over SSH
iohr whoami
iohr api GET /v1/me
```

`iohr` keeps several accounts as profiles, creates and revokes API tokens, and talks
only to `api.inorbit.hr` and `auth.inorbit.hr`, with no telemetry. Releases before 1.0
are pre-releases. Every release carries checksums, an SBOM and build provenance you can
check with `gh attestation verify`. [cli/README.md](cli/README.md) has every command.

## What the libraries will do

- Authenticate with an API token, or exchange an API key for a short-lived token, cache
  it and refresh it, so you never handle the exchange yourself.
- Return typed results and one error type per language, carrying the API's error `code`
  and `details`.
- Retry what is safe to retry (`429`, `503`, `504`, connection failures), honouring
  `Retry-After`.
- Behave the same in every language: one set of [conformance cases](conformance/) runs
  against all four.

A token reaches only the routes of the scopes it holds: identity, the account, its usage
and units, the radar, events and webhooks. Generated SDKs cut to what one account can
call, in Java and C# as well as the four above, come with `iohr sdk generate`
([RFC 0020](https://inorbit.hr/lab/rfc/0020-sdks-cut-to-the-caller/)). See
[docs/design.md](docs/design.md) for the library design.

## Repository layout

| Path | Contents |
|---|---|
| [`cli/`](cli/) | The `iohr` command line, its release scripts and packaging |
| [`spec/`](spec/) | The API contract, synced from the platform |
| [`conformance/`](conformance/) | Shared behaviour cases and the replay server every SDK runs them against |
| [`go/`](go/), [`rust/`](rust/), [`typescript/`](typescript/), [`python/`](python/) | One package per language |
| [`examples/`](examples/) | Small programs that CI compiles |
| [`docs/`](docs/) | Design, decisions (ADRs), releasing, style |

## Security

- Report vulnerabilities privately: [SECURITY.md](SECURITY.md).
- Supported versions: [SUPPORT.md](SUPPORT.md). Before 1.0 only the latest release gets
  fixes; the security support period starts with 1.0.
- Requirements, threat model, controls and how to verify releases:
  [docs/security/](docs/security/).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## License

[Apache License 2.0](LICENSE).
