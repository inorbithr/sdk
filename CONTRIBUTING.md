# Contributing

Thanks for helping. This page covers setup, the rules that keep four SDKs in step, and
how a change gets merged.

## Setup

1. Install [mise](https://mise.jdx.dev/getting-started.html).
2. `mise install` in the repository root. It installs the pinned Go, Rust, Node, pnpm,
   Python, uv and every linter.
3. `mise run ci` runs every check CI runs. `mise tasks` lists the rest.

A dev container (`.devcontainer/`) does the same for VS Code and Codespaces.

## The rules that matter

- **One behaviour, four languages.** A public change lands in Go, Rust, TypeScript and
  Python in the same pull request, with a [conformance case](conformance/). If you only
  know one of the languages, open the PR anyway and say so; a maintainer will finish
  the rest.
- **The contract comes from the API.** `spec/` and every `generated/` directory are
  produced by tools (`mise run spec:sync`, `mise run gen`); never edit them by hand.
- **Same names everywhere**, adjusted to each language's style, as described in
  [docs/design.md](docs/design.md).
- **Small dependency sets.** Each language's `AGENTS.md` lists what it may depend on.

## Commits and pull requests

- Titles follow [Conventional Commits](https://www.conventionalcommits.org) with the
  language as scope: `feat(go): add GetUsage`, `fix(py): keep unknown error codes`.
  Release notes and versions are generated from them, so the title is what users read.
- Breaking changes use `!` (`feat(ts)!: ...`) and explain the migration in the body.
- Keep a PR to one change. CI must be green; the required check is `ci-ok`.

## Using coding agents

The repository is set up for agents. `AGENTS.md` (read by most agents) and `CLAUDE.md`
(Claude Code) hold the same rules, and each language directory adds its own. Claude Code
users get project skills (`/add-endpoint`, `/sync-spec`, `/parity`) and review agents in
`.claude/`. Whatever tool you use, you are responsible for what you submit: run
`mise run ci` and read the diff.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
