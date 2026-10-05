"""The retry rules of design.md section 6 and config.md section 7.4, shared by both clients."""

from __future__ import annotations

import email.utils
import random
import secrets
import threading
import time

import httpx

#: `Retry-After` on the token exchange and the socket upgrade is honoured up to a minute.
RETRY_AFTER_CAP = 60.0
_BACKOFF_BASE = 0.5
_BACKOFF_CAP = 8.0

#: The retry budget's capacity (config.md section 7.4).
BUDGET_CAPACITY = 500
#: What a retry after a 429, or a 503 with `Retry-After`, costs.
COST_THROTTLED = 5
#: What any other retry costs.
COST_OTHER = 10


def retryable_status(status: int) -> bool:
    """Whether an answer with `status` is worth another attempt on an idempotent call."""
    return status in (429, 503, 504)


def retry_after_seconds(headers: httpx.Headers) -> float | None:
    """The wait `Retry-After` asks for, as delay-seconds or an HTTP date; not capped."""
    value = headers.get("retry-after", "").strip()
    if not value:
        return None
    if value.isascii() and value.isdigit():
        return float(value)
    try:
        when = email.utils.parsedate_to_datetime(value)
    except (TypeError, ValueError, IndexError):
        return None
    if when.tzinfo is None:
        return None
    return max(0.0, when.timestamp() - time.time())


def retry_after(headers: httpx.Headers) -> float | None:
    """The wait `Retry-After` asks for, in seconds, at most a minute."""
    wait = retry_after_seconds(headers)
    return None if wait is None else min(wait, RETRY_AFTER_CAP)


def backoff(retry: int, base: float = _BACKOFF_BASE, cap: float = _BACKOFF_CAP) -> float:
    """Full jitter: a random wait up to `base` doubled per retry, at most `cap`."""
    ceiling = min(base * 2 ** min(retry, 30), cap)
    # Jitter only spreads retries out; it is not a secret.
    return random.uniform(0, ceiling)  # noqa: S311


def request_id() -> str:
    """`iohr-<16 hex>`, the id each call is sent with."""
    return f"iohr-{secrets.token_hex(8)}"


class RetryBudget:
    """The per-client retry quota: a token bucket after AWS's standard retry mode.

    A retry draws its cost; a call that succeeds after retries returns the cost of its
    last retry, and one that succeeds on the first attempt adds 1, up to the capacity.
    """

    def __init__(self, capacity: int = BUDGET_CAPACITY, *, enabled: bool = True) -> None:
        """A full bucket of `capacity`; a disabled budget always pays."""
        self.capacity = capacity
        self.enabled = enabled
        self._level = capacity
        self._lock = threading.Lock()

    @property
    def level(self) -> int:
        """What is left in the bucket."""
        return self._level

    def draw(self, cost: int) -> bool:
        """Takes `cost` for a retry; `False` when the bucket cannot pay."""
        if not self.enabled:
            return True
        with self._lock:
            if self._level < cost:
                return False
            self._level -= cost
            return True

    def refund(self, amount: int) -> None:
        """Puts `amount` back, up to the capacity."""
        if not self.enabled:
            return
        with self._lock:
            self._level = min(self.capacity, self._level + amount)
