# 0018. Built-in roles: one root they inherit, one text limit, a check before import

Status: proposed, 2026-11-19

## Context

A code reading of iohr's built-in roles (`cli/crates/iohr/roles/`, `src/roles.rs`) found ten
problems. A catalogue file that did not parse was dropped without a word. The privacy patterns
the service applies at import lived only in a test, so iohr never warned before a 400. A role's
text was cut twice, at 4,096 bytes and at 8,000 characters (both added in #441 on 2026-11-07,
with no reason recorded). `iohr project init` and `iohr atlas init` each had their own roles
steps. The pack text said an edit "is not read back", which is wrong: `--role` reads the
project's pack first. A role under the wrong parent counted as unchanged. The dates were one
hard-coded day. Roles upgraded with no `--upgrade` did not say so. One timeout line was copied
into four roles. And the wire's older "personalities" names were scattered string literals.

The service (InOrbit core, a private repository) is the authority for what an import accepts.
Its `lint_tree` checks the privacy filter over the name, aliases, section titles, lines and
citations, a known model, and no override word in a child's core. Its `GetPersonality` builds
a role's `core` and `recall` from the chain, root first.

## Decision

- **One root, inherited.** Lines every engineer needs live once in `base-engineer`. Each pack
  and the `--role` text list the inherited lines first, as the service's brief does. The
  700-token core limit counts them too. `base-engineer` is never a pack and never routed.
- **A check before import.** `src/privacy.rs` copies core's patterns whole (core
  `crates/coordinator/src/learning/privacy.rs`, e3bdf449f). A test pins a digest of them.
  `src/roles/lint.rs` runs the client-side parts of `lint_tree` before anything is sent. A
  role that would be refused is reported as `failed`, naming the line, never the text. Core
  cannot be a dependency of this public repository, so the copy stays until core exports the
  patterns as a spec file.
- **One text limit.** 8,000 characters, cut once on a character boundary where the text is
  made (`roles::runtime::bound`). The 4,096-byte cut in the headless prompt is gone.
- **A parent is a line.** `decide` treats a parent other than the account's root as a change,
  under the same rules as a changed line.
- **Auto-upgrade, said.** A copy whose history is all imports is still upgraded without
  `--upgrade`, and its line says why ("not edited in InOrbit"). `iohr sync` never passes
  `--upgrade`. An unedited older root is upgraded the same way; a root of the person's own
  is never touched.
- **Dates per role.** Each catalogue role has a required `checked` day. A line may override
  it. Packs and seed memories carry the date, and a test refuses a missing or future one.
- **One roles start for both inits.** A language hint (the template's answer), else
  detection; the legacy `atlas/personalities/` move; and one line when nothing is sent. Each
  command keeps its own flags. The file writing stays separate: project init renders in
  memory, atlas init writes on disk.
- **Wire names behind constants.** `WIRE_ROLES_PATH`, `WIRE_IMPORT_PATH`, `WIRE_ROLES_KEY`,
  `WIRE_ROLE_KEY` and `LEGACY_CLI_ALIAS` keep the published "personalities" names, never
  renamed.
- **A catalogue file that fails to parse panics at load,** naming the file. It is compiled in,
  so a test catches it before release.

## Consequences

Packs written before this change lack the inherited lines until they are written again
(`iohr atlas init` never overwrites a pack). The privacy copy must move in the same release
as core's file; the pinned digest makes a silent drift fail the build.

## Sources

- core `crates/coordinator/src/learning/privacy.rs` at e3bdf449f (2026-10-14), `lint.rs` and
  `text.rs` at 00c977e9d (2026-10-31), read with `git show origin/main:`.
- #441 (094d1e5c, 2026-11-07): where `ROLE_MAX` and `TEXT_MAX` came in.
- [Roles and memory](../../cli/docs/roles-and-memory.md).
