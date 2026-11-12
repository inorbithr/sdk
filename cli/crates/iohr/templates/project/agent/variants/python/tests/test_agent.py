"""The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
with a named reason; a tool outside the allow-list is refused and never runs; a tool that
needs approval does not run without an answer."""

import io
import unittest

from @@MODULE@@.agent import (
    AWAITING_APPROVAL, BUDGET_STEPS, BUDGET_TOKENS, BUDGET_TOOL_CALLS, DONE,
    Agent, Budget, Tool, ToolCall, Turn,
)
from @@MODULE@@.model import ScriptedModel, demo
from @@MODULE@@.tools import ALLOW, TOOLS


def ask(turns, *, tools=TOOLS, allow=ALLOW, budget=Budget(), approve=None, log=None):
    return Agent(ScriptedModel(turns), tools, allow, budget, approve, log).run("task")


class Loop(unittest.TestCase):
    def test_a_scripted_run_finishes_within_the_budget(self):
        run = Agent(demo(), TOOLS, ALLOW, Budget()).run("add 2 and 3")
        self.assertEqual((run.stop, run.text, run.tool_calls), (DONE, "2 + 3 = 5", 1))
        self.assertLessEqual(run.steps, Budget().max_steps)
        self.assertEqual(run.history[1]["result"], 5)

    def test_a_run_that_would_pass_a_budget_stops_and_names_it(self):
        forever = [Turn(call=ToolCall("add", {"a": 1, "b": 1}), tokens=10)] * 50
        self.assertEqual(ask(forever, budget=Budget(max_tool_calls=3)).stop, BUDGET_TOOL_CALLS)
        self.assertEqual(ask(forever, budget=Budget(max_steps=2, max_tool_calls=9)).stop, BUDGET_STEPS)
        heavy = [Turn(call=ToolCall("add", {"a": 1, "b": 1}), tokens=3000)] * 5
        run = ask(heavy, budget=Budget(max_tokens=4000, max_tool_calls=9))
        self.assertEqual(run.stop, BUDGET_TOKENS)
        self.assertLessEqual(run.tokens, 4000)

    def test_a_tool_outside_the_allow_list_is_refused_and_never_runs(self):
        ran = []
        spy = Tool("shell", lambda args: ran.append(args))
        run = ask(
            [Turn(call=ToolCall("shell", {"cmd": "rm -rf /"})), Turn(text="ok")],
            tools=TOOLS + [spy],
        )
        self.assertEqual(ran, [])
        self.assertEqual(run.history[1]["error"], "not_allowed")
        self.assertEqual(run.stop, DONE)
        self.assertEqual(run.tool_calls, 0)

    def test_a_tool_that_needs_approval_does_not_run_without_an_answer(self):
        ran = []
        tools = [Tool("delete_note", lambda args: ran.append(args), needs_approval=True)]
        call = Turn(call=ToolCall("delete_note", {"id": "n1"}))
        allow = {"delete_note"}
        self.assertEqual(ask([call], tools=tools, allow=allow).stop, AWAITING_APPROVAL)
        self.assertEqual(ask([call], tools=tools, allow=allow, approve=lambda c: None).stop, AWAITING_APPROVAL)
        denied = ask([call, Turn(text="ok")], tools=tools, allow=allow, approve=lambda c: False)
        self.assertEqual((denied.stop, denied.history[1]["error"]), (DONE, "denied"))
        self.assertEqual(ran, [])
        yes = ask([call, Turn(text="ok")], tools=tools, allow=allow, approve=lambda c: True)
        self.assertEqual((yes.stop, len(ran)), (DONE, 1))

    def test_the_log_holds_what_happened_and_never_a_tool_argument(self):
        log = io.StringIO()
        ask([Turn(call=ToolCall("add", {"a": 424242, "b": 1})), Turn(text="secret-answer")], log=log)
        text = log.getvalue()
        self.assertIn('"event": "tool"', text)
        self.assertNotIn("424242", text)
        self.assertNotIn("secret-answer", text)


if __name__ == "__main__":
    unittest.main()
