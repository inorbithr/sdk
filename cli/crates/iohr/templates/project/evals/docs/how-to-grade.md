# How to grade

This is how the suite in @@NAME@@ decides pass or fail, and how to change it without fooling
yourself. The run is `@@RUN@@`; the bar is `thresholds.toml`.

## The parts

| Part | File | What it is |
| --- | --- | --- |
| Dataset | `datasets/tickets.jsonl` | One case per line: an `id`, an `input` and the `expected` answer. |
| Solver | `@@SOLVER_FILE@@` | The thing under test: input in, answer out. The one here is a **stub**. |
| Scorers | `@@SCORERS_FILE@@` | Pure functions: case and answer in, 0 or 1 out. |
| Bar | `thresholds.toml` | What the results must reach. Missing it exits 1 and fails CI. |

A case passes when every scorer gives 1. The suite passes when the share of passing cases
reaches `min_pass_rate`, no more than `max_errors` cases raised, and each scorer's own mean
reaches its line under `[bar.scorers]`.

## The stub

The fixture solver is keyword rules, not a model. The report says so (`stub: true`, a solver
name that starts with `stub-`) and the terminal output starts with "STUB solver, not a model".
Its job is to let the suite run offline, for free and identically every time, so a change in
the result is a change in your code. A pass here shows the harness works; it says nothing
about how good a model is. Never quote it as a model's score.

## Grade your own thing

1. **Write cases from real work.** Take inputs people actually send, label the answer by hand
   and have a second person check the labels. Keep the cases small and one idea each. Twelve
   good cases beat two hundred copied ones. Remove customer data before a case goes in this
   repository (names, emails, card numbers): the dataset is committed.
2. **Cover the edges.** Every label needs cases, and so does the answer that is easy to get
   wrong: the ambiguous message, the empty input, the very long one, the one that tempts the
   solver into a wrong label. The test checks that every label has a case.
3. **Replace the solver.** @@SOLVER_HOWTO@@ Set `stub` to false only for a real model, and keep the version of the
   model in `name`. Sign in the way your provider's subscription allows and keep the credential
   in your secret store: the secret scan in CI fails the build if one lands here. A real
   model is not deterministic, so keep the offline suite on the stub (it guards the harness
   and the scorers) and run the real model as a separate, labelled job with its own bar.
4. **Write scorers that cannot be argued with.** Exact match, a regular expression, a
   structure check, a number within a tolerance. If you must use a model as the judge,
   label it a stub in tests, give it its own cases with known answers and its own bar.
5. **Set the bar from a measurement, then move it on purpose.** Run the suite, look at the
   result, and set each line a little below what you can defend. Raise a line when the
   solver improves. Lowering one is a change with a reason in the pull request, because a bar
   that slides down on every red run guards nothing.

## Prove the bar bites

A bar that cannot fail is decoration. `@@TEST_FILE@@` changes one expected answer in a
copy of the dataset and checks that the run exits 1 and names the case; it also checks that
a chatty but correct answer fails the format scorer, that a solver that raises is an error
and never a pass, and that a broken dataset or a scorer with no bar stops the run with exit
2 instead of grading fewer cases. When you add a scorer, add its line to `thresholds.toml`
and a test that a wrong answer scores 0.

## Reading a result

```text
STUB solver, not a model: stub-keyword-rules-v1: 12/12 cases pass
  exact: mean 1.000 (bar 0.950)
  format: mean 1.000 (bar 1.000)
  pass rate 1.000 (bar 0.950), errors 0 (at most 0)
BAR MET
```

`--json` prints the same as data (case ids and scores, never inputs or answers). `--show`
prints each failed case's input, expected and answer to your terminal only, so you can see
why without writing customer data into a report or a CI log. Exit codes: 0 the bar is met, 1
it is missed, 2 the dataset or the bar is wrong and nothing was graded.
