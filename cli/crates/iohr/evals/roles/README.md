# Role evals

These measure whether an agent in a built-in role catches the defects the role exists for. The
agent must invent no others, show its proof, keep the change small, and cost no more than it
should. The decision and its limits are in [ADR 0022](../../../../../docs/adr/0022-role-evals.md).
The test is [`tests/role_evals.rs`](../../tests/role_evals.rs).

## What is here

| Path | What it holds |
| --- | --- |
| `thresholds.toml` | The bar every arm must clear, the landing rule, the runs per arm, the task suffix, the live recorder's model and the commands it may run per language. |
| `cases/<role>/<case>/case.toml` | One case: its kind, task, planted defect, fix check, proof commands and verdicts. A shared case can point at another role's with `same_as`. |
| `cases/<role>/<case>/fixture/` | A small repository with the defect planted. |
| `cases/<role>/<case>/clean/` | The same repository with the defect fixed: the reference fix and the precision twin. |
| `cases/<role>/<case>/check/` | Files the fix check needs beside the tree (a `check.sh`), never shown to the agent. |
| `transcripts/<role>@<pack>/<model>/<case>/run-N.jsonl` | One recorded run with the role: a line for the fixture and a line for the clean twin. |
| `transcripts/none/<model>/<role>/<case>/run-N.jsonl` | The same case with no role, the arm the role must beat. |
| `scores/<model>/<role>.toml` | The scores the replay computes from the transcripts. They are the baseline the next pack change is held to. |

Manifests and workflows in the fixtures are stored as `<name>.eval` (`go.mod.eval`,
`package.json.eval`, `.github/workflows/ci.yml.eval`). That keeps Dependabot, osv-scanner and
CodeQL from treating them as this repository's own. The recorder copies them without the
suffix.

Every role has the five shared cases: `stale-dependency`, `dead-code`, `no-test`,
`scope-trap` and `method` (find the current version at its source before the first edit,
and say the date it was checked). Each also has cases of its own for planted defects.

## The scores

The replay scores each run on four measures:

- **recall**: the defect was caught. A planted defect counts when it is named, or fixed (the
  fix check passes on a changed tree). The other kinds need the fix check to pass and their
  own sign: the advisory's id named, a search for callers or a linter run, a test added, the scope's
  files left alone, the version and the date it was checked reported.
- **precision**: nothing true was called wrong, on the fixture or on the clean twin.
- **proof**: the case's proof commands ran, and every finding cites a command that ran.
- **size**: the change is at most twice the reference fix in lines.

A case passes a scorer when the median of its runs does. Cost is the median tokens per case.

## Run them

```sh
cd cli
# The replay: offline and deterministic, part of every CI run.
cargo nextest run -p iohr --test role_evals
# Rewrite scores/ from the transcripts after a recording.
IOHR_UPDATE_EVALS=1 cargo nextest run -p iohr --test role_evals -E 'test(replay)'
# Every fix check fails on its fixture and passes on its clean twin (needs go, cargo,
# python3, node and zizmor).
cargo nextest run -p iohr --test role_evals --run-ignored only -E 'test(fixture_checks)'
# Record with the scripted fake agent: synthetic, proves the pipeline, never a role.
cargo nextest run -p iohr --test role_evals --run-ignored only -E 'test(record_synthetic)'
# Record with the real agent: `claude` on the PATH, signed in with a subscription token.
IOHR_EVAL_ROLES=go-engineer cargo nextest run -p iohr --test role_evals --run-ignored only -E 'test(record_live)'
```

`IOHR_EVAL_ROLES` (comma separated) narrows the recorders and the replay to some roles.
`IOHR_EVAL_ARMS` (`none`, `current` or both) narrows the recorders to some arms.

## When a role changes

A change to a role's text changes its pack hash, and the replay then fails: there are no
transcripts for the new pack. Record the role again, rewrite its scores, and commit both. The
replay then holds the new scores to the bar and, against a real baseline, to the landing rule:

- recall or precision must rise by at least one case;
- neither may fall by more than one case;
- proof may not fall;
- cost may rise by at most the larger of 10% and twice the measured run-to-run spread.

## Synthetic and real

Every transcript and score file says `synthetic = true` or `false`. The recordings committed
with this harness are synthetic. A scripted fake agent applies the reference fix and writes
the expected report, so they prove the pipeline and the scorers but measure no role. Real
baselines come from the nightly workflow (`.github/workflows/role-evals.yml`), which uploads
its recordings for a person to commit. A synthetic recording may never replace a real
baseline. While the baseline is synthetic, only the bar applies.

## Add a case

1. Make `cases/<role>/<name>/` with `case.toml`, `fixture/`, `clean/` and, if the check
   needs it, `check/`. Copy a case of the same kind and read `tests/role_evals/model.rs` for
   every field.
2. Run `fixture_checks` for the role. The check must fail on `fixture/` and pass on
   `clean/`.
3. Record the role again and rewrite its scores. If the replay names an unjudged finding,
   add a `[[verdict]]` to the case that says whether it is true and why.
