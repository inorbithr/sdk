<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/logo-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset=".github/assets/logo-light.svg">
    <img alt="InOrbit SDK logo: a terminal prompt with an amber cursor" src=".github/assets/logo-light.svg" width="88" height="88">
  </picture>
</p>

<h1 align="center">sdk</h1>

<p align="center">SDKs and developer tooling for the InOrbit platform.</p>

# InOrbit SDK

The `iohr` command line and the official client libraries for the
[InOrbit API](https://docs.inorbit.hr) in Rust, TypeScript, Go, Python, C# and Java.

The repository holds two things: the `iohr` command line, released as a pre-release,
and six client libraries. Each library is a hand-written runtime plus an API surface that
`iohr sdk generate` writes, cut to what your credentials may call
([ADR 0011](docs/adr/0011-runtime-and-surface.md)). The libraries pass the shared conformance
suite, streams included (server-sent events or one `/v1/ws` socket, [design.md](docs/design.md)
section 7). Rust, TypeScript, Python and Go are on their registries. C# and Java are not
on NuGet or Maven Central yet: build them from source until their registry releases
([roadmap](docs/roadmap.md), M4b).

The versions below are read live from each registry, or from the repository's release
tags where there is no registry release, so the table does not go stale between releases:

| Part | Package | Latest release |
|---|---|---|
| Command line | `iohr` (APT, Homebrew, installers, [GitHub releases](https://github.com/inorbithr/sdk/releases?q=iohr)) | [![iohr pre-release](https://img.shields.io/github/v/tag/inorbithr/sdk?filter=iohr%2F*&include_prereleases&label=iohr)](https://github.com/inorbithr/sdk/releases?q=iohr), pre-release |
| Rust | [`inorbithr`](https://crates.io/crates/inorbithr), Rust 1.94 | [![crates.io](https://img.shields.io/crates/v/inorbithr?label=crates.io)](https://crates.io/crates/inorbithr) |
| TypeScript | [`@inorbithr/sdk`](https://www.npmjs.com/package/@inorbithr/sdk), Node 22.12, Bun, Deno, browsers | [![npm](https://img.shields.io/npm/v/%40inorbithr%2Fsdk?label=npm)](https://www.npmjs.com/package/@inorbithr/sdk) [![JSR](https://img.shields.io/jsr/v/%40inorbithr/sdk?label=JSR)](https://jsr.io/@inorbithr/sdk) |
| Go | [`github.com/inorbithr/sdk/go`](https://pkg.go.dev/github.com/inorbithr/sdk/go), Go 1.26 | [![Go module tag](https://img.shields.io/github/v/tag/inorbithr/sdk?filter=go%2Fv*&label=module)](https://pkg.go.dev/github.com/inorbithr/sdk/go) |
| Go, OpenTelemetry | [`github.com/inorbithr/sdk/go/otel`](https://pkg.go.dev/github.com/inorbithr/sdk/go/otel), an optional module for spans and metrics | [![Go module tag](https://img.shields.io/github/v/tag/inorbithr/sdk?filter=go%2Fotel%2Fv*&label=module)](https://pkg.go.dev/github.com/inorbithr/sdk/go/otel) |
| Python | [`inorbithr`](https://pypi.org/project/inorbithr/), Python 3.11 | [![PyPI](https://img.shields.io/pypi/v/inorbithr?label=PyPI)](https://pypi.org/project/inorbithr/) |
| C# | `InOrbit.Sdk`, .NET 8 | [![release tag](https://img.shields.io/github/v/tag/inorbithr/sdk?filter=csharp%2Fv*&label=tag)](https://github.com/inorbithr/sdk/releases?q=csharp), built from source; not on NuGet yet |
| Java | `hr.inorbit:inorbit-sdk`, Java 17 | [![release tag](https://img.shields.io/github/v/tag/inorbithr/sdk?filter=java%2Fv*&label=tag)](https://github.com/inorbithr/sdk/releases?q=java), built from source; not on Maven Central yet |

The contract the libraries are built against is synced from the platform into
[`spec/`](spec/), and the replay server in
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

`iohr` keeps several accounts as profiles, creates and revokes API tokens, proves
domains, connects an account's apps (`iohr connectors`, `iohr connections`: a key or a
browser sign-in), installs signed extensions such as the InOrbit agent (`iohr ext`),
checks Lab documents (`iohr lab check`) and keeps RFCs in the RFCs product (`iohr rfc`).
It talks to `api.inorbit.hr` and
`auth.inorbit.hr`, and to the extension registry only in `iohr ext install`, `upgrade` and
`sync` ([ADR 0012](docs/adr/0012-extensions.md)); there is no telemetry and no update
check. Releases before 1.0 are pre-releases. Every release carries checksums, an SBOM and build provenance you can
check with `gh attestation verify`. [cli/README.md](cli/README.md) has every command.

## Add a library to your project

```sh
iohr sdk add        # cargo add, npm install, pnpm/yarn/bun/deno add, uv/poetry/pdm add, pip or go get, from your project's files
```

The command is printed, then run by your project's own package manager
([cli/README.md](cli/README.md#add-the-sdk-to-a-project)).

## Generate a client for your account

```sh
iohr login
iohr sdk generate --lang rust --for default --out src/iohr   # or typescript, go, python, csharp, java
```

The generated code holds only the operations your profile's credential may call, so a call
it may not make fails to compile (in TypeScript, Rust, Go, C#, Java; in Python, the type checker).
Commit it with the `iohr.lock` beside it and run `iohr sdk check` in CI: it fails, with a
diff, when what the credential may call changes. The guide is at
[docs.inorbit.hr/docs/sdk](https://docs.inorbit.hr/docs/sdk/).

## What every library does

- Authenticate with an API token, or exchange an API key for a short-lived token, cache
  it and refresh it, so you never handle the exchange yourself.
- Return typed results and one error type per language, carrying the API's error `code`
  and `details`; a code the library does not know yet is kept, not dropped.
- Retry what is safe to retry (`429`, `503`, `504`, connection failures), honouring
  `Retry-After`; a write is retried only when the operation is marked safe to repeat.
- Behave the same in every language: one set of [conformance cases](conformance/) runs
  against all of them.

See [docs/design.md](docs/design.md) for the library design.

## Repository layout

| Path | Contents |
|---|---|
| [`cli/`](cli/) | The `iohr` command line, its release scripts and packaging |
| [`spec/`](spec/) | The API contract, synced from the platform |
| [`conformance/`](conformance/) | Shared behaviour cases and the replay server every SDK runs them against |
| [`rust/`](rust/), [`typescript/`](typescript/), [`go/`](go/), [`python/`](python/), [`csharp/`](csharp/), [`java/`](java/) | One runtime per language, with its generated public surface |
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
