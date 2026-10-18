#!/usr/bin/env python3
"""Run every SDK example against the conformance replay server (RFC 0165).

For each language and call type in examples/examples.toml the runner selects the case of
that name (conformance/cases/examples), runs the example with the credentials the call
type names and the server's address in the environment, and checks two things: the
process exits 0, and the server says the case passed (every exchange seen, in order).
It writes examples-results.json, one entry per example, and exits 1 if any failed.

    python tools/run-examples.py [--lang go --lang rust] [--only unary-get] [--results FILE]
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TIMEOUT_S = 120

KEY = {
    "INORBIT_KEY_ID": "key_example",
    "INORBIT_KEY_SECRET": "secret_example",
    "INORBIT_SCOPES": "identity:read connections:read events:read",
}
TOKEN = {"INORBIT_TOKEN": "api-token-1"}
CREDENTIALS = {"key": KEY, "token": TOKEN, "none": {}}


def call(base: str, method: str, path: str, body: dict | None = None) -> dict:
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data, method=method)
    if data:
        req.add_header("content-type", "application/json")
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.load(resp)


def start_server() -> tuple[subprocess.Popen[str], str]:
    binary = ROOT / "conformance/server/bin/replay"
    if not binary.exists():
        subprocess.run(
            ["go", "build", "-trimpath", "-o", str(binary), "."],
            cwd=ROOT / "conformance/server",
            check=True,
        )
    proc = subprocess.Popen(
        [str(binary), "--cases", str(ROOT / "conformance/cases")],
        stdout=subprocess.PIPE,
        text=True,
    )
    assert proc.stdout is not None
    line = proc.stdout.readline()
    m = re.match(r"replay: listening on (\S+)", line)
    if not m:
        proc.kill()
        sys.exit(f"examples: the replay server did not print its address: {line!r}")
    return proc, m.group(1)


def commit() -> str:
    out = subprocess.run(
        ["git", "rev-parse", "--short=12", "HEAD"], cwd=ROOT, capture_output=True, text=True
    )
    return out.stdout.strip() or "unknown"


def words(name: str) -> tuple[str, str]:
    parts = name.split("-")
    return "_".join(parts), "".join(p.capitalize() for p in parts)


def shell(cmd: str, cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", "-c", cmd], cwd=cwd, env=env, capture_output=True, text=True, timeout=TIMEOUT_S * 5
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lang", action="append", help="only this language (repeatable)")
    ap.add_argument("--only", action="append", help="only this call type (repeatable)")
    ap.add_argument("--results", default=str(ROOT / "examples-results.json"))
    args = ap.parse_args()

    manifest = tomllib.loads((ROOT / "examples/examples.toml").read_text())
    calls = manifest["call"]
    langs = manifest["language"]
    cases = {p.stem for p in (ROOT / "conformance/cases/examples").glob("*.yaml")}
    problems = [f"no case for call type {c['name']}" for c in calls if c["name"] not in cases]
    problems += [f"case {n} is in no call type" for n in sorted(cases - {c["name"] for c in calls})]
    if problems:
        sys.exit("examples: " + "; ".join(problems))

    base_env = {k: v for k, v in os.environ.items() if not k.startswith("INORBIT_")}
    sha = commit()
    server, addr = start_server()
    results: list[dict[str, object]] = []
    try:
        listed = call(addr, "GET", "/_cases?area=examples")["cases"]
        missing = [c["name"] for c in calls if f"examples/{c['name']}" not in listed]
        if missing:
            sys.exit(f"examples: the server does not list the cases {missing}")
        for lang, spec in langs.items():
            if args.lang and lang not in args.lang:
                continue
            cwd = ROOT / spec["dir"]
            skip = spec.get("skip", {})
            if "prepare" in spec:
                t0 = time.monotonic()
                done = shell(spec["prepare"], cwd, {**base_env})
                if done.returncode != 0:
                    print(f"[{lang}] prepare failed:\n{done.stdout}{done.stderr}", file=sys.stderr)
                    for c in calls:
                        results.append(entry(lang, c["name"], False, 0, sha, "prepare failed"))
                    continue
                print(f"[{lang}] prepared in {time.monotonic() - t0:.1f}s")
            for c in calls:
                name = c["name"]
                if args.only and name not in args.only:
                    continue
                if name in skip:
                    results.append(entry(lang, name, None, 0, sha, f"skipped: {skip[name]}"))
                    print(f"[{lang}] {name}: skipped ({skip[name]})")
                    continue
                results.append(run_one(lang, spec, c, addr, base_env, cwd, sha))
    finally:
        server.terminate()
        server.wait(timeout=10)

    Path(args.results).write_text(json.dumps(results, indent=2) + "\n")
    ran = [r for r in results if r["ok"] is not None]
    bad = [r for r in ran if not r["ok"]]
    for lang in dict.fromkeys(str(r["language"]) for r in results):
        mine = [r for r in ran if r["language"] == lang]
        print(f"{lang}: {sum(1 for r in mine if r['ok'])}/{len(mine)} passed")
    print(f"examples: {len(ran) - len(bad)}/{len(ran)} passed, results in {args.results}")
    return 1 if bad else 0


def entry(
    lang: str, name: str, ok: bool | None, ms: int, sha: str, note: str = ""
) -> dict[str, object]:
    out: dict[str, object] = {
        "language": lang,
        "example": name,
        "ok": ok,
        "duration_ms": ms,
        "commit": sha,
    }
    if note:
        out["note"] = note
    return out


def run_one(
    lang: str,
    spec: dict,
    c: dict,
    addr: str,
    base_env: dict[str, str],
    cwd: Path,
    sha: str,
) -> dict[str, object]:
    name = c["name"]
    snake, pascal = words(name)
    cmd = spec["run"].format(name=name, snake=snake, pascal=pascal)
    env = {
        **base_env,
        "INORBIT_BASE_URL": addr,
        "INORBIT_TOKEN_URL": addr + "/oauth2/token",
        "INORBIT_CONFIG_FILE": "off",
        **CREDENTIALS[c["env"]],
        **c.get("extra", {}),
    }
    call(addr, "POST", "/_case", {"name": f"examples/{name}"})
    t0 = time.monotonic()
    note = ""
    try:
        done = shell(cmd, cwd, env)
        ok = done.returncode == 0
        if not ok:
            note = f"exit {done.returncode}: " + (done.stderr or done.stdout).strip()[-400:]
    except subprocess.TimeoutExpired:
        ok, note = False, f"timed out after {TIMEOUT_S * 5}s"
    ms = round((time.monotonic() - t0) * 1000)
    verdict = call(addr, "GET", "/_result")
    if ok and verdict.get("status") != "pass":
        ok, note = False, f"the server says {verdict.get('status')}: {json.dumps(verdict)[:300]}"
    print(f"[{lang}] {name}: {'ok' if ok else 'FAILED'} ({ms} ms){' ' + note if note else ''}")
    return entry(lang, name, ok, ms, sha, note)


if __name__ == "__main__":
    sys.exit(main())
