"""Repository invariants that no linter checks. Run through `mise run repo:check`.

- Every third-party action is pinned to a full commit SHA with a version comment.
- Every workflow sets top-level `permissions`.
- No workflow uses `pull_request_target` or `workflow_run`.
- Agent instruction files stay under the size that keeps them effective (200 lines).
- Each language directory and cli/ has AGENTS.md, CLAUDE.md (importing it) and README.md.
- Every member of the cli Cargo workspace is in the release-please config's `cli` package
  (a `cli/Cargo.lock` version entry per crate) and carries the manifest's cli version, so a
  release bump never leaves a crate behind in the lockfile (`cargo build --locked` fails).
"""

import glob
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USES = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)(\s*#\s*(\S+))?", re.MULTILINE)
SHA = re.compile(r"@[0-9a-f]{40}$")
LANGS = ("go", "rust", "typescript", "python", "java", "csharp", "dart", "cli")
MAX_LINES = 200


def cli_release_problems() -> list[str]:
    """Every cli workspace member must be bumped by release-please with the cli version."""
    cli = ROOT / "cli"
    ws = tomllib.loads((cli / "Cargo.toml").read_text())["workspace"]
    ws_version = ws["package"]["version"]
    members: dict[str, str] = {}
    for pattern in ws["members"]:
        for d in sorted(glob.glob(str(cli / pattern))):
            manifest = Path(d) / "Cargo.toml"
            if not manifest.is_file():
                continue
            pkg = tomllib.loads(manifest.read_text())["package"]
            version = pkg["version"]
            members[pkg["name"]] = ws_version if isinstance(version, dict) else version

    config = json.loads((ROOT / "release-please-config.json").read_text())
    manifest_version = json.loads((ROOT / ".release-please-manifest.json").read_text())["cli"]
    listed: set[str] = set()
    for extra in config["packages"]["cli"]["extra-files"]:
        if isinstance(extra, dict) and extra.get("path") == "Cargo.lock":
            m = re.search(r"@\.name\.value==='([^']+)'", extra.get("jsonpath", ""))
            if m:
                listed.add(m.group(1))

    problems = []
    for name, version in sorted(members.items()):
        if name not in listed:
            problems.append(
                f"release-please-config.json: cli workspace crate {name} has no Cargo.lock entry in "
                f"packages.cli.extra-files; add {{ \"type\": \"toml\", \"path\": \"Cargo.lock\", "
                f"\"jsonpath\": \"$.package[?(@.name.value==='{name}')].version\" }} "
                f"or its version is not bumped and the locked release build fails"
            )
        if version != manifest_version:
            problems.append(
                f"cli crate {name} is at {version}, the release manifest's cli is {manifest_version}"
            )
    return problems


def main() -> int:
    problems: list[str] = cli_release_problems()

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

    # A published package carries the licence (Apache-2.0 section 4) and pkg.go.dev shows a
    # Go module's docs only with a licence in the module's own directory, so each published
    # package keeps an exact copy of the root LICENSE and NOTICE.
    for pkg in ("go", "rust", "typescript", "python"):
        for name in ("LICENSE", "NOTICE"):
            copy = ROOT / pkg / name
            if not copy.is_file() or copy.read_bytes() != (ROOT / name).read_bytes():
                problems.append(f"{pkg}/{name} must be an exact copy of the root {name}")

    for p in problems:
        print(p, file=sys.stderr)
    print(f"repo:check: {len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
