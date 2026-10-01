---
name: new-adr
description: Write a new architecture decision record under docs/adr with the next number. Use when a design choice is made or an earlier decision changes.
disable-model-invocation: true
argument-hint: "[short title]"
---

# New ADR: $ARGUMENTS

Existing records:
!`ls docs/adr`

1. Take the next number. File name: `NNNN-kebab-title.md`.
2. Sections: Status (proposed or accepted, with today's date), Context, Decision,
   Consequences, Sources. Under a page. Cite primary sources.
3. If it replaces an earlier record, set that record's status to
   `superseded by NNNN` and change nothing else in it.
4. Add the row to `docs/adr/README.md`.
5. Update `docs/design.md` or an `AGENTS.md` if the decision changes a rule there.
