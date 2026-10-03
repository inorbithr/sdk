# 0014. What CI runs where

Status: accepted, 2026-10-03. Amends the CI shape of [0005](0005-ci-and-supply-chain.md).

## Context

Six languages, each with a version matrix and Windows and macOS cells, made a merge to
`main` take 13 minutes, 30 jobs and 90 runner-minutes (2026-10-03, the C# merge): Windows
Rust alone took 11 minutes 49 seconds. Every language job also installed Rust and Go and
compiled `iohr` and the replay server, the same two binaries six times a run. Locally,
`mise run ci` ran every language one after another, over an hour.

## Decision

| Run | Languages | Versions | Operating systems |
|---|---|---|---|
| Pull request, push to `main`, merge queue | the ones the change touches (path filters) | oldest and newest supported | Linux |
| Nightly (03:23 UTC) and by hand (`workflow_dispatch`) | all | every supported version | Linux, Windows, macOS |
| release-please pull requests | all | every supported version | Linux, Windows, macOS |

- A `tools` job builds `iohr` and the replay server once on Linux and hands them to every
  Linux language job as an artifact; the tasks take them through `IOHR_BIN` and
  `REPLAY_PREBUILT` and build them themselves when those are unset, as on a laptop.
  Windows and macOS cells build their own.
- The generator's compile test, which needs Rust, runs in one cell per language (Linux,
  newest version).
- A failed nightly opens an issue titled "Nightly CI failed", or comments on the open one.
- `ci-ok` stays the only required check.
- Locally, `mise run ci:changed` runs what CI would for the changes since `origin/main`,
  with the same path rules; `mise run ci` stays the full run before a release.

## Consequences

- A merge is checked on Linux at the edges of each version range; a break specific to
  Windows, macOS or a middle version shows up in the nightly run, at most a day later,
  and always before a release, because release pull requests run everything.
- The supply-chain rules of 0005 are unchanged: pinned actions, least privilege, no
  credentials persisted; the nightly report job alone holds `issues: write`.

## Sources

- https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/storing-and-sharing-data-from-a-workflow
- https://docs.github.com/en/actions/writing-workflows/choosing-when-your-workflow-runs/events-that-trigger-workflows#schedule
