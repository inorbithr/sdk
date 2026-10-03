"""The runtime on its own: credentials, errors, retries, limits and the codegen contract."""

from __future__ import annotations

import asyncio
import json
import threading
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor

import httpx
import pytest
from pydantic import BaseModel

from inorbithr import (
    MAX_BODY,
    ApiError,
    AsyncClient,
    AsyncClientCredentials,
    Client,
    ClientCredentials,
    ConfigError,
    Int64,
    Operation,
    RetryDetail,
    StaticToken,
    TooLargeError,
    UnknownDetail,
    codegen,
)
from inorbithr._errors import CODES, code_for_status
from inorbithr._retry import backoff

TOKEN_ANSWER = {"access_token": "tok-123", "token_type": "bearer", "expires_in": 900}


def handler(
    counts: dict[str, int], answer: httpx.Response
) -> Callable[[httpx.Request], httpx.Response]:
    """A transport that answers the token endpoint and `answer` for everything else."""
    lock = threading.Lock()

    def respond(request: httpx.Request) -> httpx.Response:
        key = "token" if request.url.path == "/oauth2/token" else "api"
        with lock:
            counts[key] = counts.get(key, 0) + 1
        if key == "token":
            return httpx.Response(200, json=TOKEN_ANSWER)
        return answer

    return respond


def key_client(transport: httpx.MockTransport, **options: object) -> Client:
    return Client(
        key_id="ak_test",
        key_secret="s3cr3t",
        scopes=["identity:read"],
        base_url="https://api.example.test",
        token_url="https://api.example.test/oauth2/token",
        http_client=httpx.Client(transport=transport),
        **options,  # type: ignore[arg-type]
    )


def test_secrets_never_print() -> None:
    assert "s3cr3t" not in repr(
        ClientCredentials(key_id="ak_1", key_secret="s3cr3t", scopes=["identity:read"])
    )
    assert "s3cr3t" not in repr(
        AsyncClientCredentials(key_id="ak_1", key_secret="s3cr3t", scopes=["identity:read"])
    )
    assert "tok-1" not in repr(StaticToken("tok-1"))
    assert "tok-1" not in repr(StaticToken("tok-1").token())


