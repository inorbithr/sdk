"""The command line: text and JSON output, exit codes that mean something, and a version that
says which commit it was built from. Standard library only."""

import json
import shutil
import subprocess
from importlib import metadata
from pathlib import Path

VERSION = "0.1.0"
OK, FAILURE, USAGE = 0, 1, 2

HELP = """Usage: @@NAME@@ [--json] <command>

Commands:
  greet [NAME]   Say hello to NAME (default: world)
  version        Print the version and the commit it was built from

Options:
      --json     Machine-readable output
  -h, --help     Show this help

Exit codes: 0 success, 1 failure, 2 usage error.
"""


def build_commit() -> str:
    """The commit this was built from: the one a VCS install records (`pip install git+...`),
    else the checkout it runs in, else "unknown"."""
    try:
        direct = metadata.distribution("@@NAME@@").read_text("direct_url.json")
        commit = json.loads(direct or "{}").get("vcs_info", {}).get("commit_id", "")
        if commit:
            return commit[:12]
    except (metadata.PackageNotFoundError, ValueError):
        pass
    git = shutil.which("git")
    if git:
        try:
            out = subprocess.run(
                [git, "-C", str(Path(__file__).parent), "rev-parse", "--short=12", "HEAD"],
                capture_output=True, text=True, timeout=5, check=False,
            )
            if out.returncode == 0 and out.stdout.strip():
                return out.stdout.strip()
        except (OSError, subprocess.SubprocessError):
            pass
    return "unknown"


def _quote(s: str) -> str:
    # A hostile argument must not shape the error line: cut it and escape it.
    return repr(s if len(s) <= 40 else s[:40] + "...")


def _emit(out, value: dict) -> None:
    out.write(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n")


def _usage_error(err, msg: str) -> int:
    err.write(f"@@NAME@@: {msg}\nrun `@@NAME@@ --help` for the commands\n")
    return USAGE


def run(args: list[str], out, err, commit: str | None = None) -> int:
    """The whole program, with its streams as arguments so a test can read them."""
    as_json = False
    rest = []
    for a in args:
        if a == "--json":
            as_json = True
        elif a in ("-h", "--help"):
            out.write(HELP)
            return OK
        else:
            rest.append(a)
    if not rest:
        err.write(HELP)
        return USAGE
    cmd, tail = rest[0], rest[1:]
    if cmd == "greet":
        if len(tail) > 1:
            return _usage_error(err, "greet takes at most one name")
        name = tail[0].strip() if tail else "world"
        if not name or len(name) > 64:
            err.write("@@NAME@@: the name is 1 to 64 characters of text\n")
            return FAILURE
        msg = f"Hello, {name}"
        if as_json:
            _emit(out, {"message": msg})
        else:
            out.write(msg + "\n")
        return OK
    if cmd in ("version", "--version"):
        if tail:
            return _usage_error(err, "version takes no arguments")
        c = commit or build_commit()
        if as_json:
            _emit(out, {"name": "@@NAME@@", "version": VERSION, "commit": c})
        else:
            out.write(f"@@NAME@@ {VERSION} (commit {c})\n")
        return OK
    return _usage_error(err, f"unknown command {_quote(cmd)}")
