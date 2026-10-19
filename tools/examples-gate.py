"""The examples gate (RFC 0165 slice 5): a release needs `examples:run` green on its commit.

  attest            read examples-results.json and post the commit status `examples-run`
                    on its commit, only when it is a pass (see `verdict`).
  gate --ref REF    exit 1 unless REF's commit has the `examples-run` status `success`.

results file (written by `mise run examples:run`):
  {"schema": 1, "commit": "<40 hex>", "dirty": false, "results":
   [{"language": "go", "example": "whoami", "status": "pass|fail|skip"}]}
Several runs (Linux and the Mac, for Swift) are given as `attest --results a.json --results b.json`;
each must be for HEAD, clean and without a fail, and the languages are checked on their union.
A pass needs: the commit is HEAD, the tree was clean, no fail, and every language directory
under examples/ has at least one pass. Uses `gh` (GH_TOKEN: statuses: write to attest,
statuses: read to gate).
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTEXT = "examples-run"


def sh(*args: str) -> str:
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def verdict(res: dict, head: str, languages: set[str]) -> str | None:
    """None when the results pass, else the reason they do not."""
    if res.get("schema") != 1:
        return "schema: expected 1"
    if res.get("commit") != head:
        return f"results are for {res.get('commit')}, HEAD is {head}"
    if res.get("dirty"):
        return "the tree was dirty when the examples ran"
    rows = res.get("results") or []
    failed = [f"{r.get('language')}/{r.get('example')}" for r in rows if r.get("status") == "fail"]
    if failed:
        return "failed: " + ", ".join(failed)
    passed = {r.get("language") for r in rows if r.get("status") == "pass"}
    if missing := sorted(languages - passed):
        return "no passing example for: " + ", ".join(missing)
    return None


def merge(files: list[dict], head: str) -> tuple[dict | None, str | None]:
    """One results document from several runs (Linux and Mac), or the reason they cannot be one.

    Each file must itself pass the schema, commit, dirty and no-fail checks of `verdict` (no
    language requirement); then the rows are joined, so the language check runs on the union."""
    for i, f in enumerate(files):
        if why := verdict(f, head, set()):
            return None, f"results file {i + 1}: {why}"
    rows = [r for f in files for r in f.get("results") or []]
    return {"schema": 1, "commit": head, "dirty": False, "results": rows}, None


def repo() -> str:
    return sh("gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner")


def attest(paths: list[Path]) -> int:
    head = sh("git", "rev-parse", "HEAD")
    langs = {p.name for p in (ROOT / "examples").iterdir() if p.is_dir()}
    res, why = merge([json.loads(p.read_text()) for p in paths], head)
    if res is not None:
        why = verdict(res, head, langs)
    else:
        res = {"results": []}
    state = "failure" if why else "success"
    desc = (why or f"{len(res['results'])} examples passed against the live API")[:140]
    sh("gh", "api", f"repos/{repo()}/statuses/{head}", "-f", f"state={state}",
       "-f", f"context={CONTEXT}", "-f", f"description={desc}")
    print(f"{CONTEXT} {state} on {head}: {desc}")
    return 1 if why else 0


def gate(ref: str) -> int:
    sha = sh("gh", "api", f"repos/{repo()}/commits/{ref}", "-q", ".sha")
    states = sh("gh", "api", f"repos/{repo()}/commits/{sha}/statuses", "--paginate",
                "-q", f'[.[] | select(.context == "{CONTEXT}")] | .[0].state // "missing"')
    state = states.splitlines()[0] if states else "missing"
    print(f"{CONTEXT} on {sha} ({ref}): {state}")
    if state != "success":
        print(f"refused: run `mise run examples:run && mise run examples:attest` on {sha}", file=sys.stderr)
        return 1
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("attest")
    a.add_argument("--results", action="append", help="a results file; repeat for the Linux and the Mac run (default examples-results.json)")
    g = sub.add_parser("gate")
    g.add_argument("--ref", required=True)
    args = p.parse_args()
    return attest([Path(r) for r in args.results or [ROOT / "examples-results.json"]]) if args.cmd == "attest" else gate(args.ref)


if __name__ == "__main__":
    sys.exit(main())
