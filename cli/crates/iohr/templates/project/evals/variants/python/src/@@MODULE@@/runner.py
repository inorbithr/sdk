"""Run a solver over the cases, score each answer and check the result against the bar."""

import tomllib
from dataclasses import dataclass, field
from pathlib import Path

from .dataset import Case, SuiteError
from .scorers import SCORERS, Scorer
from .solver import Solver


@dataclass(frozen=True)
class Bar:
    min_pass_rate: float
    max_errors: int
    scorers: dict[str, float]


@dataclass
class Result:
    id: str
    answer: str | None  # None when the solver raised
    scores: dict[str, int]
    error: str | None = None

    @property
    def passed(self) -> bool:
        return self.error is None and all(s == 1 for s in self.scores.values())


@dataclass
class Report:
    solver: str
    stub: bool
    results: list[Result] = field(default_factory=list)

    @property
    def pass_rate(self) -> float:
        return sum(r.passed for r in self.results) / len(self.results)

    @property
    def errors(self) -> int:
        return sum(r.error is not None for r in self.results)

    def mean(self, scorer: str) -> float:
        return sum(r.scores[scorer] for r in self.results) / len(self.results)


def load_bar(path: Path, scorers: dict[str, Scorer] = SCORERS) -> Bar:
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except OSError as e:
        raise SuiteError(f"cannot read the bar {path}: {e.strerror}") from e
    except tomllib.TOMLDecodeError as e:
        raise SuiteError(f"{path.name}: not valid TOML ({e})") from e
    bar = raw.get("bar")
    if not isinstance(bar, dict):
        raise SuiteError(f"{path.name}: the [bar] table is missing")
    try:
        rate = float(bar["min_pass_rate"])
        errors = int(bar["max_errors"])
        per = {str(k): float(v) for k, v in bar["scorers"].items()}
    except (KeyError, TypeError, ValueError, AttributeError) as e:
        raise SuiteError(f"{path.name}: [bar] needs min_pass_rate, max_errors and [bar.scorers] ({e!r})") from e
    if not 0 <= rate <= 1 or any(not 0 <= v <= 1 for v in per.values()):
        raise SuiteError(f"{path.name}: every rate is a number from 0 to 1")
    if errors < 0:
        raise SuiteError(f"{path.name}: max_errors cannot be negative")
    unguarded = sorted(set(scorers) - set(per))
    if unguarded:
        raise SuiteError(f"{path.name}: no bar for the scorer(s) {', '.join(unguarded)}; a scorer with no bar guards nothing")
    unknown = sorted(set(per) - set(scorers))
    if unknown:
        raise SuiteError(f"{path.name}: a bar for {', '.join(unknown)}, which is not a scorer")
    return Bar(rate, errors, per)


def evaluate(cases: list[Case], solver: Solver, scorers: dict[str, Scorer] = SCORERS) -> Report:
    report = Report(solver=solver.name, stub=solver.stub)
    for case in cases:
        try:
            answer = solver.solve(case)
            report.results.append(Result(case.id, answer, {n: s(case, answer) for n, s in scorers.items()}))
        except Exception as e:  # a broken solver is an error case, never a pass
            report.results.append(Result(case.id, None, {n: 0 for n in scorers}, error=type(e).__name__))
    return report


def missed(report: Report, bar: Bar) -> list[str]:
    """Every line of the bar the report does not reach; empty means the bar is met."""
    out: list[str] = []
    if report.pass_rate < bar.min_pass_rate:
        out.append(f"pass rate {report.pass_rate:.3f} is below {bar.min_pass_rate:.3f}")
    if report.errors > bar.max_errors:
        out.append(f"{report.errors} error(s), at most {bar.max_errors} allowed")
    for name, minimum in bar.scorers.items():
        if report.mean(name) < minimum:
            out.append(f"scorer {name}: mean {report.mean(name):.3f} is below {minimum:.3f}")
    return out
