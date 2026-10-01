@AGENTS.md

## Claude Code

- Plan mode first for anything that touches more than one language, `spec/`,
  `conformance/cases/` or `docs/design.md`. Parity work is easier to get right as a plan.
- For a new endpoint or behaviour, use `/add-endpoint`; to bring in a new contract, use
  `/sync-spec`; to check that the four languages still match, use `/parity` or the
  `parity-checker` agent.
- Verify with the narrowest task first (`mise run go:check`), then `mise run ci` before
  reporting done, and show the output. A hook formats each file you edit; another runs
  the checks for the languages you touched when you stop, and hands you the failures.
  Permissions deny edits to the synced spec, generated code and changelogs: if one
  blocks you, the fix belongs upstream or in the generator, not around the rule.
- When a convention here turns out wrong or missing, propose the edit to `AGENTS.md`
  (or the language file) in the same PR rather than leaving it for later.
