"""The driver for `conformance/cases`.

It starts the replay server, loads every case, runs its action through the generated
public surface (both the blocking and the `asyncio` client), and compares the result and
the server's verdict with `expect` (`conformance/README.md`). Without the server binary
(`mise run conformance:server:build`) it skips, unless IOHR_TEST_REQUIRE_REPLAY is set.
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import os
import re
import subprocess
import sys
import tempfile
from collections.abc import Callable, Iterator
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, cast

import httpx
import pytest
from opentelemetry.sdk.trace import ReadableSpan, TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.sdk.trace.id_generator import RandomIdGenerator

from inorbithr import (
    ApiError,
    AsyncCallNext,
    AsyncClient,
    AsyncPublic,
    CallNext,
    Client,
    CreateDocumentRequest,
    CreateEndpointRequest,
    InOrbitError,
    LoadOptions,
    Pipeline,
    Public,
    RawResponse,
    SdkRequest,
    SdkResponse,
    UpdateEndpointRequest,
    call_options,
)

ROOT = Path(__file__).resolve().parents[3]
BIN = ROOT / ("conformance/server/bin/replay" + (".exe" if sys.platform == "win32" else ""))
FORMS = ("sync", "async")


@dataclass
class Streamed:
    """What a stream yielded, in wire names, and the error that ended it, if any."""

    items: list[object] = field(default_factory=list[object])
    error: InOrbitError | None = None


Result = RawResponse | InOrbitError | Streamed


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


def obj(value: object) -> dict[str, Any]:
    """A YAML or JSON object, or an empty one."""
    return cast("dict[str, Any]", value) if isinstance(value, dict) else {}


def lst(value: object) -> list[Any]:
    """A YAML or JSON list, or an empty one."""
    return cast("list[Any]", value) if isinstance(value, list) else []


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


def matches(want: str, got: str | None, captures: dict[str, str]) -> bool:
    """A header value against a matcher: a literal, `*`, `$name` or `~regex`."""
    if got is None:
        return False
    if want == "*":
        return True
    if want.startswith("$"):
        return captures.setdefault(want, got) == got
    if want.startswith("~"):
        return re.fullmatch(want[1:], got) is not None
    return want == got


# --- what a case's client is built with ----------------------------------------------


@dataclass
class Setup:
    """A case's client options, what the driver watches, and its temporary directory."""

    case: dict[str, Any]
    answer: dict[str, Any]
    directory: Path
    kwargs: dict[str, Any] = field(default_factory=dict[str, Any])
    load: LoadOptions | None = None
    probes: dict[str, list[dict[str, str]]] = field(default_factory=dict[str, list[dict[str, str]]])
    records: list[dict[str, Any]] = field(default_factory=list[dict[str, Any]])
    messages: list[str] = field(default_factory=list[str])
    exporter: InMemorySpanExporter | None = None

    @property
    def action(self) -> dict[str, Any]:
        """The case's action."""
        return cast("dict[str, Any]", self.case["action"])


class LevelOneIds(RandomIdGenerator):
    """Random ids without the W3C level 2 `random` flag, so `traceparent` ends in `-01`.

    The case's pattern is level 1's; the OpenTelemetry SDK for Python sets `03` by
    default, which the SDK sends unchanged.
    """

    def is_trace_id_random(self) -> bool:
        return False


class Capture(logging.Handler):
    """Keeps every record the client logs, as data and as text."""

    def __init__(self, setup: Setup) -> None:
        super().__init__(logging.DEBUG)
        self.setup = setup

    def emit(self, record: logging.LogRecord) -> None:
        self.setup.records.append(cast("dict[str, Any]", getattr(record, "inorbithr", {})))
        self.setup.messages.append(record.getMessage())


