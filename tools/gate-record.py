#!/usr/bin/env python3
"""The gate record `mise run ci:changed` leaves on a pass (PRD 0221, item 2).

The record is core's gate store, byte for byte (inorbithr/core `devops/ci/gates.py`:
`store`, `clean_tree`, `key`, `record_path`, `write`, `mirror`), so one reader serves
both repositories: the PR guard that requires a passing `ci:changed` for HEAD's clean
tree before a ready pull request, and `GATES_MIRROR` copies to the build server.

    $XDG_STATE_HOME (or ~/.local/state) /tbd/gates/<HEAD^{tree}>/<key>.json

where <key> is the first 32 hex digits of sha256("1\\0$CI\\0mise\\0run\\0ci:changed").
Only a clean checkout counts: no modified, staged or untracked file before the run and
the same clean tree after it.

    gate-record.py tree                  HEAD's tree when the checkout is clean, else nothing
    gate-record.py write <tree> <start>  after a pass: record it when the checkout is still
                                         clean at <tree>; <start> is the run's start, in
                                         seconds since the epoch. Prints one line either way.
"""

from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

SCHEMA = "1"
CMD = ["mise", "run", "ci:changed"]
LABEL = "ci:changed"


def store() -> Path:
    state = os.environ.get("XDG_STATE_HOME") or str(Path.home() / ".local/state")
    return Path(state) / "tbd" / "gates"


def _git(*args: str, cwd: Path) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def clean_tree(cwd: Path) -> str | None:
    """HEAD's tree hash when the checkout holds exactly HEAD, else None."""
    try:
        if _git("status", "--porcelain", "--untracked-files=normal", cwd=cwd):
            return None
        return _git("rev-parse", "HEAD^{tree}", cwd=cwd)
    except subprocess.CalledProcessError:
        return None


def key(cmd: list[str]) -> str:
    text = "\0".join([SCHEMA, os.environ.get("CI", ""), *cmd])
    return hashlib.sha256(text.encode()).hexdigest()[:32]


def record_path(tree: str, cmd: list[str]) -> Path:
    return store() / tree / f"{key(cmd)}.json"


def write(tree: str, cmd: list[str], seconds: float, cwd: Path, label: str) -> Path:
    path = record_path(tree, cmd)
    rec = {
        "tree": tree,
        "cmd": cmd,
        "commit": _git("rev-parse", "HEAD", cwd=cwd),
        "passed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "seconds": round(seconds, 1),
        "by": os.environ.get("ROLL_WHAT") or label,
        "host": socket.gethostname(),
        "checkout": str(cwd),
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(f".{os.getpid()}.tmp")
    tmp.write_text(json.dumps(rec, indent=1) + "\n")
    tmp.replace(path)
    mirror(path)
    return path


def mirror(path: Path) -> None:
    """Copy a pass to another host's record store (`GATES_MIRROR=<ssh host>`). Best
    effort: a copy that fails only means the check runs again there."""
    host = os.environ.get("GATES_MIRROR", "").strip()
    if not host:
        return
    rel = f".local/state/tbd/gates/{path.parent.name}"
    try:
        subprocess.run(
            ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host,
             f"mkdir -p {rel} && cat > {rel}/{path.name}"],
            input=path.read_bytes(), check=True, capture_output=True, timeout=20,
        )
    except (OSError, subprocess.SubprocessError) as err:
        print(f"gates: could not mirror the pass to {host} ({err})", file=sys.stderr)


def main(argv: list[str]) -> int:
    cwd = Path.cwd()
    if argv == ["tree"]:
        print(clean_tree(cwd) or "")
        return 0
    if len(argv) == 3 and argv[0] == "write":
        before, start = argv[1], float(argv[2])
        if not before:
            print(f"{LABEL}: no gate record: the checkout had uncommitted or untracked "
                  "changes when the run started; commit them and run again")
            return 0
        if clean_tree(cwd) != before:
            print(f"{LABEL}: no gate record: the checkout changed during the run "
                  "(a task left a file, or HEAD moved)")
            return 0
        try:
            path = write(before, CMD, time.time() - start, cwd, LABEL)
        except (OSError, subprocess.CalledProcessError) as err:
            print(f"{LABEL}: could not record the pass ({err})", file=sys.stderr)
            return 0
        print(f"{LABEL}: recorded the pass for tree {before[:12]} ({path})")
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
