"""The loop. One model, a set of tools, and three limits you can read in one place:

- a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
- an allow-list: a tool that is not on it is refused, and its code never runs;
- approval: a tool marked `needs_approval` runs only when someone answers yes.

Every stop has a name (`Run.stop`), so a caller never has to guess why a run ended. The log
holds what happened (tool names, outcomes, counts), never a tool's arguments or results: those
can be customer data."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Callable, Protocol

# Why a run stopped.
DONE = "done"
BUDGET_STEPS = "budget_steps"
BUDGET_TOOL_CALLS = "budget_tool_calls"
BUDGET_TOKENS = "budget_tokens"
AWAITING_APPROVAL = "awaiting_approval"


@dataclass(frozen=True)
class Budget:
    max_steps: int = 8
    max_tool_calls: int = 4
    max_tokens: int = 4000


@dataclass(frozen=True)
class ToolCall:
    name: str
    args: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class Turn:
    """What the model answered: text (it is done) or a tool call, and the tokens it used."""

    text: str | None = None
    call: ToolCall | None = None
    tokens: int = 0


class Model(Protocol):
    """Anything with `next(history) -> Turn`. Wrap your provider here, signed in the way your
    provider's subscription allows; this template never holds a key."""

    def next(self, history: list[dict[str, Any]]) -> Turn: ...


@dataclass(frozen=True)
class Tool:
    name: str
    fn: Callable[[dict[str, Any]], Any]
    needs_approval: bool = False


@dataclass
class Run:
    stop: str
    text: str | None
    steps: int
    tool_calls: int
    tokens: int
    history: list[dict[str, Any]]


class Agent:
    def __init__(
        self,
        model: Model,
        tools: list[Tool],
        allow: set[str],
        budget: Budget = Budget(),
        approve: Callable[[ToolCall], bool | None] | None = None,
        log=None,
    ):
        self.model = model
        self.tools = {t.name: t for t in tools}
        self.allow = allow
        self.budget = budget
        self.approve = approve  # True runs it, False refuses it, None (no answer) stops the run
        self.log = log

    def _log(self, event: str, **fields: Any) -> None:
        if self.log is not None:
            self.log.write(json.dumps({"event": event, **fields}, sort_keys=True) + "\n")

    def run(self, task: str) -> Run:
        history: list[dict[str, Any]] = [{"role": "user", "content": task}]
        steps = tool_calls = tokens = 0

        def stop(reason: str, text: str | None = None) -> Run:
            self._log("stop", reason=reason, steps=steps, tool_calls=tool_calls, tokens=tokens)
            return Run(reason, text, steps, tool_calls, tokens, history)

        while True:
            if steps >= self.budget.max_steps:
                return stop(BUDGET_STEPS)
            turn = self.model.next(history)
            steps += 1
            if tokens + turn.tokens > self.budget.max_tokens:
                return stop(BUDGET_TOKENS)  # this turn is dropped: it is over the budget
            tokens += turn.tokens
            if turn.call is None:
                history.append({"role": "assistant", "content": turn.text or ""})
                return stop(DONE, turn.text)
            call = turn.call
            if tool_calls >= self.budget.max_tool_calls:
                return stop(BUDGET_TOOL_CALLS)
            tool = self.tools.get(call.name)
            if tool is None or call.name not in self.allow:
                self._log("refused", tool=call.name[:64], why="not_allowed")
                history.append({"role": "tool", "name": call.name, "error": "not_allowed"})
                continue
            if tool.needs_approval:
                answer = self.approve(call) if self.approve else None
                if answer is None:
                    self._log("approval", tool=call.name, answer="none")
                    return stop(AWAITING_APPROVAL)
                if not answer:
                    self._log("approval", tool=call.name, answer="no")
                    history.append({"role": "tool", "name": call.name, "error": "denied"})
                    continue
                self._log("approval", tool=call.name, answer="yes")
            result = tool.fn(call.args)
            tool_calls += 1
            self._log("tool", tool=call.name, ok=True)
            history.append({"role": "tool", "name": call.name, "result": result})
