"""The server: one tool and one resource. Replace them with yours; keep the rules in the test.

Every tool needs a description, an annotation that says whether it only reads or can change
things, and inputs with a bound (a maximum length, a range). `tests/test_server.py` fails when
one is missing, so a tool a host will let a model call is never left unlabelled.
"""

from typing import Annotated

from mcp.server import MCPServer
from mcp.types import ToolAnnotations
from pydantic import BaseModel, Field

mcp = MCPServer(
    "@@NAME@@",
    version="0.1.0",
    instructions="Counts the words and characters in a text. It reads nothing else and changes nothing.",
)


class Count(BaseModel):
    """The structured result: a host gets this as `structuredContent`, checked against the schema."""

    words: int
    characters: int


@mcp.tool(
    annotations=ToolAnnotations(
        read_only_hint=True,
        destructive_hint=False,
        idempotent_hint=True,
        open_world_hint=False,
    )
)
def word_count(
    text: Annotated[str, Field(max_length=10_000, description="The text to count.")],
) -> Count:
    """Count the words and the characters in a text (at most 10,000 characters)."""
    return Count(words=len(text.split()), characters=len(text))


@mcp.resource("about://server")
def about() -> str:
    """What this server is and what it can do."""
    return "@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process."