def test_a_named_profile_reads_only_its_own_variables(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("INORBIT_TOKEN", "bare")
    for name in ("INORBIT_ACME_CI_TOKEN", "INORBIT_ACME_CI_KEY_ID"):
        monkeypatch.delenv(name, raising=False)
    with pytest.raises(ConfigError, match="INORBIT_ACME_CI_TOKEN"):
        Client.from_env("ACME_CI")
    monkeypatch.setenv("INORBIT_ACME_CI_KEY_ID", "ak_1")
    monkeypatch.setenv("INORBIT_ACME_CI_KEY_SECRET", "s")
    with pytest.raises(ConfigError, match="INORBIT_ACME_CI_SCOPES"):
        Client.from_env("ACME_CI")
    assert Client.from_env().base_url == "https://api.inorbit.hr"


def test_plain_http_only_to_this_machine() -> None:
    with pytest.raises(ConfigError, match="https"):
        Client(token="t", base_url="http://api.example.test")
    with pytest.raises(ConfigError, match="origin only"):
        Client(token="t", base_url="https://api.example.test/v1")
    assert Client(token="t", base_url="http://127.0.0.1:8080").base_url == "http://127.0.0.1:8080"


def test_one_exchange_for_many_threads() -> None:
    counts: dict[str, int] = {}
    transport = httpx.MockTransport(handler(counts, httpx.Response(200, json={"ok": True})))
    client = key_client(transport)
    op = Operation(method="GET", path="/v1/me")
    with ThreadPoolExecutor(max_workers=10) as pool:
        for future in [pool.submit(client.send, op) for _ in range(10)]:
            future.result()
    assert counts == {"token": 1, "api": 10}


def test_one_exchange_for_many_tasks() -> None:
    counts: dict[str, int] = {}
    transport = httpx.MockTransport(handler(counts, httpx.Response(200, json={"ok": True})))

    async def main() -> None:
        async with AsyncClient(
            key_id="ak_test",
            key_secret="s3cr3t",
            scopes=["identity:read"],
            base_url="https://api.example.test",
            token_url="https://api.example.test/oauth2/token",
            http_client=httpx.AsyncClient(transport=transport),
        ) as client:
            op = Operation(method="GET", path="/v1/me")
            await asyncio.gather(*(client.send(op) for _ in range(10)))

    asyncio.run(main())
    assert counts == {"token": 1, "api": 10}


def test_a_401_refreshes_once_then_surfaces() -> None:
    counts: dict[str, int] = {}
    transport = httpx.MockTransport(handler(counts, httpx.Response(401, text="Jwt is expired")))
    client = key_client(transport)
    with pytest.raises(ApiError) as caught:
        client.send(Operation(method="GET", path="/v1/me"))
    assert caught.value.code == "unauthenticated"
    assert counts == {"token": 2, "api": 2}


def test_a_post_is_not_retried_and_a_get_is() -> None:
    counts: dict[str, int] = {}
    unavailable = httpx.Response(503, headers={"retry-after": "0"}, json={"code": "unavailable"})
    client = key_client(httpx.MockTransport(handler(counts, unavailable)))
    with pytest.raises(ApiError):
        client.send(Operation(method="POST", path="/v1/things", body={"a": 1}))
    assert counts["api"] == 1
    with pytest.raises(ApiError):
        client.send(Operation(method="GET", path="/v1/things"))
    assert counts["api"] == 1 + 3


def test_unknown_codes_and_details_are_kept() -> None:
    body = {
        "code": "brand_new",
        "error": "nope",
        "details": [{"type": "retry", "after_seconds": 7}, {"type": "future", "x": 1}],
    }
    client = key_client(httpx.MockTransport(handler({}, httpx.Response(409, json=body))))
    with pytest.raises(ApiError) as caught:
        client.send(Operation(method="GET", path="/v1/me"))
    e = caught.value
    assert e.code == "brand_new"
    assert e.details[0] == RetryDetail(7)
    assert isinstance(e.details[1], UnknownDetail)
    assert e.retry_after_seconds() == 7
    assert "s3cr3t" not in str(e)


def test_an_answer_over_the_cap_is_refused() -> None:
    big = httpx.Response(200, headers={"content-length": str(MAX_BODY + 1)}, content=b"{}")
    client = key_client(httpx.MockTransport(handler({}, big)))
    with pytest.raises(TooLargeError):
        client.send(Operation(method="GET", path="/v1/me"))


def test_query_values_and_body_go_on_the_wire() -> None:
    seen: list[httpx.Request] = []

    def respond(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={})

    client = Client(
        token="t",
        base_url="https://api.example.test",
        http_client=httpx.Client(transport=httpx.MockTransport(respond)),
    )
    client.send(
        Operation(
            method="POST",
            path="/v1/x",
            query=(("tag", ["a", "b"]), ("drafts", True), ("gone", None)),
            body={"n": 1},
        )
    )
    request = seen[0]
    assert request.url.query == b"tag=a&tag=b&drafts=true"
    assert json.loads(request.content) == {"n": 1}
    assert request.headers["authorization"] == "Bearer t"
    assert request.headers["x-request-id"].startswith("iohr-")


def test_int64_reads_both_forms_and_writes_a_string() -> None:
    class Row(BaseModel):
        units: Int64

    assert Row.model_validate({"units": "9007199254740993"}).units == 9007199254740993
    assert Row.model_validate({"units": 5}).units == 5
    assert json.loads(Row(units=9007199254740993).model_dump_json()) == {
        "units": "9007199254740993"
    }


def test_a_path_parameter_is_one_segment() -> None:
    assert codegen.path_segment("a/b c?") == "a%2Fb%20c%3F"
    assert codegen.path_segment("ok-._~") == "ok-._~"
    with pytest.raises(ImportError, match="iohr sdk generate"):
        codegen.check(codegen.VERSION + 1)


def test_backoff_stays_under_its_ceiling() -> None:
    assert all(0 <= backoff(n) <= 8.0 for n in range(12) for _ in range(20))


def test_unprocessable_is_known_and_an_unknown_code_is_kept() -> None:
    assert CODES["unprocessable"] == 422
    assert code_for_status(422) == "unprocessable"
    assert "brand_new_code" not in CODES
