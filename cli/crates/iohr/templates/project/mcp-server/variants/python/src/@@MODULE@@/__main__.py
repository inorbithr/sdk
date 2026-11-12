"""Run the server over stdio, the way an MCP host starts it: `python -m @@MODULE@@`."""

from .server import mcp


def main() -> None:
    # stdout carries the protocol and nothing else; the SDK logs to stderr.
    mcp.run("stdio")


if __name__ == "__main__":
    main()
