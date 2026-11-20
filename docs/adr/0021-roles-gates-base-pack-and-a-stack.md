# 0020. Built-in roles: gates in the role, the base as a pack, a project's whole stack

Status: proposed, 2026-11-19. Supersedes two points of [0019](0019-built-in-roles-inherit-and-check-before-import.md):
"`base-engineer` is never a pack" and "the 700-token core limit counts [the inherited lines]".

## Context

Three gaps were left after ADR 0019 (PR #588). First, a role said how to work but not how to
prove the work: no role carried the commands to pass before saying done. Second,
`base-engineer` reached a role only by inheritance, so an agent that loads the base by
itself had no file to read. Third, `selection` took one language. A repository with Go at the
root and TypeScript in `web/` got only the Go engineer. Roles that belong to files rather than
languages (infrastructure for a `Dockerfile` or `*.tf`, SQL for `migrations/` or `*.sql`) had
no way to turn on.

Two tables already listed the build files of each language: `roles::detect_language` and the
`MARKERS` table in `commands/sdk_add.rs`. They differed: roles read `tsconfig.json` and
`setup.py`, and `sdk add` read the lock files.

## Decision

- **Gates in the role.** A role TOML may hold `[[check]]` tables with three fields:
  `command`, what it `proves`, and whether it `blocks` (true unless it says false). Unknown
  fields are refused, so a misspelt `block` fails the catalogue test. The pack prints them
  after the core under "Before you say done, run", the inherited ones first, and
  `--role` carries that section with the core. `iohr atlas roles list --json` shows them as
  `checks`. The import lint keeps to the lines: a check is a command, and
  `npm ci --ignore-scripts` holds a word the lint refuses in a child's core line. The privacy
  filter still applies to every check command in the catalogue test.
- **The base is a pack.** `selection` returns `base-engineer` first. It is written to
  `atlas/roles/base-engineer.md` and recorded in `[roles]`. It is still never routed to
  (`host_roles` drops it), and it is still imported only as the root. Each child pack keeps
  its inherited lines, so a single pack still stands alone for `--role`.
- **Two budgets.** A pack's own core stays within 700 tokens (`PACK_CORE_MAX`). The base and
  a role together, gates included, stay within 1,400 (`LOADED_CORE_MAX`). Tokens are
  characters over four, as before.
- **A project's stack.** `roles::stack` returns every language and every marked role.
  It reads the template's answer first, then the language `inorbit.toml` declares, and
  otherwise the build files at the root and two levels down. The scan skips hidden folders,
  dependency and build folders, and fixtures. It does not follow links and stops after 2,000
  folders. Workspace manifests are not parsed: a depth-two scan finds the members of every
  common layout, and it costs one directory read per folder.
- **Markers in the role.** A role may list `markers`: `name/` is a folder, `*.ext` or
  `prefix*` is a file pattern, and anything else is an exact file name. Markers always apply,
  even when a language is declared. No catalogue role has markers yet, so the tests use
  fixture roles.
- **One build-file table.** `roles::LANGUAGE_FILES` is the only list. `sdk add` reads it
  through `roles::languages_at`, so `sdk add` now also finds a project by `tsconfig.json` or
  `setup.py`.

## Consequences

A project made before this change gets `base-engineer.md` the next time `iohr atlas init`
runs, because missing packs are written and existing ones are kept. Its `[roles]` record is
kept as it is. The service import does not carry checks: InOrbit's brief has no gates until
the service schema takes a `checks` field, so an account-sourced `--role` has none.
`keel-check` does not read the list yet. `atlas roles list` does not show the base, because it
lists the roles a project imports.

## Sources

- [0019](0019-built-in-roles-inherit-and-check-before-import.md) and PR #588 (3f68e73a, 2026-11-19).
- `cli/crates/iohr/src/roles.rs`, `roles/lint.rs`, `roles/runtime.rs`, `commands/sdk_add.rs`
  at 3f68e73a: the two build-file tables and the one-language `selection`.
- [Roles and memory](../../cli/docs/roles-and-memory.md).
