"""The streaming pieces on their own: the event-stream parser, the socket's call body,
the stream options and the codegen contract (design.md section 7)."""

from __future__ import annotations

import pytest

from inorbithr import Client, ConfigError, Operation, TooLargeError, codegen
from inorbithr._stream import SseParser, call_body, problem_error, socket_url


def feed_all(chunks: list[bytes]) -> list[tuple[str, str]]:
    parser = SseParser()
    out: list[tuple[str, str]] = []
    for chunk in chunks:
        out.extend(parser.feed(chunk))
    return out


def test_events_dispatch_on_a_blank_line_and_comments_and_other_fields_are_skipped() -> None:
    body = (
        b': open\n\nid: 7\nretry: 10\nfoo: bar\ndata: {"a":\ndata:  1}\n\nevent: error\ndata: x\n\n'
    )
    assert feed_all([body]) == [("message", '{"a":\n 1}'), ("error", "x")]


def test_lines_end_with_lf_crlf_or_cr_even_split_across_chunks() -> None:
    assert feed_all([b"data: a\r", b"\n\r\n", b"data: b\r\rdata: c\n\n"]) == [
        ("message", "a"),
        ("message", "b"),
        ("message", "c"),
    ]


def test_a_utf8_character_split_across_chunks_reads_whole() -> None:
    text = "data: čž\n\n".encode()
    assert feed_all([text[:8], text[8:]]) == [("message", "čž")]


def test_an_event_over_the_limit_is_refused() -> None:
    parser = SseParser(limit=16)
    with pytest.raises(TooLargeError):
        parser.feed(b"data: " + b"x" * 20 + b"\n")
    with pytest.raises(TooLargeError):
        SseParser(limit=16).feed(b"x" * 40)


def test_an_error_event_takes_its_status_from_its_code() -> None:
    import httpx

    e = problem_error(b'{"code":"forbidden","error":"no","details":[]}', httpx.Headers(), "r", 1)
    assert (e.code, e.status) == ("forbidden", 403)
    assert problem_error(b"not json", httpx.Headers(), "r", 1).status == 500


def test_the_call_body_holds_set_parameters_by_wire_name_nested_by_dots() -> None:
    assert call_body([("types", "a"), ("account_id", None), ("x.y", 2), ("l", ("p", "q"))]) == {
        "types": "a",
        "x": {"y": 2},
        "l": ["p", "q"],
    }
    assert socket_url("https://api.inorbit.hr") == "wss://api.inorbit.hr/v1/ws"
    assert socket_url("http://127.0.0.1:9") == "ws://127.0.0.1:9/v1/ws"


def test_stream_options_are_checked() -> None:
    with pytest.raises(ConfigError, match="streams"):
        Client(token="t", streams="carrier-pigeon")  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]
    with pytest.raises(ConfigError, match="stream_idle_timeout"):
        Client(token="t", stream_idle_timeout=0)
    client = Client(token="t", streams="socket")
    with pytest.raises(ConfigError, match="names no RPC"):
        client.stream(Operation(method="GET", path="/v1/x/events"), dict)
    client.close()


def test_a_surface_for_the_previous_contract_still_imports() -> None:
    codegen.check(1)
    codegen.check(codegen.VERSION)
    with pytest.raises(ImportError):
        codegen.check(codegen.VERSION + 1)
