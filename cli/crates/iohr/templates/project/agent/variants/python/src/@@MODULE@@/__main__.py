"""`python -m @@MODULE@@ "your task"`: runs the loop with the scripted model and prints the
outcome, the stop reason and what it used."""

import sys

from .agent import Agent, Budget
from .model import STUB, demo
from .tools import ALLOW, TOOLS


def main() -> int:
    task = " ".join(sys.argv[1:]) or "add 2 and 3"
    run = Agent(demo(), TOOLS, ALLOW, Budget(), log=sys.stderr).run(task)
    print(f"[{STUB}]")
    print(run.text if run.text is not None else f"(stopped: {run.stop})")
    print(f"stop={run.stop} steps={run.steps} tool_calls={run.tool_calls} tokens={run.tokens}")
    return 0 if run.stop == "done" else 1


if __name__ == "__main__":
    sys.exit(main())
