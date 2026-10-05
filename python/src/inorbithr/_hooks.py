"""Observers of every attempt: logging, metrics, tracing. Run by the `hooks` middleware."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from inorbithr._errors import InOrbitError, RawResponse


@dataclass(frozen=True)
class Attempt:
    """One attempt of a call, as hooks see it. Never a header or a body."""

    operation: str
    """The operation (`radar.list_digests`), or the path for a raw call."""
    method: str
    """The HTTP method."""
    path: str
    """The path, parameters bound."""
    number: int
    """1 for the first attempt."""
    request_id: str
    """The `x-request-id` sent."""
    idempotency_key: str | None = None
    """The `Idempotency-Key` sent, the same on every attempt; `None` when the call has none."""
    stage: str = "per_retry"
    """The pipeline stage the hooks run in (`per_retry`)."""


class Hook:
    """Observes calls. Override the methods you need; each does nothing by default.

    Hooks observe; to change a request, write a middleware (`docs/config.md` section 7).
    """

    def on_request(self, attempt: Attempt) -> None:
        """Before an attempt is sent."""

    def on_response(self, attempt: Attempt, response: RawResponse) -> None:
        """After an answer arrived, whatever its status."""

    def on_error(self, attempt: Attempt, error: InOrbitError) -> None:
        """When the call fails for good."""

    def on_retry(self, attempt: Attempt, reason: str, delay: float) -> None:
        """Before the wait for a retry: the attempt that failed, why, and the wait in seconds."""
