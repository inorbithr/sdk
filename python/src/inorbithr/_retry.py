"""The retry rules of design.md section 6, shared by both clients."""

from __future__ import annotations

import random
import secrets

import httpx

#: `Retry-After` is honoured up to a minute.
RETRY_AFTER_CAP = 60.0
_BACKOFF_BASE = 0.5
_BACKOFF_CAP = 8.0


def retryable_status(status: int) -> bool:
    """Whether an answer with `status` is worth another attempt on an idempotent call."""
    return status in (429, 503, 504)


def retry_after(headers: httpx.Headers) -> float | None:
    """The wait `Retry-After` asks for, in seconds, capped; whole seconds only."""
    value = headers.get("retry-after", "").strip()
    if not value.isdigit():
        return None
    return min(float(value), RETRY_AFTER_CAP)


def backoff(retry: int) -> float:
    """Full jitter: a random wait up to 0.5 s doubled per retry, at most 8 s."""
    ceiling = min(_BACKOFF_BASE * 2 ** min(retry, 5), _BACKOFF_CAP)
    # Jitter only spreads retries out; it is not a secret.
    return random.uniform(0, ceiling)  # noqa: S311


def request_id() -> str:
    """`iohr-<16 hex>`, the id each call is sent with."""
    return f"iohr-{secrets.token_hex(8)}"
