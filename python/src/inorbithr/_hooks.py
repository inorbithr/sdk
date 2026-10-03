"""Observers of every attempt: logging, metrics, tracing."""

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


class Hook:
    """Observes calls. Override the methods you need; each does nothing by default."""

    def on_request(self, attempt: Attempt) -> None:
        """Before an attempt is sent."""

    def on_response(self, attempt: Attempt, response: RawResponse) -> None:
        """After an answer arrived, whatever its status."""

    def on_error(self, attempt: Attempt, error: InOrbitError) -> None:
        """When the call fails for good."""
