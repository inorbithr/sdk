"""The driver for `conformance/cases`.

It starts the replay server, loads every case, runs its action through the generated
public surface (both the blocking and the `asyncio` client), and compares the result and
the server's verdict with `expect` (`conformance/README.md`). Without the server binary
(`mise run conformance:server:build`) it skips, unless IOHR_TEST_REQUIRE_REPLAY is set.
"""

from __future__ import annotations

import asyncio
import os
import subprocess
import sys
from collections.abc import Callable, Iterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, cast

import httpx
import pytest

from inorbithr import (
    ApiError,
    AsyncClient,
    AsyncPublic,
    Client,
    CreateEndpointRequest,
    InOrbitError,
    Public,
    RawResponse,
)

ROOT = Path(__file__).resolve().parents[3]
BIN = ROOT / ("conformance/server/bin/replay" + (".exe" if sys.platform == "win32" else ""))
FORMS = ("sync", "async")

Result = RawResponse | InOrbitError


@pytest.fixture(scope="module")
def replay() -> Iterator[str]:
    """The replay server's address, for the whole module."""
    if not BIN.exists():
        note = (
            "no replay server at conformance/server/bin/replay; run "
            "`mise run conformance:server:build`"
        )
        if os.environ.get("IOHR_TEST_REQUIRE_REPLAY"):
            pytest.fail(note)
        pytest.skip(note)
    child = subprocess.Popen(
        [str(BIN), "--addr", "127.0.0.1:0", "--cases", str(ROOT / "conformance/cases")],
        stdout=subprocess.PIPE,
        text=True,
    )
    assert child.stdout is not None
    first = child.stdout.readline()
    prefix = "replay: listening on "
    assert first.startswith(prefix), f"unexpected first line: {first!r}"
    try:
        yield first[len(prefix) :].strip()
    finally:
        child.terminate()
        child.wait(timeout=10)


def subset(want: object, got: object) -> bool:
    """Whether `want` is contained in `got`: objects by key, lists element by element."""
    if isinstance(want, list):
        w = cast("list[object]", want)
        if not isinstance(got, list):
            return False
        g = cast("list[object]", got)
        return len(w) == len(g) and all(subset(a, b) for a, b in zip(w, g, strict=True))
    if isinstance(want, dict):
        w_map = cast("dict[str, object]", want)
        if not isinstance(got, dict):
            return False
        g_map = cast("dict[str, object]", got)
        return all(k in g_map and subset(v, g_map[k]) for k, v in w_map.items())
    return want == got


def _arg(args: dict[str, Any], key: str) -> str:
    value = args.get(key)
    return "" if value is None else str(value)


def call_sync(api: Public, action: dict[str, Any]) -> RawResponse:
    """Runs one action through the blocking surface."""
    args: dict[str, Any] = action.get("args") or {}
    op = action["op"]
    if op == "me":
        return api.me().raw
    if op == "accounts.get_me":
        return api.accounts.get_me().raw
    if op == "accounts.get_usage":
        return api.accounts.get_usage(
            _arg(args, "org_id"), from_=args.get("from"), to=args.get("to")
        ).raw
    if op == "radar.get_digest":
        return api.radar.get_digest(_arg(args, "id")).raw
    if op == "events.create_endpoint":
        return api.events.create_endpoint(CreateEndpointRequest.model_validate(args)).raw
    if op == "events.delete_endpoint":
        return api.events.delete_endpoint(_arg(args, "endpoint_id")).raw
    raise AssertionError(f"the conformance schema names an op this driver does not know: {op}")


async def call_async(api: AsyncPublic, action: dict[str, Any]) -> RawResponse:
    """Runs one action through the `asyncio` surface."""
    args: dict[str, Any] = action.get("args") or {}
    op = action["op"]
    if op == "me":
        return (await api.me()).raw
    if op == "accounts.get_me":
        return (await api.accounts.get_me()).raw
    if op == "accounts.get_usage":
        r = await api.accounts.get_usage(
            _arg(args, "org_id"), from_=args.get("from"), to=args.get("to")
        )
        return r.raw
    if op == "radar.get_digest":
        return (await api.radar.get_digest(_arg(args, "id"))).raw
    if op == "events.create_endpoint":
        body = CreateEndpointRequest.model_validate(args)
        return (await api.events.create_endpoint(body)).raw
    if op == "events.delete_endpoint":
        return (await api.events.delete_endpoint(_arg(args, "endpoint_id"))).raw
    raise AssertionError(f"the conformance schema names an op this driver does not know: {op}")


def options(case: dict[str, Any], url: str) -> dict[str, Any]:
    """The client options a case asks for."""
    o: dict[str, Any] = case.get("client") or {}
    found: dict[str, Any] = {
        "base_url": url,
        "token_url": f"{url}/oauth2/token",
        "key_id": o.get("key_id", "ak_test"),
        "key_secret": o.get("key_secret", "s3cr3t"),
        "scopes": o.get("scopes", ["identity:read"]),
        "max_retries": o.get("max_retries", 2),
    }
    if "timeout_ms" in o:
        found["timeout"] = o["timeout_ms"] / 1000
    return found


