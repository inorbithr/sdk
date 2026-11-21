#!/usr/bin/env python3
"""Name, in one place, every check a pull request will fail (PRD 0221 item 5, ADR 0019).

Three things, each from the rule that decides it rather than a copy of it:

- the title, against the types and scopes in .github/workflows/pr-title.yml (the list the
  pr-title check itself passes to action-semantic-pull-request);
- the body's document lines the merge guard reads: `PRD:`, `Implements: RFC`, `ADR:`, a
  `Study:` line for a fix, and `Monitors:`. The line rules mirror core's
  crates/coordinator/src/document_lines.rs (5073e3da, 2026-10-23); the Study-for-fix rule is
  the plugin's approval_docs_guard (ai-kit). `PRD: 0182` counts, `PRD: PRD 0182` does not;
- the packages and legs the change runs, from `tools/ci-changed.sh --dry-run`, the same path
  rules `mise run ci:changed` uses.

Locally (`mise run pr:preflight`) it reads the open pull request through `gh`, or the HEAD
commit, prints the report and exits 1 on a finding. In CI (`--github`) it writes the report to
the job summary and one error annotation per finding, and always exits 0: it is advisory.
`--comment FILE` creates or edits the one marked comment on the pull request (CI, same-repo
pull requests only).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PR_TITLE_YML = ".github/workflows/pr-title.yml"
MARKER = "<!-- pr-preflight -->"

# Title: Conventional Commits `type(scope)!: subject`, the shape approval_docs_guard checks.
CC_TITLE = re.compile(r"^(?P<type>[A-Za-z]+)(?:\((?P<scope>[^()\n]*)\))?!?: \S")
FIX_TITLE = re.compile(r"(?<![\w-])fix(\([^)\n]*\))?!?:", re.I)

# Document lines, as core's document_lines.rs reads them: at the start of a line, optionally
# after `Implements:`, `Part of:` or `Touches:`. Prose ("this follows RFC 0171") does not count.
LINE = re.compile(
    r"(?im)^\s*(?:(?:implements|part of|touches)\s*:\s*)?(PRD|RFC|ADR)[\s:#_-]*(\d{1,4})(\.\d{1,2})?\b")
STUDY = re.compile(
    r"(?im)^\s*(?:(?:implements|part of|touches)\s*:\s*)?(Study)[\s:#_-]*(\d{1,4})(\.\d{1,2})?\b")
DOUBLED = re.compile(r"(?im)^\s*(?:implements\s*:\s*)?(PRD|RFC|ADR|Study)\s*:\s*(PRD|RFC|ADR|Study)\b.*$")
MONITORS = re.compile(r"(?i)^\s*(?:[-*]\s*)?monitors\s*:\s*(.*?)\s*$")
NONE = re.compile(r"(?i)^none(?:$|[\s(:–—-])(.*)$", re.S)
LINE_FOR = {"PRD": "PRD: <n>", "RFC": "Implements: RFC <n>", "ADR": "ADR: <n>", "Study": "Study: <n>"}


def yaml_block(text: str, key: str) -> list[str] | None:
    """The words of a `key: |` block scalar (each line one or more words), or None."""
    lines = text.splitlines()
    for i, line in enumerate(lines):
        m = re.match(rf"^(\s*){key}:\s*\|[-+]?\s*$", line)
        if not m:
            continue
        words: list[str] = []
        for nxt in lines[i + 1:]:
            if nxt.strip() and len(nxt) - len(nxt.lstrip()) <= len(m.group(1)):
                break
            words += nxt.split("#", 1)[0].split()
        return words
    return None


def title_rules(text: str) -> tuple[list[str], list[str] | None, bool]:
    """(types, scopes or None for any, scope required) from pr-title.yml's text."""
    types = yaml_block(text, "types")
    if not types:
        raise SystemExit(f"pr-preflight: no `types: |` list in {PR_TITLE_YML}")
    req = re.search(r"^\s*requireScope:\s*['\"]?true\b", text, re.M | re.I)
    return types, yaml_block(text, "scopes"), bool(req)


