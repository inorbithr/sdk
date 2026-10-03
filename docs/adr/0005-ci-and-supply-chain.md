# 0005. CI shape and supply-chain rules

Status: accepted, 2026-10-01; where each check runs is amended by [0014](0014-what-ci-runs-where.md)

## Context

A public repository that publishes to four registries is a supply-chain target. Fork pull
requests run untrusted code; workflow injection and compromised actions are the common
failures.

## Decision

- **CI equals local.** Every CI job calls a `mise run` task; `mise run ci` locally runs
  the same checks. Toolchains come from `mise.toml`, matrix cells override one tool
  version through `MISE_<TOOL>_VERSION`.
- **Path filters plus one required check.** Jobs run only for the languages a change
  touches (a `spec/` or `conformance/` change runs all of them); the `ci-ok` job
  aggregates results and is the only required status check.
- **Actions pinned by full commit SHA**, with the version in a comment; Dependabot bumps
  them on a 7-day cooldown.
- **Least privilege**: `permissions: {}` at workflow level, grants per job;
  `persist-credentials: false` on checkout; no `${{ github.event.* }}` inside `run:`.
- **No `pull_request_target` and no `workflow_run` on fork code.** Fork PRs get read-only
  tokens and no secrets.
- **Static checks on the workflows themselves**: zizmor on every PR touching
  `.github/`; CodeQL for Go, Rust, JavaScript/TypeScript, Python and Actions; OpenSSF
  Scorecard weekly; dependency review on PRs.
- **Releases**: OIDC trusted publishing, build provenance attestations, an SBOM per
  release, environments with required reviewers (0003).
- **Claude in CI**: `@claude` answers only users with write access; automatic review runs
  only on pull requests from branches in this repository, because secrets are withheld
  from forks.

## Sources

- https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions
- https://docs.zizmor.sh
- https://github.com/ossf/scorecard
- https://github.blog/changelog/2025-11-07-actions-pull_request_target-and-environment-branch-protections-changes/
- https://code.claude.com/docs/en/github-actions