class Probe:
    """A user middleware that records the headers of every request it sees."""

    def __init__(self, name: str, seen: list[dict[str, str]]) -> None:
        self.name = name
        self.seen = seen

    def __call__(self, request: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        self.seen.append(dict(request.headers.items()))
        return call_next(request)


class AsyncProbe:
    """`Probe` for the `asyncio` client."""

    def __init__(self, name: str, seen: list[dict[str, str]]) -> None:
        self.name = name
        self.seen = seen

    async def __call__(self, request: SdkRequest, call_next: AsyncCallNext, /) -> SdkResponse:
        self.seen.append(dict(request.headers.items()))
        return await call_next(request)


def substitute(value: str, setup: Setup) -> str:
    """`{replay}` and `{dir}` replaced."""
    return value.replace("{replay}", str(setup.answer["base_url"])).replace(
        "{dir}", str(setup.directory)
    )


def write_files(setup: Setup, files: dict[str, str]) -> None:
    """Writes `files` into the case's directory."""
    for name, content in files.items():
        p = setup.directory / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(substitute(content, setup), encoding="utf-8")


def configure(setup: Setup) -> None:
    """The options a case's `client` asks for (conformance/README.md)."""
    o: dict[str, Any] = obj(setup.case.get("client"))
    a = setup.answer
    url = str(a["base_url"])
    k: dict[str, Any] = {}
    if o.get("load"):
        env = {name: substitute(str(v), setup) for name, v in obj(o.get("env")).items()}
        write_files(setup, obj(o.get("files")))
        if "config_file" in o:
            path = setup.directory / "config.toml"
            path.write_text(substitute(o["config_file"], setup), encoding="utf-8")
            k["config_file"] = str(path)
        if o.get("cli"):
            k["cli_path"] = str(BIN)
        for name in ("profile", "credential_sources"):
            if name in o:
                k[name] = o[name]
        setup.load = LoadOptions(env=env, home="", cwd=str(setup.directory))
    else:
        k.update(
            base_url=url,
            token_url=f"{url}/oauth2/token",
            key_id=o.get("key_id", "ak_test"),
            key_secret=o.get("key_secret", "s3cr3t"),
            scopes=o.get("scopes", ["identity:read"]),
            max_retries=o.get("max_retries", 2),
        )
    if "max_retries" in o:
        k["max_retries"] = o["max_retries"]
    if "timeout_ms" in o:
        k["timeout"] = o["timeout_ms"] / 1000
    if "total_timeout_ms" in o:
        k["total_timeout"] = o["total_timeout_ms"] / 1000
    if "streams" in o:
        k["streams"] = o["streams"]
    if "stream_idle_timeout_ms" in o:
        k["stream_idle_timeout"] = o["stream_idle_timeout_ms"] / 1000
    for name in (
        "rate_limit",
        "retry_budget_capacity",
        "no_proxy",
        "log_headers",
        "log_allow_headers",
    ):
        if name in o:
            k[name] = o[name]
    transport = o.get("transport")
    if transport in ("https", "mtls", "proxy") and o.get("ca_bundle", True) is not False:
        k["ca_bundle"] = a["ca_file"]
    if transport == "mtls":
        k["client_cert"] = a["client_cert_file"]
        k["client_key"] = a["client_key_file"]
    if transport == "proxy":
        k["proxy"] = a["proxy_url"]
    if "log" in o:
        logger = logging.getLogger(f"inorbithr.conformance.{id(setup)}")
        logger.propagate = False
        logger.setLevel(logging.DEBUG)
        logger.addHandler(Capture(setup))
        k["log"] = o["log"]
        k["logger"] = logger
    if o.get("tracing") is True:
        setup.exporter = InMemorySpanExporter()
        provider = TracerProvider(id_generator=LevelOneIds())
        provider.add_span_processor(SimpleSpanProcessor(setup.exporter))
        k["tracing"] = True
        k["tracer_provider"] = provider
    elif o.get("tracing") is False:
        k["tracing"] = False
    setup.kwargs = k


def edit_pipeline(setup: Setup, form: str) -> Callable[[Any], object] | None:
    """The pipeline edits a case asks for: probes added, built-ins removed."""
    spec: dict[str, Any] = obj(obj(setup.case.get("client")).get("pipeline"))
    if not spec:
        return None

    def edit(p: Pipeline[Any]) -> None:
        for add in lst(spec.get("add")):
            seen = setup.probes.setdefault(add["name"], [])
            m: Any = Probe(add["name"], seen) if form == "sync" else AsyncProbe(add["name"], seen)
            if add.get("stage") == "per_call":
                p.add_per_call(m)
            else:
                p.add_per_retry(m)
        for name in lst(spec.get("remove")):
            p.remove(name)

    return edit


# --- running the action --------------------------------------------------------------


def _arg(args: dict[str, Any], key: str) -> str:
    value = args.get(key)
    return "" if value is None else str(value)


def _per_call(action: dict[str, Any]) -> dict[str, Any]:
    o: dict[str, Any] = obj(action.get("options"))
    out: dict[str, Any] = {}
    if "timeout_ms" in o:
        out["timeout"] = o["timeout_ms"] / 1000
    return out


def stream_sync(api: Public, action: dict[str, Any]) -> Streamed:
    """Reads a stream with the blocking surface, `take` items at most."""
    args: dict[str, Any] = obj(action.get("args"))
    out = Streamed()
    try:
        with api.events.stream_events(
            types=args.get("types"), account_id=args.get("account_id")
        ) as events:
            for event in events:
                out.items.append(event.model_dump(mode="json", by_alias=True))
                if len(out.items) == action.get("take"):
                    break
    except InOrbitError as e:
        out.error = e
    return out


async def stream_async(api: AsyncPublic, action: dict[str, Any]) -> Streamed:
    """Reads a stream with the `asyncio` surface, `take` items at most."""
    args: dict[str, Any] = obj(action.get("args"))
    out = Streamed()
    try:
        async with api.events.stream_events(
            types=args.get("types"), account_id=args.get("account_id")
        ) as events:
            async for event in events:
                out.items.append(event.model_dump(mode="json", by_alias=True))
                if len(out.items) == action.get("take"):
                    break
    except InOrbitError as e:
        out.error = e
    return out


def call_sync(api: Public, action: dict[str, Any]) -> RawResponse:  # noqa: PLR0911 - one return per op
    """Runs one action through the blocking surface."""
    args: dict[str, Any] = obj(action.get("args"))
    op = action["op"]
    kw = _per_call(action)
    if op == "me":
        return api.me(**kw).raw
    if op == "accounts.get_me":
        return api.accounts.get_me(**kw).raw
    if op == "accounts.get_usage":
        return api.accounts.get_usage(
            _arg(args, "org_id"), from_=args.get("from"), to=args.get("to"), **kw
        ).raw
    if op == "radar.get_digest":
        return api.radar.get_digest(_arg(args, "id"), **kw).raw
    if op == "events.create_endpoint":
        key = obj(action.get("options")).get("idempotency_key")
        body = CreateEndpointRequest.model_validate(args)
        return api.events.create_endpoint(body, idempotency_key=key, **kw).raw
    if op == "events.update_endpoint":
        body = UpdateEndpointRequest.model_validate(
            {k: v for k, v in args.items() if k != "endpoint_id"}
        )
        return api.events.update_endpoint(_arg(args, "endpoint_id"), body, **kw).raw
    if op == "events.delete_endpoint":
        return api.events.delete_endpoint(_arg(args, "endpoint_id"), **kw).raw
    if op == "decisions.create_document":
        doc = CreateDocumentRequest.model_validate(
            {k: v for k, v in args.items() if k != "space_id"}
        )
        return api.decisions.create_document(_arg(args, "space_id"), doc, **kw).raw
    raise AssertionError(f"the conformance schema names an op this driver does not know: {op}")


async def call_async(api: AsyncPublic, action: dict[str, Any]) -> RawResponse:  # noqa: PLR0911 - one return per op
    """Runs one action through the `asyncio` surface."""
    args: dict[str, Any] = obj(action.get("args"))
    op = action["op"]
    kw = _per_call(action)
    if op == "me":
        return (await api.me(**kw)).raw
    if op == "accounts.get_me":
        return (await api.accounts.get_me(**kw)).raw
    if op == "accounts.get_usage":
        r = await api.accounts.get_usage(
            _arg(args, "org_id"), from_=args.get("from"), to=args.get("to"), **kw
        )
        return r.raw
    if op == "radar.get_digest":
        return (await api.radar.get_digest(_arg(args, "id"), **kw)).raw
    if op == "events.create_endpoint":
        key = obj(action.get("options")).get("idempotency_key")
        body = CreateEndpointRequest.model_validate(args)
        return (await api.events.create_endpoint(body, idempotency_key=key, **kw)).raw
    if op == "events.update_endpoint":
        body = UpdateEndpointRequest.model_validate(
            {k: v for k, v in args.items() if k != "endpoint_id"}
        )
        return (await api.events.update_endpoint(_arg(args, "endpoint_id"), body, **kw)).raw
    if op == "events.delete_endpoint":
        return (await api.events.delete_endpoint(_arg(args, "endpoint_id"), **kw)).raw
    if op == "decisions.create_document":
        doc = CreateDocumentRequest.model_validate(
            {k: v for k, v in args.items() if k != "space_id"}
        )
        return (await api.decisions.create_document(_arg(args, "space_id"), doc, **kw)).raw
    raise AssertionError(f"the conformance schema names an op this driver does not know: {op}")


def traced(action: dict[str, Any]) -> contextlib.AbstractContextManager[None]:
    """The caller's `traceparent`, when the action passes one."""
    tp = obj(action.get("options")).get("traceparent")
    return call_options(traceparent=tp) if tp else contextlib.nullcontext()


def rewrite(setup: Setup, done: int) -> None:
    """Rewrites the action's files after `after` calls (rotation)."""
    spec = setup.action.get("rewrite")
    if spec and done == spec.get("after"):
        write_files(setup, obj(spec.get("files")))
        # A new modification time even on a coarse clock.
        for name in obj(spec.get("files")):
            p = setup.directory / name
            st = p.stat()
            os.utime(p, ns=(st.st_atime_ns, st.st_mtime_ns + 2_000_000_000))


def run_sync(setup: Setup) -> tuple[list[Result], Any]:
    """Runs a case's action with the blocking client."""
    action = setup.action
    edit = edit_pipeline(setup, "sync")
    kw = dict(setup.kwargs)
    if edit is not None:
        kw["pipeline"] = edit
    client = Client.load(load_options=setup.load, **kw) if setup.load else Client(**kw)
    with client:
        api = Public(client)

        def once() -> Result:
            if action["op"] == "events.stream_events":
                return stream_sync(api, action)
            try:
                with traced(action):
                    return call_sync(api, action)
            except InOrbitError as e:
                return e

        if "concurrent" in action:
            with ThreadPoolExecutor(max_workers=action["concurrent"]) as pool:
                futures = [pool.submit(once) for _ in range(action["concurrent"])]
                return [f.result() for f in futures], client.config().describe()
        results: list[Result] = []
        for i in range(action.get("repeat", 1)):
            results.append(once())
            rewrite(setup, i + 1)
        return results, client.config().describe()


def run_async(setup: Setup) -> tuple[list[Result], Any]:
    """Runs a case's action with the `asyncio` client."""
    action = setup.action
    edit = edit_pipeline(setup, "async")
    kw = dict(setup.kwargs)
    if edit is not None:
        kw["pipeline"] = edit

    async def main() -> tuple[list[Result], Any]:
        client = (
            AsyncClient.load(load_options=setup.load, **kw) if setup.load else AsyncClient(**kw)
        )
        async with client:
            api = AsyncPublic(client)

            async def once() -> Result:
                if action["op"] == "events.stream_events":
                    return await stream_async(api, action)
                try:
                    with traced(action):
                        return await call_async(api, action)
                except InOrbitError as e:
                    return e

            if "concurrent" in action:
                got = list(await asyncio.gather(*(once() for _ in range(action["concurrent"]))))
                return got, client.config().describe()
            results: list[Result] = []
            for i in range(action.get("repeat", 1)):
                results.append(await once())
                rewrite(setup, i + 1)
            return results, client.config().describe()

    return asyncio.run(main())


# --- checking ------------------------------------------------------------------------


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


def check_stream(expect: dict[str, Any], r: Streamed) -> list[str]:
    """What differs between a stream's run and the case's `expect`."""
    problems: list[str] = []
    want: list[object] = lst(expect.get("items"))
    if not subset(want, r.items):
        problems.append(f"items: want {want}, got {r.items}")
    if "error" in expect:
        if r.error is None:
            problems.append("want the stream to end with an error, it ended cleanly")
        else:
            problems.extend(check_error(r.error, expect["error"]))
    elif r.error is not None:
        problems.append(f"want the stream to end cleanly, got {r.error}")
    return problems


def check_result(expect: dict[str, Any], r: RawResponse) -> list[str]:
    """`rate_limit` and `idempotency_key` on a result."""
    problems: list[str] = []
    if "rate_limit" in expect:
        rl = r.rate_limit
        got = (
            None
            if rl is None
            else {
                "limit": rl.limit,
                "remaining": rl.remaining,
                "reset_ms": None if rl.reset is None else round(rl.reset.total_seconds() * 1000),
            }
        )
        if got is None or not subset(expect["rate_limit"], got):
            problems.append(f"rate_limit: want {expect['rate_limit']}, got {got}")
    if "idempotency_key" in expect:
        want = expect["idempotency_key"]
        key = r.idempotency_key
        if not matches(want, key, {}) if want == "*" else key != want:
            problems.append(f"idempotency_key: want {want}, got {key}")
    return problems


def check_watched(expect: dict[str, Any], setup: Setup, config: Any) -> list[str]:
    """Probes, logs, spans and the described configuration."""
    problems: list[str] = []
    for name, want in obj(expect.get("probes")).items():
        seen = setup.probes.get(name, [])
        if "count" in want and len(seen) != want["count"]:
            problems.append(f"probe {name}: ran {len(seen)} times, want {want['count']}")
        captures: dict[str, str] = {}
        for i, headers in enumerate(lst(want.get("seen"))):
            got = seen[i] if i < len(seen) else {}
            for h, v in headers.items():
                if not matches(str(v), got.get(h), captures):
                    problems.append(
                        f"probe {name} #{i + 1}: {h} {got.get(h)!r} does not match {v!r}"
                    )
    logs = obj(expect.get("logs"))
    for want in lst(logs.get("contains")):
        if not any(subset(want, r) for r in setup.records):
            problems.append(f"logs: no record contains {want}; got {setup.records}")
    text = json.dumps(setup.records) + "\n".join(setup.messages)
    problems.extend(
        f"logs: {x!r} appears in a record" for x in lst(logs.get("excludes")) if x in text
    )
    if "spans" in expect:
        assert setup.exporter is not None
        spans: list[ReadableSpan] = list(setup.exporter.get_finished_spans())
        got_spans = [
            {
                "name": s.name,
                "kind": s.kind.name.lower(),
                "attributes": {
                    k: (str(v) if k == "error.type" else v) for k, v in (s.attributes or {}).items()
                },
            }
            for s in spans
        ]
        if len(got_spans) != len(expect["spans"]):
            problems.append(f"spans: want {len(expect['spans'])}, got {got_spans}")
        unused = list(got_spans)
        for want in expect["spans"]:
            found = next((s for s in unused if subset(want, s)), None)
            if found is None:
                problems.append(f"spans: none matches {want}; got {got_spans}")
            else:
                unused.remove(found)
    if "config" in expect and not subset(expect["config"], config):
        problems.append(f"config: want {expect['config']}, got {config}")
    return problems


def check(setup: Setup, results: list[Result], verdict: dict[str, Any], config: Any) -> list[str]:
    """What differs between a run and the case's `expect`."""
    expect: dict[str, Any] = setup.case["expect"]
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
        if isinstance(r, Streamed):
            problems.extend(check_stream(expect, r))
        elif isinstance(r, RawResponse):
            if "ok" in expect and not subset(expect["ok"], r.json()):
                problems.append(f"ok: want a superset of {expect['ok']}, got {r.json()}")
            elif "error" in expect:
                problems.append(f"want an error, got HTTP {r.status}")
        elif "error" in expect:
            problems.extend(check_error(r, expect["error"]))
        elif "ok" in expect:
            problems.append(f"want ok, got {r}")
    last = results[-1] if results else None
    if isinstance(last, RawResponse):
        problems.extend(check_result(expect, last))
    problems.extend(check_watched(expect, setup, config))
    return problems


RUNNERS: dict[str, Callable[[Setup], tuple[list[Result], Any]]] = {
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
        assert loaded.is_success, f"{name}: loading answered {loaded.status_code}"
        answer: dict[str, Any] = loaded.json()
        case: dict[str, Any] = answer["case"]
        if "py" in lst(case.get("pending")):
            continue
        ran.append(name)
        with tempfile.TemporaryDirectory() as d:
            setup = Setup(case, answer, Path(d))
            try:
                configure(setup)
                results, config = RUNNERS[form](setup)
            except InOrbitError as e:
                failed.append(f"{name}:\n  the client could not be built: {e}")
                continue
            verdict: dict[str, Any] = httpx.get(f"{replay}/_result").json()
            problems = check(setup, results, verdict, config)
        if problems:
            failed.append(f"{name}:\n  " + "\n  ".join(problems))
    assert not failed, "\n".join(failed)
    assert ran, "every case was skipped"
    print(f"{form}: {len(ran)} cases passed")


def _leaf_pin(url: str, ca_file: str) -> str:
    """The SPKI pin of the certificate the replay's TLS listener presents."""
    import socket
    import ssl

    from inorbithr._transport import (
        _spki_sha256,  # pyright: ignore[reportPrivateUsage]
    )

    u = httpx.URL(url)
    ctx = ssl.create_default_context(cafile=ca_file)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    with (
        socket.create_connection((u.host, u.port or 443)) as raw,
        ctx.wrap_socket(raw, server_hostname=u.host) as tls,
    ):
        der = tls.getpeercert(binary_form=True)
    assert der is not None
    return _spki_sha256(der)


@pytest.mark.parametrize("form", FORMS)
def test_pinned_keys_hold_the_connection_to_its_key(replay: str, form: str) -> None:
    """SR-06: a matching pin connects, a set of pins that matches nothing does not."""
    other = "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="  # SHA-256 of nothing
    for pins, ok in (([other, "PIN"], True), ([other, other], False)):
        answer = httpx.post(f"{replay}/_case", json={"name": "a-private-ca-is-trusted"}).json()
        url, ca = str(answer["https_url"]), str(answer["ca_file"])
        keys = [_leaf_pin(url, ca) if p == "PIN" else p for p in pins]
        options: dict[str, Any] = {
            "base_url": url,
            "token_url": f"{url}/oauth2/token",
            "key_id": "ak_test",
            "key_secret": "s3cr3t",
            "scopes": ["identity:read"],
            "ca_bundle": ca,
            "pinned_keys": keys,
            "max_retries": 0,
        }
        if form == "sync":
            with Client(**options) as c:
                try:
                    Public(c).me()
                    got = True
                except InOrbitError:
                    got = False
        else:

            async def main(options: dict[str, Any] = options) -> bool:
                async with AsyncClient(**options) as c:
                    try:
                        await AsyncPublic(c).me()
                    except InOrbitError:
                        return False
                    return True

            got = asyncio.run(main())
        assert got is ok, f"pins {pins}: want ok={ok}"
