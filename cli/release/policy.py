#!/usr/bin/env python3
"""Propose an iohr release as the command line's `latest` in inorbithr/core.

The platform tells every client the newest release of its type
(`GET /v1/clients/versions`, the `clients_versions` MCP tool, the 426 and
`InOrbit-Client-Update` headers; platform RFC 0109). Until an admin publishes a
policy in the console, that answer is `[clients.policy.cli]` in core's
`configs/protocol/base.toml`, and nothing bumped it, so it stayed at
0.1.0-alpha.14 while the releases went on to alpha.28. This is that step: a pull
request that moves `latest` only. `minimum`, `grace_until` and `enforce` are an
admin's decision (7 days of grace for the command line) and stay as they are.

Uses the GitHub API only; GH_TOKEN is a short-lived GitHub App token limited to
inorbithr/core (contents and pull-requests write).

  GH_TOKEN=... cli/release/policy.py <version> [--dry-run]
"""

import base64
import json
import os
import re
import subprocess
import sys

REPO = "inorbithr/core"
PATH = "configs/protocol/base.toml"
SECTION = "[clients.policy.cli]"
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$")


def key(version):
    """Semantic-version order: a release is above its own prereleases."""
    m = SEMVER.match(version)
    if not m:
        raise ValueError(f"not a semantic version: {version!r}")
    major, minor, patch, pre = m.groups()
    ids = []
    for part in (pre.split(".") if pre else []):
        ids.append((0, int(part), "") if part.isdigit() else (1, 0, part))
    return (int(major), int(minor), int(patch), pre is None, ids)


def bump(text, version):
    """`text` with the cli policy's `latest` set to `version`, or None when
    it is already that or newer (a re-run of an older release must not go back)."""
    out, inside, done, old = [], False, False, None
    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("["):
            inside = stripped == SECTION
        m = re.match(r'^(latest\s*=\s*)"([^"]*)"(.*)$', line.rstrip("\n"))
        if inside and not done and m:
            old, done = m.group(2), True
            if key(version) <= key(old):
                return None
            line = f'{m.group(1)}"{version}"{m.group(3)}\n'
        out.append(line)
    if not done:
        raise ValueError(f"no `latest` under {SECTION} in {PATH}")
    return "".join(out)


def gh(*args, check=True):
    p = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and p.returncode:
        sys.exit(f"cli-policy: gh {' '.join(args[:2])} failed: {p.stderr.strip()}")
    return p


def main(argv):
    if not argv or argv[0].startswith("-"):
        sys.exit("usage: policy.py <version> [--dry-run]")
    version, dry = argv[0], "--dry-run" in argv[1:]
    key(version)
    if not dry and not os.environ.get("GH_TOKEN"):
        sys.exit("cli-policy: GH_TOKEN (a token for inorbithr/core) is not set")
    branch = f"clients-cli-{version}"
    cur = json.loads(gh("api", f"repos/{REPO}/contents/{PATH}?ref=main").stdout)
    new = bump(base64.b64decode(cur["content"]).decode(), version)
    if new is None:
        print(f"cli-policy: core already serves iohr {version} or newer; nothing to do")
        return
    if dry:
        print(new, end="")
        return
    if gh("api", f"repos/{REPO}/git/ref/heads/{branch}", check=False).returncode:
        base = gh("api", f"repos/{REPO}/git/ref/heads/main", "--jq", ".object.sha").stdout.strip()
        gh("api", "-X", "POST", f"repos/{REPO}/git/refs", "-f", f"ref=refs/heads/{branch}", "-f", f"sha={base}")
    existing = gh("api", f"repos/{REPO}/contents/{PATH}?ref={branch}", "--jq", ".sha").stdout.strip()
    gh("api", "-X", "PUT", f"repos/{REPO}/contents/{PATH}", "-f", f"branch={branch}",
       "-f", f"message=chore(clients): the iohr command line is {version}",
       "-f", f"content={base64.b64encode(new.encode()).decode()}", "-f", f"sha={existing}")
    if not gh("pr", "list", "--repo", REPO, "--head", branch, "--json", "number", "--jq", ".[].number").stdout.strip():
        body = (
            f"Moves `[clients.policy.cli] latest` to [{version}]"
            f"(https://github.com/inorbithr/sdk/releases/tag/iohr/v{version}), written by "
            "inorbithr/sdk's release workflow. Only `latest` moves: `minimum`, grace and "
            "`enforce` stay an admin's decision. An admin's published policy, when there is "
            "one, takes the place of this value.\n\n"
            "Implements: RFC 0109\nPRD 0144\nADR 0075\nStudy 0062\nMonitors: clients_versions\n"
        )
        gh("pr", "create", "--repo", REPO, "--head", branch, "--base", "main",
           "--title", f"chore(clients): the iohr command line is {version}", "--body", body)
    print(f"cli-policy: proposed iohr {version} to {REPO}")


if __name__ == "__main__":
    main(sys.argv[1:])
