"""M6 on its own: `load`, credentials, the pipeline, retries, logging (docs/config.md)."""

from __future__ import annotations

import asyncio
import base64
import json
import logging
import re
import sys
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import httpx
import pytest

from inorbithr import (
    ApiError,
    AsyncClient,
    AsyncPublic,
    AsyncTokenFile,
    AuthError,
    CachedToken,
    CallNext,
    ChainedCredential,
    Client,
    CliToken,
    ConfigError,
    Hook,
    LoadOptions,
    Middleware,
    Operation,
    Pipeline,
    Public,
    SdkRequest,
    SdkResponse,
    StaticToken,
    Token,
    TokenFile,
    call_options,
)
from inorbithr._retry import RetryBudget, retry_after_seconds
from inorbithr._transport import user_agent

ME = Operation(method="GET", path="/v1/me", name="me")
TOKEN = {"access_token": "tok-secret-9", "token_type": "bearer", "expires_in": 900}


def answers(*responses: httpx.Response) -> tuple[httpx.MockTransport, list[httpx.Request]]:
    """A transport answering the token endpoint, then `responses` in order (the last repeats)."""
    seen: list[httpx.Request] = []
    queue = list(responses)

    def respond(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/oauth2/token":
            return httpx.Response(200, json=TOKEN)
        seen.append(request)
        return queue.pop(0) if len(queue) > 1 else queue[0]

    return httpx.MockTransport(respond), seen


def client(transport: httpx.MockTransport, **options: Any) -> Client:
    return Client(
        key_id="ak_test",
        key_secret="s3cr3t",
        scopes=["identity:read"],
        base_url="https://api.example.test",
        token_url="https://api.example.test/oauth2/token",
        http_client=httpx.Client(transport=transport),
        **options,
    )


# --- load and describe ---------------------------------------------------------------


def test_describe_shows_sources_and_never_a_secret(tmp_path: Path) -> None:
    env = {
        "INORBIT_KEY_ID": "ak_shown",
        "INORBIT_KEY_SECRET": "s3cr3t-do-not-print",
        "INORBIT_SCOPES": "identity:read",
        "INORBIT_PROXY": "http://ana:pw-do-not-print@proxy.example:3128",
        "INORBIT_CONFIG_FILE": "off",
    }
    c = Client.load(timeout=5, load_options=LoadOptions(env=env, home="", cwd=str(tmp_path)))
    doc = c.config().describe()
    text = json.dumps(doc) + repr(c.config())
    assert "s3cr3t-do-not-print" not in text
    assert "pw-do-not-print" not in text
    assert doc["settings"]["timeout"] == {"value": "5s", "source": "code"}
    assert doc["settings"]["key_id"]["value"] == "ak_shown"
    assert doc["credential"]["kind"] == "client_credentials"
    assert doc["pipeline"][5] == "retry"


def test_an_explicit_client_describes_itself_from_code() -> None:
    c = Client(token="t", timeout=7, total_timeout=60)
    doc = c.config().describe()
    assert doc["settings"]["timeout"] == {"value": "7s", "source": "code"}
    assert doc["settings"]["total_timeout"] == {"value": "60s", "source": "code"}
    assert doc["settings"]["max_retries"] == {"value": 2, "source": "default"}
    assert doc["credential"]["source"] == "code"


def test_the_public_profile_loads(tmp_path: Path) -> None:
    env = {"INORBIT_TOKEN": "t", "INORBIT_CONFIG_FILE": "off"}
    api = Public.load(load_options=LoadOptions(env=env, home="", cwd=str(tmp_path)))
    assert api.client.config().describe()["credential"]["kind"] == "static_token"
    aapi = AsyncPublic.load(load_options=LoadOptions(env=env, home=""))
    assert aapi.client.base_url == "https://api.inorbit.hr"


def test_a_typed_profile_reads_its_own_prefix() -> None:
    env = {"INORBIT_ACME_CI_TOKEN": "t", "INORBIT_CONFIG_FILE": "off"}
    c = Client.load(profile_type="acme-ci", load_options=LoadOptions(env=env, home=""))
    assert c.config().describe()["profile"]["name"] == "acme-ci"
    with pytest.raises(ConfigError, match="typed profile"):
        Client.load(profile_type="acme-ci", profile="x", load_options=LoadOptions(env=env))


def test_from_env_normalises_the_profile_name(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("INORBIT_ACME_CI_TOKEN", "t")
    assert Client.from_env("acme-ci").base_url == "https://api.inorbit.hr"


def test_transport_settings_next_to_an_http_client_are_refused() -> None:
    with pytest.raises(ConfigError, match="http_client") as caught:
        Client(token="t", http_client=httpx.Client(), proxy="http://proxy.example:3128")
    assert caught.value.problems[0].setting == "proxy"


def test_an_explicit_proxy_and_trust_build_the_clients_own_transport() -> None:
    c = Client(token="t", proxy="http://user:pw@proxy.example:3128", no_proxy="localhost")
    doc = c.config().describe()
    assert doc["settings"]["proxy"]["value"] == "http://<redacted>@proxy.example:3128"
    assert doc["settings"]["no_proxy"]["value"] == ["localhost"]


# --- the pipeline --------------------------------------------------------------------


class Named:
    def __init__(self, name: str) -> None:
        self.name = name

    def __call__(self, request: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        return call_next(request)


def test_the_pipeline_is_edited_by_name() -> None:
    def edit(p: Pipeline[Any]) -> None:
        p.add_per_call(Named("breaker"))
        p.add_per_retry(Named("probe"))
        p.insert_before("auth", Named("first"))
        p.insert_after("hooks", Named("late"))
        p.replace("rate_limit", Named("other"))
        p.remove("logging")

    names = Client(token="t", pipeline=edit).config().describe()["pipeline"]
    assert names == [
        "request_id",
        "user_agent",
        "idempotency_key",
        "call_tracing",
        "deadline",
        "breaker",
        "retry",
        "first",
        "auth",
        "rate_limit",
        "attempt_tracing",
        "hooks",
        "late",
        "probe",
        "timeout",
    ]


def _remove_retry(p: Pipeline[Middleware]) -> None:
    p.remove("retry")


def _remove_timeout(p: Pipeline[Middleware]) -> None:
    p.remove("timeout")


def _duplicate(p: Pipeline[Middleware]) -> None:
    p.add_per_call(Named("auth"))


def _unknown(p: Pipeline[Middleware]) -> None:
    p.insert_before("nope", Named("x"))


@pytest.mark.parametrize(
    ("edit", "message"),
    [
        (_remove_retry, "cannot be removed"),
        (_remove_timeout, "cannot be removed"),
        (_duplicate, "already has"),
        (_unknown, "no middleware named"),
    ],
)
def test_a_wrong_edit_is_a_config_error(
    edit: Callable[[Pipeline[Middleware]], None], message: str
) -> None:
    with pytest.raises(ConfigError, match=message):
        Client(token="t", pipeline=edit)


def test_a_middleware_may_answer_without_the_network() -> None:
    transport, seen = answers(httpx.Response(500))

    class Canned:
        name = "canned"

        def __call__(self, request: SdkRequest, call_next: CallNext, /) -> SdkResponse:
            assert request.info.attempt == 1
            assert request.headers["authorization"] == "Bearer tok-secret-9"
            return SdkResponse(200, httpx.Headers({"x-request-id": "srv-1"}), b'{"ok": true}')

    def edit(p: Pipeline[Middleware]) -> None:
        p.add_per_retry(Canned())

    raw = client(transport, pipeline=edit).send(ME)
    assert raw.json() == {"ok": True}
    assert raw.server_request_id == "srv-1"
    assert seen == []


# --- retries -------------------------------------------------------------------------


def test_a_retry_after_over_the_limit_ends_the_call() -> None:
    transport, seen = answers(httpx.Response(429, headers={"retry-after": "120"}))
    with pytest.raises(ApiError):
        client(transport).send(ME)
    assert len(seen) == 1


def test_retry_after_reads_an_http_date() -> None:
    soon = time.strftime("%a, %d %b %Y %H:%M:%S GMT", time.gmtime(time.time() + 30))
    wait = retry_after_seconds(httpx.Headers({"retry-after": soon}))
    assert wait is not None
    assert 25 < wait <= 30


def test_a_conflict_with_a_key_is_not_retried() -> None:
    transport, seen = answers(httpx.Response(409, json={"code": "conflict"}))
    write = Operation(method="POST", path="/v1/x", body={}, name="x.create", idempotency_key=True)
    with pytest.raises(ApiError) as caught:
        client(transport).send(write, idempotency_key="k-1")
    assert len(seen) == 1
    assert seen[0].headers["idempotency-key"] == "k-1"
    assert caught.value.idempotency_key == "k-1"


def test_a_key_for_an_operation_that_takes_none_is_refused() -> None:
    transport, _ = answers(httpx.Response(200, json={}))
    with pytest.raises(ConfigError, match="does not take an idempotency key"):
        client(transport).send(Operation(method="POST", path="/v1/x"), idempotency_key="k")


def test_on_retry_sees_the_reason_and_the_wait() -> None:
    seen_retries: list[tuple[int, str, float]] = []

    class Watch(Hook):
        def on_retry(self, attempt: Any, reason: str, delay: float) -> None:
            seen_retries.append((attempt.number, reason, delay))

    transport, _ = answers(
        httpx.Response(503, headers={"retry-after": "0"}), httpx.Response(200, json={})
    )
    client(transport, hooks=[Watch()]).send(ME)
    assert seen_retries == [(1, "503", 0.0)]


def test_the_retry_budget_drains_and_refills() -> None:
    b = RetryBudget(12)
    assert b.draw(10)
    assert not b.draw(5)
    b.refund(10)
    assert b.level == 12
    assert RetryBudget(0, enabled=False).draw(10)


def test_the_rate_limit_snapshot_is_on_the_result_and_the_client() -> None:
    headers = {"x-ratelimit-limit": "10", "x-ratelimit-remaining": "4", "x-ratelimit-reset": "2"}
    transport, _ = answers(httpx.Response(200, headers=headers, json={}))
    c = client(transport)
    raw = c.send(ME)
    assert raw.rate_limit is not None
    assert (raw.rate_limit.limit, raw.rate_limit.remaining) == (10, 4)
    assert c.rate_limit() == raw.rate_limit


def test_a_static_token_refused_is_an_auth_error() -> None:
    def respond(_: httpx.Request) -> httpx.Response:
        return httpx.Response(401, text="Jwt is expired")

    c = Client(
        token="t",
        base_url="https://api.example.test",
        http_client=httpx.Client(transport=httpx.MockTransport(respond)),
    )
    with pytest.raises(AuthError, match="refused the token"):
        c.send(ME)


def test_the_total_timeout_ends_an_async_call() -> None:
    async def respond(_: httpx.Request) -> httpx.Response:
        await asyncio.sleep(1)
        return httpx.Response(200, json={})

    async def main() -> None:
        async with AsyncClient(
            token="t",
            base_url="https://api.example.test",
            total_timeout=0.2,
            http_client=httpx.AsyncClient(transport=httpx.MockTransport(respond)),
        ) as c:
            started = time.monotonic()
            with pytest.raises(Exception, match="did not answer"):
                await c.send(ME)
            assert time.monotonic() - started < 0.9

    asyncio.run(main())


# --- logging -------------------------------------------------------------------------


def test_logs_hold_metadata_and_never_a_secret_or_a_body() -> None:
    records: list[logging.LogRecord] = []

    class Keep(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            records.append(record)

    logger = logging.getLogger("inorbithr.test.logs")
    logger.propagate = False
    logger.setLevel(logging.DEBUG)
    logger.addHandler(Keep())
    transport, _ = answers(
        httpx.Response(
            200, headers={"x-ratelimit-remaining": "9", "set-cookie": "c=1"}, json={"a": 1}
        )
    )
    op = Operation(
        method="POST",
        path="/v1/x",
        name="x.make",
        query=(("q", "private"),),
        body={"b": "body-marker"},
    )
    client(transport, log="debug", log_headers=True, logger=logger).send(op)
    text = " ".join(r.getMessage() for r in records) + json.dumps(
        [getattr(r, "inorbithr", {}) for r in records]
    )
    for secret in ("tok-secret-9", "s3cr3t", "body-marker", "private", "c=1", "Bearer"):
        assert secret not in text
    events = [getattr(r, "event", None) for r in records]
    assert events == ["request", "response", "call"]
    response = records[1].__dict__["inorbithr"]
    assert response["headers"]["x-ratelimit-remaining"] == "9"
    assert response["headers"]["set-cookie"] == "REDACTED"
    assert response["path"] == "/v1/x"


def test_the_user_agent_uses_the_shared_vocabulary() -> None:
    ua = user_agent("myapp/1.2")
    assert re.fullmatch(
        r"inorbithr-sdk-python/\S+ python/\S+ "
        r"(linux|macos|windows|freebsd|android|ios|other)/(x86_64|aarch64|x86|arm|riscv64|other) "
        r"myapp/1\.2",
        ua,
    ), ua


# --- credentials ---------------------------------------------------------------------


class Flaky:
    """A provider that gives one short-lived token, then fails."""

    def __init__(self) -> None:
        self.calls = 0

    def token(self) -> Token:
        self.calls += 1
        if self.calls > 1:
            raise AuthError("the vault is down")
        return Token("vault-1", time.time() + 0.5)

    def invalidate(self) -> None:
        pass


def test_a_failed_refresh_keeps_a_valid_token(monkeypatch: pytest.MonkeyPatch) -> None:
    flaky = Flaky()
    cached = CachedToken(flaky)
    assert cached.token().access == "vault-1"
    now = time.time()
    monkeypatch.setattr(time, "time", lambda: now + 0.45)  # past 80 %, still valid
    assert cached.token().access == "vault-1"
    assert flaky.calls == 2
    assert cached.token().access == "vault-1"  # no new try within 5 s
    assert flaky.calls == 2
    monkeypatch.setattr(time, "time", lambda: now + 10)  # expired
    with pytest.raises(AuthError, match="vault is down"):
        cached.token()


def test_a_token_file_is_read_again_when_refused(tmp_path: Path) -> None:
    f = tmp_path / "token"
    f.write_text("tok-1\n")
    tf = TokenFile(f)
    assert tf.token().access == "tok-1"
    f.write_text("tok-2")
    assert tf.token().access == "tok-1"  # checked at most once a minute
    tf.invalidate()
    assert tf.token().access == "tok-2"
    f.unlink()
    assert tf.token().access == "tok-2"  # kept until refused
    tf.invalidate()
    with pytest.raises(AuthError, match="cannot read the token file"):
        tf.token()
    assert "tok" not in repr(tf).replace(repr(str(f)), "")  # repr doubles a Windows \\
    assert asyncio.run(AsyncTokenFile(tmp_path / "missing").invalidate()) is None


def test_a_jwt_in_a_token_file_carries_its_expiry(tmp_path: Path) -> None:
    claims = base64.urlsafe_b64encode(json.dumps({"exp": 2_000_000_000}).encode()).rstrip(b"=")
    f = tmp_path / "jwt"
    f.write_text(f"e30.{claims.decode()}.sig")
    assert TokenFile(f).token().expires_at == 2_000_000_000


@pytest.mark.skipif(sys.platform == "win32", reason="a shebang script")
def test_the_cli_source_runs_iohr(tmp_path: Path) -> None:
    script = tmp_path / "iohr"
    script.write_text(
        f"#!{sys.executable}\n"
        "import json, sys\n"
        "p = sys.argv[sys.argv.index('--profile') + 1]\n"
        "if p == 'missing':\n"
        "    print('not signed in: run iohr login', file=sys.stderr); sys.exit(3)\n"
        "print(json.dumps({'access_token': 'cli-' + p, 'expires_at': '2099-01-01T00:00:00Z', "
        "'profile': p}))\n"
    )
    script.chmod(0o755)
    tok = CliToken("dev", cli_path=str(script)).token()
    assert tok.access == "cli-dev"
    assert tok.expires_at is not None
    with pytest.raises(AuthError, match="not signed in"):
        CliToken("missing", cli_path=str(script)).token()
    with pytest.raises(AuthError, match="cannot run"):
        CliToken("dev", cli_path=str(tmp_path / "nope")).token()


def test_a_chain_names_every_failure() -> None:
    chain = ChainedCredential(Flaky(), StaticToken("t"))
    assert chain.token().access == "vault-1"
    failing = Flaky()
    failing.calls = 1
    with pytest.raises(AuthError, match="vault is down"):
        ChainedCredential(failing).token()


def test_a_callers_traceparent_survives_an_api_without_an_sdk() -> None:
    seen: list[httpx.Request] = []

    def respond(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={})

    tp = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0cb902b7-01"
    c = Client(
        token="t",
        base_url="https://api.example.test",
        tracing=True,
        http_client=httpx.Client(transport=httpx.MockTransport(respond)),
    )
    with call_options(traceparent=tp):
        c.send(ME)
    assert seen[0].headers["traceparent"] == tp
