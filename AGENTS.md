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
| `go/`, `rust/`, `typescript/`, `python/` | One package each. `generated/` inside is codegen output; everything else is written by hand. |
| `examples/<lang>/` | Small programs that compile in CI and are quoted by the READMEs. |
| `docs/` | `design.md` (cross-language API rules), `adr/` (decisions), `releasing.md`, `style.md`. |
| `tools/` | Repository scripts used by `mise` tasks and hooks. |

## Source of truth

1. **The platform defines the API; this repo follows it.** `spec/` is synced from the
   platform with `mise run spec:sync` and records the commit it came from in
   `spec/SOURCE`. A wrong spec is fixed upstream, then synced. Never patch `spec/` by hand.
2. **`docs/design.md` defines how the SDKs look.** All four languages expose the same
   concepts with the same names, adjusted only for each language's idiom
   (`get_me` / `GetMe` / `getMe`). A difference between languages is a bug unless
   `design.md` lists it.
3. **`conformance/cases/` defines behaviour.** Retries, token refresh, error mapping and
   streaming are specified as cases, and every SDK runs every case. A behaviour change is
   a new or edited case first, then code in all four languages.

## Commands

[mise](https://mise.jdx.dev) pins every toolchain and runs every task. `mise install`
once, then:

- `mise run ci`: exactly what CI runs. Run it before saying a change is done.
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
- **Every public change lands in all four languages in the same pull request**, with a
  conformance case and an example when behaviour or surface changes. If one language
  cannot follow yet, the PR says so and opens an issue labelled `parity`.
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

## Commits and pull requests

- [Conventional Commits](https://www.conventionalcommits.org) with the language as scope:
  `feat(go): ...`, `fix(py): ...`, `docs: ...`, `ci: ...`, `spec: ...`. Scopes:
  `go`, `rust`, `ts`, `py`, `spec`, `conformance`, `examples`, `ci`, `docs`, `repo`.
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
