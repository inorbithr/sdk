---
name: release-check
description: Pre-release review for one package: pending release PR, API changes since the last tag, changelog wording, parity, docs. Use before merging a release-please PR.
disable-model-invocation: true
argument-hint: "[go|rust|typescript|python]"
allowed-tools: Bash(git log *) Bash(git describe *) Bash(git diff *) Bash(gh pr list *) Bash(gh pr view *) Bash(mise run *)
---

# Release check: $ARGUMENTS

Last tag:
!`git describe --tags --abbrev=0 --match "$ARGUMENTS/v*" 2>/dev/null || echo "none yet"`

1. Find the open release PR for this package (`gh pr list --label "autorelease: pending"`).
2. List the commits since the last tag for this package's path and check each is
   correctly typed: a `fix` that changes public API is a `feat` or a breaking change.
3. Run the API check for the language (`mise run go:api`, `rust:api`, `py:api`, or the
   api-extractor report for TypeScript) and confirm the proposed version bump matches.
4. Read the generated changelog as a user would: each line says what changed for them.
5. Run `/parity` for anything this release adds.
6. Report: go or no-go, with the reasons. Never merge or tag yourself.
