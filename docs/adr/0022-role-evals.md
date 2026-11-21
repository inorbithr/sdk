# 0022. Role evals: planted-defect cases, replayed offline, recorded nightly

Status: proposed, 2026-11-19

## Context

ADR 0020 made the built-in roles one tree with one root and a check before import. Nothing yet
says whether a role helps. A role can be well formed and still make the agent worse: it can
raise cost, miss the defect it exists for, or report problems that are not there. The roles
upgrade plan has more changes queued against these texts, so each one needs a measure that
says whether it is better before it lands.

Two pieces already existed. The evals project template (`templates/project/evals`, platform
ADR 0221) has a thresholds file with a pass bar and per-scorer minimums, and refuses a scorer
with no bar. But it is a template with `@@` placeholders that `init` fills in, not a library
this crate can import. `tests/roles_runtime.rs` has a fake `claude` that records its argv for
the dry run. Neither runs an agent on a repository and scores what it did.

## Decision

**Cases.** Each role has a directory of cases under `cli/crates/iohr/evals/roles/cases/`.
A case is a small repository with one defect planted (`fixture/`), the same repository fixed
(`clean/`), a fix check that fails on the first and passes on the second, the commands that
prove the work, and verdicts on findings that are not the planted one. Every role has the
same five shared cases: a stale dependency with an advisory, dead code, a change with no
test, a scope trap (a nearby problem the task does not ask to fix), and a method case. The
method case asks for the current version of a library, read at its source before the first
edit and reported with the date it was checked. A shared case may point at another role's
case of the same language (`same_as`), so it is written once. Each language role has its own
planted cases on top of the shared ones.

**Arms.** Each case is recorded with no role (`none`) and with the current pack. The pack
under test is the role text that `iohr ask --role <name> --dry-run` puts in the argv, hashed.
A pack change is the proposed arm, and the committed scores of the pack before it are the
current arm. Each arm runs each case three times (`[arms] runs`). The median of the runs is
scored.

**Scorers.** Four scorers, each pass or fail per run:

- recall: the case's defect caught. A planted defect counts when it is named, or fixed (the
  check passes on a changed tree). The other kinds need the check to pass and their own sign:
  the advisory's id named, a search for callers or a linter run, a test added, the scope's files left
  alone, the version and the date it was checked reported;
- precision: no false finding on the fixture or the clean twin;
- proof: the proof commands ran, and each finding cites a command that ran;
- size: changed lines at most twice the reference fix.

Size counts added plus removed lines (churn), not the net change, so a rewrite cannot hide
behind a small net. Lockfiles are listed but not counted. Cost is the median tokens per case.

**The bar and the landing rule.** Both live in `thresholds.toml`, which the test enforces. In
the template's format, the bar is a pass rate, an error count and a minimum per scorer. A
pack change lands only when recall or precision rises by at least one case, neither falls by
more than one case, and proof does not fall. Cost may rise by at most
max(10%, 2 × the measured run-to-run spread). The spread term exists because three runs of a
real model differ, and a rule tighter than that noise would refuse changes at random. The
bars are provisional: they will be set from the first real baseline.

**Replay and recording.** Normal CI runs `tests/role_evals.rs` offline. It reads the committed
transcripts, scores them, and fails in these cases:

- a pack changed without new transcripts;
- an arm is under the bar;
- a change breaks the landing rule against a real baseline;
- a finding has no verdict;
- the committed scores differ from the recomputed ones.

Recording is opt-in. `record_live` drives the real `claude` with the argv from the dry run,
signed in with a subscription token (`CLAUDE_CODE_OAUTH_TOKEN`), never an API key.
`record_synthetic` drives a scripted fake agent through the same pipeline.
`.github/workflows/role-evals.yml` records every role nightly and on demand, one job per role.
Its live part skips with a notice when the secret is missing. It uploads the recordings for a
person to commit, and never pushes.

**Synthetic is labelled.** Every transcript and score file says whether it is synthetic. A
synthetic recording may not replace a real baseline. Against a synthetic baseline only the bar
applies, because a synthetic score measures the pipeline, not a role. The recordings committed
with this ADR are all synthetic.

## Consequences

- Every change to a role's text must carry new recordings, or CI fails. The roles worker's
  PRs (#588, #595) re-record before they merge.
- A live recording lets Bash run only the language's commands
  (`--allowedTools=Bash(<cmd>:*)` from `[languages]`). The dry run forbids Bash, and
  `iohr ask` has no flag to allow some commands. The recorder therefore edits the argv after
  the dry run. That is a workaround: an `iohr ask --allow` flag is a follow-up.
- "Offline" for the fixture checks and the non-method cases means the package tools are set
  offline through the environment (`GOPROXY=off`, `PIP_NO_INDEX`, `npm_config_offline`,
  `CARGO_NET_OFFLINE`). It does not mean the network is cut. A tool that ignores these can
  still reach out.
- The method case's truth moves. The live recorder reads the source again on every fixture
  run. When the source has moved on, it records the run as an error ("truth moved"), and a
  person updates the case's `[method.truth]` and its check. The truth is never edited
  silently.
- The nightly run spends subscription usage: about six hundred agent runs a night across six
  roles. Its volume is the owner's to confirm.
- The cases' fixtures carry planted defects on purpose. CodeQL and typos skip them, and their
  manifests are stored as `<name>.eval`.

## Sources

- [ADR 0020](0020-built-in-roles-inherit-and-check-before-import.md), built-in roles.
- `cli/crates/iohr/templates/project/evals/thresholds.toml`, the template's bar format
  (platform PRD 0088.1, ADR 0221).
- `cli/crates/iohr/tests/roles_runtime.rs`, the fake agent this harness's recorder follows.
- `claude --help`, Claude Code 2.1.328, read 2026-11-19: `--tools`, `--allowedTools`,
  `--disallowedTools`, `--strict-mcp-config`, `--output-format stream-json`; no
  `--max-turns`. The workflow pins 2.1.327, the newest release older than the minimum
  release age.
