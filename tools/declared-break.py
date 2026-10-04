"""Whether a breaking change is declared for a package since its last release tag.

`python3 tools/declared-break.py PATH TAG` prints `breaking` when a commit since TAG that
touches PATH is marked breaking the Conventional Commits way (`type!:` or `type(scope)!:`
in the subject, or a `BREAKING CHANGE:` footer); otherwise it prints nothing. The
`<lang>:api` tasks use it: a declared break is what release-please turns into the next
breaking version (a minor before 1.0, docs/releasing.md), so the API check reports it
instead of failing on it.
"""

from __future__ import annotations

import re
import subprocess
import sys

SUBJECT = re.compile(r"^[a-z]+(\([^)]*\))?!:")
FOOTER = re.compile(r"^BREAKING[ -]CHANGE:", re.MULTILINE)


def main() -> int:
    path, tag = sys.argv[1], sys.argv[2]
    log = subprocess.run(
        ["git", "log", "--format=%B%x00", f"{tag}..HEAD", "--", path],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    for message in filter(None, (m.strip() for m in log.split("\x00"))):
        if SUBJECT.match(message) or FOOTER.search(message):
            print("breaking")
            break
    return 0


if __name__ == "__main__":
    sys.exit(main())
