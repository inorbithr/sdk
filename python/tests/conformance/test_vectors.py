"""The conformance vectors (`conformance/vectors/`, docs/config.md section 9.2).

Pure functions, no server: configuration resolution, config file paths, `no_proxy`,
rate-limit headers and durations, each through the runtime's own entry points.
"""

from __future__ import annotations

import json
import stat
import sys
from pathlib import Path
from typing import Any, cast

import pytest
import yaml

from inorbithr import Client, ConfigError, LoadOptions
from inorbithr._config import config_path, parse_duration, resolve
from inorbithr._ratelimit import parse_rate_limit
from inorbithr._transport import ProxyRule

ROOT = Path(__file__).resolve().parents[3]
VECTORS = ROOT / "conformance" / "vectors"


def vectors(kind: str) -> list[dict[str, Any]]:
    """Every vector of `kind` that is not pending for Python."""
    out: list[dict[str, Any]] = []
    for p in sorted((VECTORS / kind).glob("*.yaml")):
        v = cast("dict[str, Any]", yaml.safe_load(p.read_text(encoding="utf-8")))
        if "py" not in lst(v.get("pending")):
            out.append(v)
    return out


def obj(value: object) -> dict[str, Any]:
    """A YAML or JSON object, or an empty one."""
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


def lst(value: object) -> list[Any]:
    """A YAML or JSON list, or an empty one."""
    return cast("list[Any]", value) if isinstance(value, list) else []


def subset(want: object, got: object) -> bool:
    """`want` is contained in `got`: objects by key, lists element by element, paths by `/`."""
    if isinstance(want, dict):
        w = cast("dict[str, object]", want)
        if not isinstance(got, dict):
            return False
        g = cast("dict[str, object]", got)
        return all(k in g and subset(v, g[k]) for k, v in w.items())
    if isinstance(want, list):
        wl = cast("list[object]", want)
        if not isinstance(got, list):
            return False
        gl = cast("list[object]", got)
        return len(wl) == len(gl) and all(subset(a, b) for a, b in zip(wl, gl, strict=True))
    if isinstance(want, str) and isinstance(got, str):
        return want.replace("\\", "/") == got.replace("\\", "/")
    return want == got


def substitute(v: object, d: str, file: str) -> object:
    """`{dir}` and `{file}` replaced throughout."""
    if isinstance(v, str):
        return v.replace("{file}", file).replace("{dir}", d)
    if isinstance(v, list):
        return [substitute(x, d, file) for x in cast("list[object]", v)]
    if isinstance(v, dict):
        return {k: substitute(x, d, file) for k, x in cast("dict[str, object]", v).items()}
    return v


def fake_iohr(directory: Path) -> str:
    """A directory holding an executable named `iohr`, for `PATH`."""
    bin_dir = directory / "bin"
    bin_dir.mkdir()
    name = "iohr.bat" if sys.platform == "win32" else "iohr"
    exe = bin_dir / name
    exe.write_text("@echo off\n" if sys.platform == "win32" else "#!/bin/sh\n")
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
    return str(bin_dir)


