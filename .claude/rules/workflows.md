---
paths:
  - ".github/**"
---

# GitHub workflows

ADR 0005 is the contract. When editing anything under `.github/`:

- Pin every action to a full 40-character commit SHA with the version in a comment:
  `uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`. Resolve a
  SHA with `git ls-remote https://github.com/<owner>/<repo> refs/tags/<tag>^{}`, never
  from memory.
- `permissions: {}` at the top of the workflow; grant per job, the least that works.
- `actions/checkout` always with `persist-credentials: false`, unless the job pushes.
- Never put `${{ github.event.* }}`, `${{ github.head_ref }}` or any user-controlled
  value inside `run:`; pass it through `env:` and quote it.
- Never use `pull_request_target` or `workflow_run`. Secrets are unavailable to fork PRs
  by design; a job that needs one runs only for same-repository branches.
- Every job calls `mise run <task>` rather than inlining commands, so CI and local match.
- A new required check is added to the `needs:` of `ci-ok`, not to branch protection.
- Run `mise run repo:check` (actionlint, zizmor, pin and permission checks) before
  finishing.
