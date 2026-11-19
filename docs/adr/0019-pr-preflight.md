# 0019. A pull request preflight: every failing check named in one place, advisory

Status: proposed, 2026-11-19. Adds a job to the CI shape of [0014](0014-what-ci-runs-where.md).
Platform PRD 0221 item 5 (skills and checks so PRs stop failing CI).

## Context

A pull request here can fail for its text as well as its code: `pr-title` refuses a title
whose type or scope is not in its list (`ci(security): ...` is refused: the list has no
`security` scope, noted in the plugin's title guard on 2026-11-19), and the merge guard
refuses a merge whose description lacks the document lines it reads. Each of those says so in its own check, after a push, one at a time, and the legs a change
runs are visible only in `ci.yml`'s `changes` job. The platform PRD asked for one early check
that names all of them together.

The rules already exist, so the check must read them rather than copy them:

- the title types and scopes: the `types: |` and `scopes: |` lists and `requireScope` that
  `.github/workflows/pr-title.yml` passes to action-semantic-pull-request;
- the document lines: core's `crates/coordinator/src/document_lines.rs` (core 5073e3da,
  2026-10-23). A `PRD`, `RFC` or `ADR` line counts only at the start of a line, optionally
  after `Implements:`, `Part of:` or `Touches:` (`PRD: 0182`, `Implements: RFC 0153`,
  `rfc-0158.1`, `ADR_142`); prose (`this follows RFC 0171`) and `PRD: PRD 0182` do not.
  The first filled `Monitors:` line counts: monitor ids, checks file paths, or
  `none (<reason>)` with a real reason. A fix (a `fix:` title, or a `fix/` branch without
  another type) also needs a `Study:` line, the inorbit plugin's `approval_docs_guard` rule;
- the legs: `tools/ci-changed.sh --dry-run`, the path rules `mise run ci:changed` uses.

## Decision

- `tools/pr-preflight.py` (standard library only) checks the title and the description with
  those rules and runs `ci-changed.sh --dry-run` against the pull request's base. Its test,
  `tools/test_pr_preflight.py`, runs in `repo:check` and holds the core cases it mirrors.
- `mise run pr:preflight` runs it against the branch's open pull request (through `gh`), or
  the HEAD commit when there is none, prints the report and exits 1 on a finding;
  `--title` and `--body-file` check a draft before it is opened.
- The `pr-preflight` workflow runs it on `pull_request` (opened, edited, reopened,
  synchronize, ready for review). It writes the report to the job summary and one error
  annotation per finding, and always passes: it is not in `ci-ok`'s needs and is not a
  required check. Title and description reach the script through `env`, never `run:`.
- On a same-repository pull request it also creates or edits one comment that starts with
  `<!-- pr-preflight -->`; a clean pull request with no such comment gets none. That job
  alone holds `pull-requests: write`, and the token reaches only the comment step. A fork's
  token cannot write (and Dependabot's cannot either), so those get the summary only.

## Consequences

- The title list stays in `pr-title.yml` and the path rules in `ci-changed.sh`; a change to
  either changes the preflight with it. The document-line rules are a mirror of core's and
  of the plugin's: a change there needs the same change here, and the test names the cases.
- The preflight is advice; `pr-title`, the merge guard and `ci-ok` still decide.
- Not covered: the `no AI attribution` check, which keeps its own patterns.

## Sources

- core `crates/coordinator/src/document_lines.rs` at 5073e3da (2026-10-23)
- inorbit plugin `scripts/approval_docs_guard.py` (ai-kit), the Study-for-fix rule
- https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/workflow-commands-for-github-actions#setting-an-error-message
- https://docs.github.com/en/actions/security-for-github-actions/security-guides/automatic-token-authentication#permissions-for-the-github_token
