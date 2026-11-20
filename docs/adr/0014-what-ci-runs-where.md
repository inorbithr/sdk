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
| release-please pull requests | the component's own whole matrix; the rest by path filters | every supported version for that component | Linux, Windows, macOS for that component |

- A change that touches the command line (`cli/**`, `rust/**`, `spec/**`, `mise.toml` or the
  workflow) also runs its newest-Rust cell on macOS, so a macOS break fails the pull request
  and not the merge queue. Its Windows cell runs nightly and on the `iohr` release pull
  request only. The merge queue skips the macOS cell and the Swift macOS cell (amendments of
  2026-11-19 below).
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

## Amendment, 2026-11-19: the macOS queue

GitHub runs at most 5 macOS jobs at once for an account on the Free, Pro and Team plans (50
on Enterprise), shared by every repository and runner size. On 2026-11-19 at 11:52 UTC,
5 macOS jobs ran and 20 waited; a pull request's macOS cell had waited 54 minutes, and two
merge-queue runs had waited since 09:16 and 10:36. Of the 20 waiting jobs:

- 12 came from the six open release-please pull requests. Each ran every language's whole
  matrix, so four macOS cells (Go, Rust, the command line, Swift), and release-please
  updates all six after every push to `main`, which starts them again.
- 6 came from the merge queue, which ran the command line's and Swift's macOS cells that the
  pull request had already passed.

Changes:

- A release-please pull request runs the whole matrix of its own component only, read from
  its branch name (`release-please--branches--main--components--<component>`). The other
  languages follow the path filters, as on any pull request. A component the workflow does
  not know yet still runs everything.
- The merge queue skips the command line's Windows and macOS cells and the Swift macOS cell.

Trade-offs:

- A break that only shows on macOS or Windows from combining a pull request with newer `main`
  is no longer caught in the merge queue. The push to `main` runs those cells again on the
  merged tree, and so does the nightly.
- A release pull request no longer proves the other languages on macOS and Windows. They
  have not changed in that pull request, and the nightly runs them every day.

## Amendment, 2026-11-19: Windows leaves pull requests

The owner works on macOS and does not ship a Windows build to anyone yet (decision of
2026-11-19 on the macOS question). The command line's Windows cell was the only Windows job
an ordinary pull request ran; the language Windows cells already ran only nightly and on
release pull requests.

Changes:

- The command line's Windows cell runs in a full run (nightly at 03:23 UTC, by hand) and on
  the `iohr` release pull request. Pull requests, pushes to `main` and the merge queue no
  longer run it.
- The command line's macOS cell is unchanged: it runs on every pull request and push to
  `main` that touches the command line, and the merge queue skips it.

Trade-offs:

- A break that only shows on Windows reaches `main` and shows up in the nightly run, at most
  a day later, and always before an `iohr` release, because its release pull request runs the
  Windows cell. This is less coverage per pull request, so it was the owner's decision.

## Amendment, 2026-11-19: caches are saved from `main` only

The command-line cells took 36 to 44 minutes. The repository's Actions cache held
10.85 GB, over its 10 GB limit, in 29 entries. About 7.8 GB of that was mise tool caches
saved by pull-request merge refs and merge-queue refs. Those caches can be read only by the
ref that saved them. Eviction then removed `main`'s Rust caches for macOS, Windows and Rust
1.94, so those cells compiled from cold on every run. On macOS that took 12 minutes
42 seconds before the first test.

- Every cache step saves only on a push to `main` and restores on every run. The Rust cache
  (`.github/actions/rust-cache`) already did this; every `jdx/mise-action` step now sets
  `cache_save` the same way.
- The trade-off is that a pull request that changes `mise.toml` installs its tools again on
  each run, about one to two minutes per job.

## Sources

- https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/storing-and-sharing-data-from-a-workflow
- https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache
- https://github.com/jdx/mise-action (inputs `cache` and `cache_save`)
- https://docs.github.com/en/actions/writing-workflows/choosing-when-your-workflow-runs/events-that-trigger-workflows#schedule
- https://docs.github.com/en/actions/reference/limits (maximum concurrent macOS jobs)
