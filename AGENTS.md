# InOrbit SDK

Client libraries for the InOrbit public API (`https://api.inorbit.hr`) in Go, Rust,
TypeScript and Python, in one repository. This file is the shared brief for every coding
agent and for people. Language-specific rules live in each language directory's own
`AGENTS.md` / `CLAUDE.md`.

## Layout

| Path | What it holds |
|---|---|
| `spec/` | The contract, vendored from the platform: the public OpenAPI slice, the error envelope, the socket frames. Never hand-edited. |
| `conformance/` | Language-neutral test cases (YAML) and the replay server every SDK is tested against. |
| `typescript/`, `python/`, `go/`, `java/`, `csharp/`, `rust/` | One package each: a hand-written runtime and the public surface `iohr sdk generate` writes into it (ADR 0011). |
| `cli/` | The `iohr` command line: a Cargo workspace of its own (ADR 0009), with `cli/AGENTS.md`. |
| `examples/<lang>/` | Small programs that compile in CI and are quoted by the READMEs. |
| `docs/` | `design.md` (cross-language API rules), `adr/` (decisions), `releasing.md`, `style.md`. |
| `tools/` | Repository scripts used by `mise` tasks and hooks. |

## Source of truth

1. **The platform defines the API; this repo follows it.** `spec/` is synced from the
   platform with `mise run spec:sync` and records the commit it came from in
   `spec/SOURCE`. A wrong spec is fixed upstream, then synced. Never patch `spec/` by hand.
2. **`docs/design.md` defines how the SDKs look.** All six languages expose the same
   concepts with the same names, adjusted only for each language's idiom
   (`get_me` / `GetMe` / `getMe`). A difference between languages is a bug unless
   `design.md` lists it.
3. **`conformance/cases/` defines behaviour.** Retries, token refresh, error mapping and
   streaming are specified as cases, and every SDK runs every case. A behaviour change is
   a new or edited case first, then code in every language.

## Commands

[mise](https://mise.jdx.dev) pins every toolchain and runs every task. `mise install`
once, then:

- `mise run ci:changed`: what CI runs for your changes since `origin/main` (same path
  rules). Run it before a push and before saying a change is done.
- `mise run ci`: every check in every language; before a release, or when in doubt.
- `mise run <lang>:check`: format check, lint, type check and unit tests for one
  language (`go`, `rust`, `ts`, `py`).
- `mise run <lang>:fmt`: format one language in place.
- `mise run gen`: regenerate every `generated/` directory from `spec/`. CI fails if the
  result differs from what is committed.
- `mise run conformance` or `conformance:<lang>`: run the shared cases.
- `mise run examples:check`: compile every example.
- `mise run spec:lint`: lint `spec/` and check it for breaking changes against `main`.

A task for a language whose package does not exist yet prints a skip line and exits 0.

## Rules that are not defaults

- **No hand edits in `generated/` or `spec/`.** Change the generator config or the
  upstream contract, then regenerate.
- **Every public change lands in every language that has a runtime, in the same pull request**, with a
  conformance case and an example when behaviour or surface changes. If one language
  cannot follow yet, the PR says so and opens an issue labelled `parity`. The runtimes
  themselves arrive one language at a time (ADR 0011): a runtime ships when it passes
  every case, and a case a runtime does not pass yet is `pending` for it, never weakened.
- **Wire names are the API's names.** JSON is snake_case on the wire in every language;
  64-bit integers travel as strings; timestamps are RFC 3339 strings. Each SDK converts at
  its boundary and nowhere else.
- **Secrets never print.** Key secrets and access tokens are redacted in every `Debug`,
  `repr`, `String`, `toJSON` and log line. A test checks this in each language.
- **No new runtime dependency without a reason in the PR.** Each package keeps a small,
  stated dependency set (see its `AGENTS.md`).
- **Stubs and unfinished paths are labelled.** Code that is not implemented returns an
  explicit "not implemented" error, never a fake value.
- **The SDK talks only to the public API and the token endpoint.** No other host, no
  telemetry sent anywhere.

## Security and compliance

- The security requirements in `docs/security/requirements.md` (SR-01 to SR-28) are as
  binding as `docs/design.md`; code review rejects a change that breaks one.
- Never put a key secret, token, health data (PHI), personal data or card number in a
  log line, error message, URL, test fixture or example output.
- A change that affects a requirement, the threat model or a control updates
  `docs/security/` in the same pull request.
- Vulnerabilities are handled privately as `SECURITY.md` describes; never in a public
  issue, pull request or commit message.
- InOrbit keeps a private compliance programme; agents with access to it follow it too.

## Commits and pull requests

- [Conventional Commits](https://www.conventionalcommits.org) with the language as scope:
  `feat(go): ...`, `fix(py): ...`, `docs: ...`, `ci: ...`, `spec: ...`. Scopes:
  `go`, `rust`, `ts`, `py`, `java`, `csharp`, `cli`, `spec`, `conformance`, `examples`,
  `ci`, `docs`,
  `repo`.
  Release notes and version bumps are generated from these.
- One logical change per commit. A breaking change says `!` and has a `BREAKING CHANGE:`
  footer.
- Never edit `CHANGELOG.md` files or versions by hand; release-please owns them.

## Where to look

- Cross-language API rules: `docs/design.md`
- Why things are the way they are: `docs/adr/`
- Writing style for READMEs, docs and error messages: `docs/style.md`
- Releasing: `docs/releasing.md`
- Contract quirks and what we asked the platform to fix: `spec/README.md`
