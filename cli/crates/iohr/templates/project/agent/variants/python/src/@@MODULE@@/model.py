"""A scripted model: it answers from a list, so a run is repeatable and costs nothing. It is
a stub, labelled as one everywhere it prints. To use a real model, write a class with
`next(history) -> Turn` that calls your provider and returns its text or tool call and the
tokens it used; the loop does not change."""

from .agent import ToolCall, Turn

STUB = "stub: scripted model, no real model was called"


class ScriptedModel:
    def __init__(self, turns: list[Turn]):
        self._turns = list(turns)

    def next(self, history: list[dict]) -> Turn:
        if self._turns:
            return self._turns.pop(0)
        return Turn(text="(the script has ended)")


def demo() -> ScriptedModel:
    return ScriptedModel([
        Turn(call=ToolCall("add", {"a": 2, "b": 3}), tokens=40),
        Turn(text="2 + 3 = 5", tokens=20),
    ])
