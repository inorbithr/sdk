---
title: "Release concurrency: one group per job, not per workflow"
status: accepted
date: 2026-11-27
summary: "A held environment review on one release blocked release-please and every other package for three days; release.yml now serialises per job and per tag instead of per workflow run."
---

# 0002. Release concurrency: one group per job, not per workflow

Platform Study 0094.92 (RFC 0147, PRD 0152, ADR 0141). Built in the pull request that adds
this file.

## Asked
Make a held environment review block only its own release, never release-please or other
packages, and never let a newer push cancel a pending publish, while keeping whatever the
workflow-level `concurrency: release` group protected.

## Found (2026-11-27)
- Release run 45410607332 (iohr 0.1.2, release PR #592) has waited on the `release-cli`
  review since 2026-11-24. It held the `release` group, so every later push-to-main run was
  pending, and GitHub keeps one pending run per group: runs 45636977645, 45639953437 (the
  merge of csharp 0.4.1, #324) and 45711587479 were cancelled, and release-please never
  refreshed release PR #608.
- The group came with the repository foundation (#1, `bb5c864c`, 2026-10-01) with no
  stated reason; no ADR or doc mentions it. What it gave in practice: no two release-please
  runs at once (release PRs, tags), no two publishes of one tag (a push run and a manual
  republish), no two uploads of the shared APT index on R2, and no running job cancelled.
- Every per-version channel writes its own place: the tap and client policy jobs use a
  branch per version (`iohr-<v>`, `clients-cli-<v>`), the registries and release files are
  per tag. Only `iohr-apt` rewrites shared state: `cli/release/apt.sh` rebuilds
  `dists/stable` from scratch and uploads it over the last one.
- Other workflows key their groups by PR number or ref (`ci.yml`, `cli-build.yml`,
  `pr-preflight.yml`); `cli-build.yml`, which `iohr-build` calls, already has a group per
  ref and tag. No second release grouping exists.

## Decided
- No workflow-level group. `release-please` gets `release-please`, `cancel-in-progress:
  false`: a run stopped after tagging would leave that release unpublished, since only the
  run that creates a release reports it; a pending run replaced by a newer push is harmless,
  as the newer one works from `main` HEAD.
- Each publish job gets `release-<tag>`; a manual republish of a tag uses the same key.
  `iohr-tap` and `iohr-policy` get `release-<tag>-tap` and `-policy`: they start together
  with `iohr-apt` after `iohr`, and siblings sharing a key would queue and cancel each
  other. The `iohr` job and its dependents never share a key, so `needs` cannot deadlock.
- `iohr-apt` keeps one key for every version, `release-iohr-apt`, because its index is
  shared. A held apt review holds only the next version's apt job.

## Built
`.github/workflows/release.yml` (job-level groups) and docs/releasing.md (Concurrency).
`mise run repo:check` (typos, actionlint, zizmor, repository checks) passes.
