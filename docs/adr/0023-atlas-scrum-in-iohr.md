# 0023. The scrum board moves into iohr: `iohr atlas scrum`

Status: proposed, 2026-11-23

Implements RFC 0234, slice S2.

## Context

Eye kept a file-based scrum board (cards and a board file under `atlas/scrum/`) with its own
checker, views, guards, watch and Planner sync. Two copies of the schema loader, the terminal
helpers, the date maths and the Planner runner lived in Eye beside ones the CLI already has.

## Decision

The board lives in the CLI as `iohr atlas scrum` (init, check, render, list, show, add, move,
start, done, drop, block, migrate, sync, watch). Reused, not copied: `schema.rs` validates cards
against the embedded `scrum/card` and `scrum/board` schemas, `planner::Plans` carries sync,
`ui.rs` draws, chrono does dates, and `atlas::files` writes the starter board for new projects.

- Every write is checked first, written to a temp file and renamed, and refused if the file
  changed since it was read.
- A blocked card refuses `start` and a move to `doing`. `done` needs `--pr` or `--evidence`.
  `--force TEXT` overrides and the reason is kept in the card notes.
- Planner mapping: backlog is proposed, ready is ready, doing and review are building (review
  adds the note "in review"), done is done, dropped is dropped. External id `scrum:<repo>/<board>`.
- Verb gap: the CLI has no verb for ready or dropped, so sync calls `Plans::set_status`. Dropped
  is final in the Planner, so a card moved out of it is reported as a refused step and its later
  steps are skipped; other cards continue. Ids are written back to `card.planner`; a second run
  sends nothing.

## Consequences

No endpoint, scope or proto changes. Semver: pre-1.0 feature, a patch. Eye's runner, schema copy,
terminal module and date module can be deleted in slice S3.