def run_config_vector(v: dict[str, Any], tmp: Path) -> list[str]:
    """What differs between resolving a `config` vector and its expectation."""
    d = str(tmp)
    inp: dict[str, Any] = obj(v.get("input"))
    os_name = inp.get("os", "linux")
    env = {k: str(x).replace("{dir}", d) for k, x in obj(inp.get("env")).items()}
    home: str | None = f"{d}/home" if inp.get("home") else ""
    if home:
        Path(home).mkdir()
    for name, content in obj(inp.get("files")).items():
        p = tmp / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding="utf-8")
    code: dict[str, object] = dict(obj(inp.get("code")))
    file = f"{d}/config.toml"
    if "config_file" in inp:
        if inp.get("home"):
            located = config_path(
                os_name, {k: x for k, x in env.items() if k != "INORBIT_CONFIG_FILE"}, home
            )
            assert located is not None
            file = located[0]
            Path(file).parent.mkdir(parents=True, exist_ok=True)
            Path(file).write_text(inp["config_file"], encoding="utf-8")
        else:
            Path(file).write_text(inp["config_file"], encoding="utf-8")
            code["config_file"] = file
    if code.get("http_client") == "custom":  # a caller-supplied client
        code["http_client"] = object()
    if inp.get("cli") == "present":
        env["PATH"] = fake_iohr(tmp)
    expect = cast("dict[str, Any]", substitute(v["expect"], d, file))
    options = LoadOptions(env=env, os=os_name, home=home, cwd=d)
    problems: list[str] = []
    got = None
    error: ConfigError | None = None
    try:
        got = resolve(code, options, profile_type=inp.get("profile_type"))
        shown = json.dumps(got.doc)
    except ConfigError as e:
        error = e
        shown = f"{e} {e.problems}"
    for x in lst(expect.get("excludes")):
        if x in shown:
            problems.append(f"{x!r} appears in {shown}")
    if "error" in expect:
        want = expect["error"]
        if error is None:
            return [*problems, f"want an error, got {shown}"]
        if "problems" in want:
            have = error.problems
            if len(want["problems"]) != len(have):
                problems.append(f"want {len(want['problems'])} problems, got {have}")
            for w, h in zip(want["problems"], have, strict=False):
                if "setting" in w and w["setting"] != h.setting:
                    problems.append(f"setting {w['setting']} != {h}")
                if "source" in w and w["source"] != h.source:
                    problems.append(f"source {w['source']!r} != {h}")
                if "message_contains" in w and w["message_contains"] not in h.message:
                    problems.append(f"message lacks {w['message_contains']!r}: {h}")
        problems.extend(
            f"the error lacks {p!r}:\n{error}"
            for p in lst(want.get("message_contains"))
            if p not in str(error)
        )
        return problems
    if error is not None or got is None:
        return [*problems, f"unexpected error:\n{error}"]
    doc = got.doc
    for key in ("profile", "settings", "credential", "pipeline"):
        if key in expect and not subset(expect[key], doc[key]):
            problems.append(f"{key}: want {expect[key]}, got {doc[key]}")
    if "config_file" in expect:
        w, h = expect["config_file"], doc["config_file"]
        if (w is None) != (h is None) or (w is not None and not subset(w, h)):
            problems.append(f"config_file: want {w}, got {h}")
    problems.extend(
        f"{a} should not be in settings"
        for a in lst(expect.get("settings_absent"))
        if a in doc["settings"]
    )
    problems.extend(
        f"ignored lacks {w}: {doc['ignored']}"
        for w in lst(expect.get("ignored"))
        if not any(subset(w, h) for h in doc["ignored"])
    )
    return problems


@pytest.mark.parametrize("vector", vectors("config"), ids=lambda v: v["name"])
def test_config(vector: dict[str, Any], tmp_path: Path) -> None:
    """Each `config` vector resolves as it says."""
    problems = run_config_vector(vector, tmp_path)
    assert not problems, "\n".join(problems)


def test_config_paths() -> None:
    """The config file is where the command line keeps it, on each OS."""
    for v in vectors("config-path"):
        for c in v["checks"]:
            code = obj(c.get("code")).get("config_file")
            got = config_path(c["os"], obj(c.get("env")), c.get("home"), code)
            assert (None if got is None else got[0]) == c["expect"], c["summary"]


def test_durations() -> None:
    """The one duration syntax."""
    for v in vectors("durations"):
        for c in v["checks"]:
            want = None if c["expect"] == "error" else c["expect"]
            assert parse_duration(c["value"]) == want, c["value"]


def test_no_proxy() -> None:
    """Which proxy each URL goes through."""
    for v in vectors("no-proxy"):
        for c in v["checks"]:
            code = {"token": "t", "config_file": "off", **obj(c.get("code"))}
            options = LoadOptions(env=obj(c.get("env")), home="")
            try:
                resolved = resolve(code, options)
                rule = ProxyRule.from_settings(resolved.values, resolved.doc["settings"])
                got: object = rule.proxy_for(c["url"])
            except ConfigError:
                got = {"error": "config"}
            assert got == c["expect"], c["summary"]


def test_rate_limit_headers() -> None:
    """The snapshot the rate-limit headers give."""
    for v in vectors("rate-limit"):
        for c in v["checks"]:
            snap = parse_rate_limit(c["headers"])
            got: dict[str, object] | None = None
            if snap is not None:
                got = {}
                if snap.limit is not None:
                    got["limit"] = snap.limit
                if snap.remaining is not None:
                    got["remaining"] = snap.remaining
                if snap.reset is not None:
                    got["reset_ms"] = round(snap.reset.total_seconds() * 1000)
                if snap.policy is not None:
                    policy: dict[str, object] = {"name": snap.policy.name}
                    if snap.policy.quota is not None:
                        policy["quota"] = snap.policy.quota
                    if snap.policy.window is not None:
                        policy["window_ms"] = round(snap.policy.window.total_seconds() * 1000)
                    got["policy"] = policy
            assert got == c["expect"], c["summary"]


def test_load_uses_the_injected_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    """`Client.load` resolves from `LoadOptions` alone, never the process environment."""
    monkeypatch.setenv("INORBIT_TIMEOUT", "not-a-duration")
    client = Client.load(load_options=LoadOptions(env={"INORBIT_TOKEN": "t"}, home=""))
    doc = client.config().describe()
    assert doc["settings"]["timeout"] == {"value": "30s", "source": "default"}
    assert doc["credential"]["source"] == "env"
