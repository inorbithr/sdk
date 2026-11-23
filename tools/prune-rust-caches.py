#!/usr/bin/env python3
"""Delete superseded Rust caches, keeping the newest entry per prefix (ADR 0014, 2026-11-23).

Swatinem/rust-cache keys end in the lockfile hash: `v0-rust-1.99.0-cli-Linux-x64-ab1d8b4d-d7b2f9a9`.
Every lockfile change saves a new entry and leaves the old one beside it, so the repository
passes GitHub's 10 GB limit and eviction (least recently used first) removes a cache a
merge-queue leg needs. The prefix is the key without its last `-<8 hex>` group.

    prune-rust-caches.py            delete (needs GH_TOKEN with actions: write, GITHUB_REPOSITORY)
    prune-rust-caches.py --dry-run  print what would go

Only entries saved by refs/heads/main whose key starts with `v0-rust` are touched.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys

PREFIX = "v0-rust"
SUFFIX = re.compile(r"-[0-9a-f]{8}$")


def superseded(caches: list[dict]) -> list[dict]:
    """The entries to delete: all but the newest per prefix, from main's rust-cache keys."""
    groups: dict[str, list[dict]] = {}
    for c in caches:
        if c.get("ref") != "refs/heads/main" or not c["key"].startswith(PREFIX):
            continue
        if not SUFFIX.search(c["key"]):
            continue
        groups.setdefault(SUFFIX.sub("", c["key"]), []).append(c)
    out = []
    for entries in groups.values():
        entries.sort(key=lambda c: c["created_at"], reverse=True)
        out.extend(entries[1:])
    return out


def gh(*args: str) -> str:
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    dry = "--dry-run" in sys.argv[1:]
    pages = gh("api", "--paginate", "--slurp", f"repos/{repo}/actions/caches?per_page=100&ref=refs/heads/main")
    caches = [c for page in json.loads(pages) for c in page["actions_caches"]]
    old = superseded(caches)
    freed = 0
    for c in old:
        print(f"{'would delete' if dry else 'delete'} {c['key']} ({c['size_in_bytes'] / 1e9:.2f} GB, {c['created_at']})")
        if not dry:
            gh("api", "-X", "DELETE", f"repos/{repo}/actions/caches/{c['id']}")
        freed += c["size_in_bytes"]
    print(f"{len(old)} of {len(caches)} main caches superseded, {freed / 1e9:.2f} GB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
