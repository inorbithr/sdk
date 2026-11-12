"""The solver: the thing under test. It turns a case's input into an answer.

The one here is a STUB: a fixed list of keyword rules, labelled as one everywhere it shows up
(`stub: true` in the report, a `stub-` version). It exists so the suite runs offline, is free
and gives the same answer every time. To grade a real model, write a class with the same
two attributes and a `solve(case)` method that calls your provider, sign in the way your
provider's subscription allows and keep the credential in your secret store, never in this
folder. A solver that raises is counted as an error, never as a pass.
"""

from typing import Protocol

from .dataset import Case

LABELS = ("billing", "bug", "feature_request", "account", "other")

# First match wins, in this order.
_RULES: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("billing", ("charged", "refund", "invoice", "payment", "card")),
    ("bug", ("crash", "error", "blank screen")),
    ("feature_request", ("would be great", "add an option", "feature")),
    ("account", ("password", "account")),
)


class Solver(Protocol):
    name: str
    stub: bool

    def solve(self, case: Case) -> str: ...


class StubSolver:
    """Keyword rules. Not a model: nothing is learned, nothing is called."""

    name = "stub-keyword-rules-v1"
    stub = True

    def solve(self, case: Case) -> str:
        text = case.input.lower()
        for label, words in _RULES:
            if any(w in text for w in words):
                return label
        return "other"
