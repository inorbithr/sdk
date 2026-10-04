"""Whether a package's pinned next version allows breaking changes over its last tag.

`python3 tools/release-as.py PACKAGE TAG` prints `breaking` when release-please-config.json
pins the package's next version (`release-as`) and that version may break the API of the
tagged one under SemVer as Cargo reads it (a new major, or a new minor before 1.0);
otherwise it prints nothing. The `<lang>:api` tasks use it: a pinned breaking release is
declared in review, so its API checks report instead of failing (docs/releasing.md).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def parts(version: str) -> tuple[int, int, int]:
    core = version.lstrip("v").split("-", 1)[0].split("+", 1)[0]
    major, minor, patch = (int(p) for p in core.split("."))
    return major, minor, patch


def main() -> int:
    package, tag = sys.argv[1], sys.argv[2]
    config = json.loads((ROOT / "release-please-config.json").read_text())
    pinned = config["packages"].get(package, {}).get("release-as")
    if not pinned:
        return 0
    old, new = parts(tag.rsplit("/", 1)[-1]), parts(pinned)
    if new[0] > old[0] or (old[0] == 0 and new[:2] > old[:2]):
        print("breaking")
    return 0


if __name__ == "__main__":
    sys.exit(main())
