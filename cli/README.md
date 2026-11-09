# iohr, the InOrbit command line

`iohr` signs you in to InOrbit and calls its public API from a terminal, a script or CI. It
also generates SDK surfaces cut to your account, installs signed extensions, sets up
projects, and answers questions about the folder you are in with your own AI agent.

## Status

Pre-release (0.x). Commands and output can still change between versions, and only the
latest release receives fixes ([support policy](../SUPPORT.md)). The latest release is
`iohr/v0.1.0-alpha.38`, published 2026-11-06 ([releases](https://github.com/inorbithr/sdk/releases?q=iohr),
[CHANGELOG.md](CHANGELOG.md)).

These pages describe `main`. A change merged after the latest release ships with the next
one. `iohr-preview` is the same program under another name, built ahead of a release and
not published as a package; today it carries the `?` modes, `--role`, the extension offer
in `iohr project init` and `iohr ext show` with no name before a release does.

## Install

```sh
brew trust inorbithr/tap                                          # once per machine: Homebrew refuses untrusted taps
brew install inorbithr/tap/iohr                                   # macOS, Linux
curl -fsSL https://packages.inorbit.hr/install.sh | sh            # Linux, macOS, no sudo
powershell -c "irm https://packages.inorbit.hr/install.ps1 | iex" # Windows
```

From a clone of this repository:

```sh
cargo install --locked --path cli/crates/iohr
```

The installers check every download against the release's `SHA256SUMS`. APT, upgrades,
winget and how to check a release by hand are on [Install and upgrade](docs/install.md).

## Quickstart

```sh
iohr login                      # opens InOrbit's sign-in page, or prints a link and a code
iohr whoami                     # subject, account, plan, scopes and expiry
iohr api GET /v1/me             # one API call as you
iohr sdk add                    # add the published SDK to the project in this folder
iohr sdk config                 # what an SDK client here would use, secrets redacted
```

`iohr --help` lists every command in five groups (Sign-in, Account, Build, Agents,
Operate), and every command ends its `--help` with real examples.

## Guides

| Page | What it covers |
|---|---|
| [Install and upgrade](docs/install.md) | Homebrew, the install scripts, APT, from source, `iohr upgrade`, checking a release |
| [Sign in](docs/sign-in.md) | `iohr login`, accounts and teams, API tokens, `IOHR_TOKEN`, profiles, `iohr auth token` for other programs |
| [Command reference](docs/commands.md) | Every command and its flags, the global flags, the help, usage errors and exit codes |
| [SDKs and the API](docs/sdk.md) | `iohr sdk add`, `config`, `generate`, `check` and `examples`; `iohr api` and `iohr openapi` |
| [Domains and connections](docs/domains-and-connections.md) | Proving a domain with DNS, connecting apps with a key or a browser sign-in, grants |
| [Extensions](docs/extensions.md) | Finding, installing, verifying and pinning extensions; building one with `iohr ext init`, `dev` and `lint` |
| [Projects](docs/projects.md) | `iohr project init`, templates, the agent-built project, reading your public site, the atlas and `iohr sync` |
| [Roles and memory](docs/roles-and-memory.md) | Built-in roles, `--role`, and `iohr memory` as local files |
| [Asking your agent](docs/ask.md) | `iohr ask`, the `?` prompt in zsh, permission rules, the asks agents file for you |
| [Sessions and the panel](docs/session-and-panel.md) | `iohr session`, `iohr stop`, bare `iohr` and the floating panel |
| [The terminal theme](docs/terminal-theme.md) | `iohr theme`: the Mac app's terminal themes as a Terminal.app profile and a zsh prompt |
| [Runner](docs/runner.md) | This machine as a coordinator runner on an Anthropic API key (your own plan for personal use only): capacity, upgrades, gate reports, approved run asks |
| [Files, network and dependencies](docs/files-and-dependencies.md) | Where `iohr` keeps things, which hosts it talks to, and every runtime dependency with its reason |

Guides with captured output are also on the developer site:
[what iohr can do](https://developers.inorbit.hr/docs/tools/iohr/),
[projects](https://developers.inorbit.hr/docs/tools/projects/),
[the `?` prompt](https://developers.inorbit.hr/docs/tools/ask/),
[extensions](https://developers.inorbit.hr/docs/tools/extensions/) and
[iohr and iohr-preview](https://developers.inorbit.hr/docs/tools/preview/). These pages are
the reference they link back to.

## More

- Configuration shared with the SDKs: [docs/config.md](../docs/config.md)
- Security requirements for the command line (SR-10 to SR-34): [docs/security/requirements.md](../docs/security/requirements.md)
- Reporting a vulnerability: [SECURITY.md](../SECURITY.md)
- Why the command line is built this way: [ADR 0009](../docs/adr/0009-the-command-line.md)

## Licence

Apache License 2.0, see [LICENSE](../LICENSE).
