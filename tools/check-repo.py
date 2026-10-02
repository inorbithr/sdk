"""Repository invariants that no linter checks. Run through `mise run repo:check`.

- Every third-party action is pinned to a full commit SHA with a version comment.
- Every workflow sets top-level `permissions`.
- No workflow uses `pull_request_target` or `workflow_run`.
- Agent instruction files stay under the size that keeps them effective (200 lines).
- Each language directory and cli/ has AGENTS.md, CLAUDE.md (importing it) and README.md.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USES = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)(\s*#\s*(\S+))?", re.MULTILINE)
SHA = re.compile(r"@[0-9a-f]{40}$")
LANGS = ("go", "rust", "typescript", "python", "cli")
MAX_LINES = 200


def main() -> int:
    problems: list[str] = []

    for wf in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
        text = wf.read_text()
        rel = wf.relative_to(ROOT)
        for m in USES.finditer(text):
            ref, comment = m.group(1), m.group(3)
            if ref.startswith("./"):
                continue
            if not SHA.search(ref):
                problems.append(f"{rel}: {ref} is not pinned to a commit SHA")
            elif not comment:
                problems.append(f"{rel}: {ref} has no '# vX.Y.Z' comment")
        if not re.search(r"^permissions:", text, re.MULTILINE):
            problems.append(f"{rel}: no top-level permissions block")
        if re.search(r"^\s*(pull_request_target|workflow_run)\s*:", text, re.MULTILINE):
            problems.append(f"{rel}: pull_request_target and workflow_run are not allowed (ADR 0005)")

    instruction_files = [ROOT / "AGENTS.md", ROOT / "CLAUDE.md", *ROOT.glob("*/AGENTS.md"), *ROOT.glob(".claude/rules/*.md")]
    for f in instruction_files:
        n = len(f.read_text().splitlines())
        if n > MAX_LINES:
            problems.append(f"{f.relative_to(ROOT)}: {n} lines, keep it under {MAX_LINES}")

    for lang in LANGS:
        for name in ("AGENTS.md", "CLAUDE.md", "README.md"):
            if not (ROOT / lang / name).is_file():
                problems.append(f"{lang}/{name} is missing")
        claude = ROOT / lang / "CLAUDE.md"
        if claude.is_file() and "@AGENTS.md" not in claude.read_text():
            problems.append(f"{lang}/CLAUDE.md must import @AGENTS.md")

    for p in problems:
        print(p, file=sys.stderr)
    print(f"repo:check: {len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
