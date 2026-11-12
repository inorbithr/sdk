"""The scorers: each takes the case and the solver's answer and returns 0 or 1.

A scorer is a pure function. Keep it deterministic: the same case and answer give the same
score, with no clock, no randomness and no network, so a change in the score is a change in
the solver. (A model that grades another model's answer is a solver of its own: label it a
stub in tests and give it a bar of its own.)
"""

from collections.abc import Callable

from .dataset import Case
from .solver import LABELS

Scorer = Callable[[Case, str], int]


def exact(case: Case, answer: str) -> int:
    """The answer is the expected label, ignoring case and surrounding space."""
    return int(answer.strip().lower() == case.expected.strip().lower())


def format_ok(case: Case, answer: str) -> int:
    """The answer is one bare label from the list and nothing else (no sentence, no full stop).

    A model that is right but chatty breaks the program that reads its answer, so it fails here."""
    return int(answer in LABELS)


SCORERS: dict[str, Scorer] = {"exact": exact, "format": format_ok}
