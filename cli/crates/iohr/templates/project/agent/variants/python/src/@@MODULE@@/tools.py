"""Your tools. Two examples: one that is safe to run, one that changes something and so needs
an answer first. Replace them; keep `needs_approval` on anything that writes, sends or deletes."""

from .agent import Tool


def add(args: dict) -> int:
    return int(args["a"]) + int(args["b"])


def delete_note(args: dict) -> str:
    # An example of a tool that changes something: it only pretends to here.
    return f"deleted {args['id']}"


TOOLS = [
    Tool("add", add),
    Tool("delete_note", delete_note, needs_approval=True),
]
# Only these may run. A tool the model asks for that is not here is refused.
ALLOW = {"add"}
