#!/usr/bin/env python3
"""Compare the command line's environments with core's list (RFC 0148, Keel K07).

core's devops/ci/envs.toml is the one list; cli/crates/iohr/environments.toml mirrors
its names, default and loopback ports (`edge_port`, `engine_port`; `iohr tunnel`). Usage: env-drift.py <path to core's envs.toml>. Exit 1 on drift.
"""
import sys
import tomllib
from pathlib import Path

MINE = Path(__file__).resolve().parent.parent / "cli/crates/iohr/environments.toml"


def names(path):
    d = tomllib.loads(Path(path).read_text())
    envs = {k: v for k, v in d.items() if isinstance(v, dict)}
    ports = {k: (v.get("edge_port"), v.get("engine_port")) for k, v in sorted(envs.items())}
    return d["default"], sorted(envs), ports


def main():
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    core, mine = names(sys.argv[1]), names(MINE)
    if core != mine:
        print(f"drift: core {core} != cli {mine}", file=sys.stderr)
        return 1
    print(f"environments match core: default {core[0]}, names {', '.join(core[1])}, ports {core[2]}")
    return 0


sys.exit(main())