def run_sync(case: dict[str, Any], url: str) -> list[Result]:
    """Runs a case's action with the blocking client."""
    action: dict[str, Any] = case["action"]
    with Client(**options(case, url)) as client:
        api = Public(client)

        def once() -> Result:
            try:
                return call_sync(api, action)
            except InOrbitError as e:
                return e

        if "concurrent" in action:
            with ThreadPoolExecutor(max_workers=action["concurrent"]) as pool:
                futures = [pool.submit(once) for _ in range(action["concurrent"])]
                return [f.result() for f in futures]
        return [once() for _ in range(action.get("repeat", 1))]


def run_async(case: dict[str, Any], url: str) -> list[Result]:
    """Runs a case's action with the `asyncio` client."""
    action: dict[str, Any] = case["action"]

    async def main() -> list[Result]:
        async with AsyncClient(**options(case, url)) as client:
            api = AsyncPublic(client)

            async def once() -> Result:
                try:
                    return await call_async(api, action)
                except InOrbitError as e:
                    return e

            if "concurrent" in action:
                return list(await asyncio.gather(*(once() for _ in range(action["concurrent"]))))
            return [await once() for _ in range(action.get("repeat", 1))]

    return asyncio.run(main())


def check_error(e: InOrbitError, want: dict[str, Any]) -> list[str]:
    """What differs between an error and the one a case expects."""
    problems: list[str] = []
    if "kind" in want and e.kind != want["kind"]:
        problems.append(f"error kind: want {want['kind']}, got {e.kind} ({e})")
    if isinstance(e, ApiError):
        if "code" in want and e.code != want["code"]:
            problems.append(f"error code: want {want['code']}, got {e.code}")
        if "status" in want and e.status != want["status"]:
            problems.append(f"error status: want {want['status']}, got {e.status}")
    elif "code" in want or "status" in want:
        problems.append(f"error: want an API error, got {e}")
    if "message_contains" in want and want["message_contains"] not in str(e):
        problems.append(f"message: want it to contain {want['message_contains']!r}, got {e}")
    if "message_excludes" in want and want["message_excludes"] in str(e):
        problems.append(f"message: must not contain {want['message_excludes']!r}, got {e}")
    return problems


def check(case: dict[str, Any], results: list[Result], verdict: dict[str, Any]) -> list[str]:
    """What differs between a run and the case's `expect`."""
    expect: dict[str, Any] = case["expect"]
    problems: list[str] = []
    if verdict.get("status") != "pass":
        problems.append(
            f"server: {verdict.get('status')} mismatch={verdict.get('mismatch')} "
            f"next={verdict.get('next')}"
        )
    for key in ("attempts", "token_exchanges"):
        if key in expect and verdict.get(key) != expect[key]:
            problems.append(f"{key}: want {expect[key]}, got {verdict.get(key)}")
    for r in results:
        if isinstance(r, RawResponse):
            if "ok" in expect and not subset(expect["ok"], r.json()):
                problems.append(f"ok: want a superset of {expect['ok']}, got {r.json()}")
            elif "error" in expect:
                problems.append(f"want an error, got HTTP {r.status}")
        elif "error" in expect:
            problems.extend(check_error(r, expect["error"]))
        elif "ok" in expect:
            problems.append(f"want ok, got {r}")
    return problems


RUNNERS: dict[str, Callable[[dict[str, Any], str], list[Result]]] = {
    "sync": run_sync,
    "async": run_async,
}


@pytest.mark.parametrize("form", FORMS)
def test_every_case_passes(replay: str, form: str) -> None:
    """Every case of `conformance/cases` passes through the generated public surface."""
    cases: list[str] = httpx.get(f"{replay}/_cases").json()["cases"]
    assert cases, "no cases listed"
    failed: list[str] = []
    ran: list[str] = []
    for name in cases:
        loaded = httpx.post(f"{replay}/_case", json={"name": name})
        if loaded.status_code == 501:
            continue
        assert loaded.is_success, f"{name}: loading answered {loaded.status_code}"
        case: dict[str, Any] = loaded.json()["case"]
        if "py" in (case.get("pending") or []) or case["area"] in ("sse", "socket"):
            continue
        ran.append(name)
        results = RUNNERS[form](case, replay)
        verdict: dict[str, Any] = httpx.get(f"{replay}/_result").json()
        problems = check(case, results, verdict)
        if problems:
            failed.append(f"{name}:\n  " + "\n  ".join(problems))
    assert not failed, "\n".join(failed)
    assert ran, "every case was skipped"
    print(f"{form}: {len(ran)} cases passed")
