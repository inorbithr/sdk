"""`python -m @@MODULE@@`: grade the solver on the dataset and check the pass bar.

Exit 0 when the bar is met, 1 when it is missed, 2 when the dataset or the bar is wrong. The
report says which solver ran and whether it is a stub. It holds case ids and scores only; the
inputs and answers stay out of it (they can be customer data), unless you ask for `--show`,
which prints the failed cases to your terminal and nowhere else.
"""

import argparse
import json
import sys
from pathlib import Path

from .dataset import DEFAULT_BAR, DEFAULT_DATASET, SuiteError, load_cases
from .runner import Report, evaluate, load_bar, missed
from .solver import StubSolver


def to_json(report: Report, bar_missed: list[str]) -> dict:
    return {
        "solver": report.solver,
        "stub": report.stub,
        "cases": len(report.results),
        "pass_rate": round(report.pass_rate, 4),
        "errors": report.errors,
        "scorers": {n: round(report.mean(n), 4) for n in report.results[0].scores},
        "bar_met": not bar_missed,
        "missed": bar_missed,
        "results": [{"id": r.id, "passed": r.passed, "scores": r.scores, "error": r.error} for r in report.results],
    }


def main(argv: list[str] | None = None, solver=None) -> int:
    p = argparse.ArgumentParser(prog="@@MODULE@@", description=__doc__.split("\n\n")[0])
    p.add_argument("--dataset", type=Path, default=DEFAULT_DATASET, help="JSONL cases (id, input, expected)")
    p.add_argument("--bar", type=Path, default=DEFAULT_BAR, help="the pass bar (thresholds.toml)")
    p.add_argument("--json", action="store_true", help="print the report as JSON")
    p.add_argument("--show", action="store_true", help="print each failed case's input, expected and answer")
    args = p.parse_args(argv)
    solver = solver or StubSolver()
    try:
        cases = load_cases(args.dataset)
        bar = load_bar(args.bar)
    except SuiteError as e:
        print(f"suite error: {e}", file=sys.stderr)
        return 2
    report = evaluate(cases, solver)
    bar_missed = missed(report, bar)
    if args.json:
        print(json.dumps(to_json(report, bar_missed), indent=2))
    else:
        label = "STUB solver, not a model: " if report.stub else ""
        print(f"{label}{report.solver}: {sum(r.passed for r in report.results)}/{len(report.results)} cases pass")
        for name in report.results[0].scores:
            print(f"  {name}: mean {report.mean(name):.3f} (bar {bar.scorers[name]:.3f})")
        print(f"  pass rate {report.pass_rate:.3f} (bar {bar.min_pass_rate:.3f}), errors {report.errors} (at most {bar.max_errors})")
        for r in report.results:
            if not r.passed:
                print(f"  FAILED {r.id}: {r.error or ', '.join(n for n, s in r.scores.items() if s == 0)}")
        print("BAR MET" if not bar_missed else "BAR MISSED: " + "; ".join(bar_missed))
    if args.show:
        by_id = {c.id: c for c in cases}
        for r in report.results:
            if not r.passed:
                c = by_id[r.id]
                print(f"  {r.id}: input={c.input!r} expected={c.expected!r} answer={r.answer!r}", file=sys.stderr)
    return 0 if not bar_missed else 1


if __name__ == "__main__":
    sys.exit(main())