def check_title(title: str, rules: tuple[list[str], list[str] | None, bool]) -> list[str]:
    types, scopes, require = rules
    t = title.strip()
    m = CC_TITLE.match(t)
    if not m:
        return [f"title {t!r} is not `type(scope): subject` (pr-title)"]
    out = []
    if m.group("type") not in types:
        out.append(f"title type `{m.group('type')}` is not one of: {', '.join(types)} (pr-title)")
    given = [s.strip() for s in m.group("scope").split(",")] if m.group("scope") is not None else []
    if m.group("scope") is not None and not all(given):
        out.append("title has an empty scope `()` (pr-title)")
    elif require and not given:
        out.append("title needs a scope (pr-title)")
    elif scopes is not None:
        bad = [s for s in given if s not in scopes]
        if bad:
            out.append(f"title scope `{', '.join(bad)}` is not one of: {', '.join(scopes)}, or none (pr-title)")
    return out


def _filled(value: str) -> bool:
    v = value.strip()
    return bool(v) and not (v.startswith("<") and v.endswith(">"))


def _reason_of(rest: str) -> str:
    r = rest.strip()
    if r.startswith("(") and r.endswith(")"):
        r = r[1:-1]
    return r.strip().lstrip("-:–—").strip()


def monitors_problem(body: str) -> str | None:
    """None when the body declares monitors or says `none (<reason>)`; else what is wrong."""
    for line in body.splitlines():
        m = MONITORS.match(line)
        if not m or not _filled(m.group(1)):
            continue
        n = NONE.match(m.group(1))
        if n is None:
            return None
        reason = _reason_of(n.group(1))
        if _filled(reason):
            return None
        return "body has `Monitors: none` without a reason: say why nothing watches it (merge guard)"
    return "body lacks a `Monitors:` line (monitor ids or checks file paths, or `none (<reason>)`) (merge guard)"


def is_fix(title: str, branch: str = "") -> bool:
    if FIX_TITLE.search(title or ""):
        return True
    return (branch or "").lower().startswith("fix/") and not re.match(r"\s*\w+(\([^)\n]*\))?!?:", title or "")


def check_body(body: str, title: str, branch: str = "") -> list[str]:
    body = body or ""
    found = {m.group(1).upper() for m in LINE.finditer(body)}
    if STUDY.search(body):
        found.add("Study")
    need = ["PRD", "RFC", "ADR"] + (["Study"] if is_fix(title, branch) else [])
    out, doubled = [], set()
    for m in DOUBLED.finditer(body):
        doubled.add(m.group(1).upper())
        out.append(f"body line `{m.group(0).strip()}` names the kind twice; write `{m.group(1)}: <n>`, "
                   f"for example `PRD: 0182` (merge guard)")
    for kind in need:
        if kind.upper() not in {k.upper() for k in found} | doubled:
            why = ", the title is a fix" if kind == "Study" else ""
            out.append(f"body lacks the line `{LINE_FOR[kind]}` (at the start of a line{why}) (merge guard)")
    problem = monitors_problem(body)
    if problem:
        out.append(problem)
    return out


def legs(base: str) -> tuple[list[str], str]:
    """(tasks, raw output) of ci-changed.sh's dry run against origin/<base>."""
    env = dict(os.environ, CI_CHANGED_BASE=f"origin/{base}")
    p = subprocess.run(["bash", str(ROOT / "tools/ci-changed.sh"), "--dry-run"], cwd=ROOT, env=env,
                       capture_output=True, text=True)
    raw = (p.stdout + p.stderr).strip()
    if p.returncode != 0:
        return [], raw or f"ci-changed.sh exited {p.returncode}"
    last = [ln for ln in p.stdout.splitlines() if ln.startswith("ci:changed: ") and "changed file" not in ln]
    return (last[-1][len("ci:changed: "):].split() if last else []), raw


def report(title: str, body: str, base: str, branch: str = "", run_legs: bool = True) -> tuple[list[str], str]:
    rules = title_rules((ROOT / PR_TITLE_YML).read_text())
    findings = check_title(title, rules) + check_body(body, title, branch)
    md = [MARKER, "## PR preflight", ""]
    if findings:
        md.append(f"This pull request will fail {len(findings)} check(s):")
        md += [f"- {f}" for f in findings]
    else:
        md.append("Title and document lines pass.")
    if run_legs:
        tasks, raw = legs(base)
        langs = sorted({t.split(":")[0] for t in tasks if t.endswith(":check") and t.count(":") == 1}
                       - {"repo", "cli", "ui", "examples"})
        md += ["", f"Legs this change runs (`tools/ci-changed.sh --dry-run`, base `origin/{base}`):"]
        md.append(f"- packages: {', '.join(langs) or 'none'}")
        md += ["", "```", raw, "```"]
    md += ["", "Advisory: this check never fails the pull request. Run `mise run pr:preflight` locally."]
    return findings, "\n".join(md) + "\n"


