"""The dataset: one JSON object per line, each with an `id`, an `input` and the `expected` answer.

Loading is strict, so a broken dataset fails loudly instead of quietly scoring fewer cases.
"""

import json
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DATASET = ROOT / "datasets" / "tickets.jsonl"
DEFAULT_BAR = ROOT / "thresholds.toml"


class SuiteError(Exception):
    """The dataset or the bar is wrong. The run stops and exits 2: nothing was graded."""


@dataclass(frozen=True)
class Case:
    id: str
    input: str
    expected: str


def load_cases(path: Path) -> list[Case]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as e:
        raise SuiteError(f"cannot read the dataset {path}: {e.strerror}") from e
    cases: list[Case] = []
    seen: set[str] = set()
    for number, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as e:
            raise SuiteError(f"{path.name} line {number}: not JSON ({e.msg})") from e
        if not isinstance(row, dict):
            raise SuiteError(f"{path.name} line {number}: expected an object")
        for key in ("id", "input", "expected"):
            if not isinstance(row.get(key), str) or not row[key].strip():
                raise SuiteError(f"{path.name} line {number}: `{key}` must be a non-empty string")
        if row["id"] in seen:
            raise SuiteError(f"{path.name} line {number}: the id `{row['id']}` is used twice")
        seen.add(row["id"])
        cases.append(Case(row["id"], row["input"], row["expected"]))
    if not cases:
        raise SuiteError(f"{path.name} has no cases: an empty suite passes nothing")
    return cases