def escape(s: str) -> str:
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def local_pr() -> tuple[str, str, str, str]:
    """(title, body, base, branch) of this branch's pull request, else of the HEAD commit."""
    branch = subprocess.run(["git", "branch", "--show-current"], cwd=ROOT, capture_output=True,
                            text=True).stdout.strip()
    try:
        p = subprocess.run(["gh", "pr", "view", "--json", "title,body,baseRefName"], cwd=ROOT,
                           capture_output=True, text=True, timeout=30)
        if p.returncode == 0:
            d = json.loads(p.stdout)
            return d["title"], d["body"] or "", d["baseRefName"] or "main", branch
    except (OSError, subprocess.TimeoutExpired, ValueError, KeyError):
        pass
    msg = subprocess.run(["git", "log", "-1", "--format=%B"], cwd=ROOT, capture_output=True,
                         text=True).stdout
    title, _, body = msg.partition("\n")
    return title, body, "main", branch


def api(method: str, url: str, token: str, data: dict | None = None):
    req = urllib.request.Request(url, method=method, data=json.dumps(data).encode() if data else None,
                                 headers={"Authorization": f"Bearer {token}",
                                          "Accept": "application/vnd.github+json",
                                          "X-GitHub-Api-Version": "2022-11-28"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read() or b"null")


def comment(path: str) -> int:
    """Create or edit the one comment that starts with MARKER. Never fails the job."""
    token, repo, pr = os.environ.get("GH_TOKEN"), os.environ.get("REPO"), os.environ.get("PR_NUMBER")
    api_url = os.environ.get("GITHUB_API_URL", "https://api.github.com")
    if not (token and repo and pr and pr.isdigit() and re.fullmatch(r"[\w.-]+/[\w.-]+", repo)):
        print("::warning title=pr-preflight::no token or pull request; summary only")
        return 0
    text = Path(path).read_text()
    clear = "will fail" not in text
    try:
        mine = None
        for page in range(1, 11):
            got = api("GET", f"{api_url}/repos/{repo}/issues/{pr}/comments?per_page=100&page={page}", token)
            mine = next((c for c in got if (c.get("body") or "").startswith(MARKER)), None)
            if mine or len(got) < 100:
                break
        if mine:
            if mine.get("body") != text:
                api("PATCH", f"{api_url}/repos/{repo}/issues/comments/{mine['id']}", token, {"body": text})
        elif not clear:
            api("POST", f"{api_url}/repos/{repo}/issues/{pr}/comments", token, {"body": text})
    except (urllib.error.URLError, OSError, ValueError) as e:
        print(f"::warning title=pr-preflight::could not write the comment: {e}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--title")
    ap.add_argument("--body-file")
    ap.add_argument("--base")
    ap.add_argument("--github", action="store_true", help="CI: read PR_TITLE, PR_BODY, PR_BASE, PR_BRANCH")
    ap.add_argument("--out", help="also write the report here (for the comment step)")
    ap.add_argument("--comment", metavar="FILE", help="create or edit the marked comment from FILE")
    a = ap.parse_args(argv)
    if a.comment:
        return comment(a.comment)
    if a.github:
        title, body = os.environ.get("PR_TITLE", ""), os.environ.get("PR_BODY", "")
        base, branch = os.environ.get("PR_BASE") or "main", os.environ.get("PR_BRANCH", "")
    elif a.title is not None:
        title, base, branch = a.title, a.base or "main", ""
        body = Path(a.body_file).read_text() if a.body_file else ""
    else:
        title, body, base, branch = local_pr()
        base = a.base or base
    if not re.fullmatch(r"[\w./-]+", base):
        base = "main"
    findings, md = report(title, body, base, branch)
    if a.out:
        Path(a.out).write_text(md)
    if a.github:
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as fh:
                fh.write(md)
        for f in findings:
            print(f"::error title=pr-preflight::{escape(f)}")
        print(md)
        return 0
    print(md)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
