"""Rate-limit headers: the edge's `X-RateLimit-*` and the IETF `RateLimit` fields.

`docs/config.md` section 7.8; the vectors in `conformance/vectors/rate-limit/`.
"""

from __future__ import annotations

import time
from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import timedelta


@dataclass(frozen=True)
class RateLimitPolicy:
    """The quota policy the IETF `RateLimit-Policy` field names."""

    name: str
    """The policy's name."""
    quota: int | None = None
    """Requests the policy allows per window (`q`)."""
    window: timedelta | None = None
    """The window (`w`)."""


@dataclass(frozen=True)
class RateLimit:
    """What the API said about this credential's rate limit on one answer."""

    limit: int | None = None
    """Requests allowed per window."""
    remaining: int | None = None
    """Requests left in the current window."""
    reset: timedelta | None = None
    """Time until the window resets, from when the answer arrived."""
    policy: RateLimitPolicy | None = None
    """The IETF policy, when the API sent one."""
    observed_at: float = field(default_factory=time.monotonic, repr=False, compare=False)
    """When the answer arrived, in `time.monotonic()` seconds."""

    def wait(self, now: float | None = None) -> float:
        """Seconds until the window resets, when nothing remains; else 0."""
        if self.remaining != 0 or self.reset is None:
            return 0.0
        at = (time.monotonic() if now is None else now) - self.observed_at
        return max(0.0, self.reset.total_seconds() - at)


def _int(value: str | None) -> int | None:
    if value is None:
        return None
    v = value.strip()
    return int(v) if v.isascii() and v.isdigit() else None


def _item(value: str) -> tuple[str, dict[str, str]] | None:
    """The first item of a structured-field list: its name and parameters."""
    first = value.split(",", 1)[0].strip()
    if not first:
        return None
    parts = [p.strip() for p in first.split(";")]
    name = parts[0]
    if len(name) >= 2 and name[0] == '"' and name[-1] == '"':
        name = name[1:-1]
    params: dict[str, str] = {}
    for p in parts[1:]:
        k, _, v = p.partition("=")
        if k:
            params[k.strip().lower()] = v.strip()
    return name, params


def parse_rate_limit(headers: Mapping[str, str]) -> RateLimit | None:
    """The snapshot an answer's headers give, or `None` when they give none.

    The IETF fields win when both kinds are sent; a malformed value is ignored.

    Args:
        headers: The answer's headers, any case.

    Returns:
        The snapshot, or `None`.
    """
    h = {k.lower(): v for k, v in headers.items()}
    if "ratelimit" in h:
        item = _item(h["ratelimit"])
        if item is not None:
            _, params = item
            remaining = _int(params.get("r"))
            t = _int(params.get("t"))
            policy: RateLimitPolicy | None = None
            if "ratelimit-policy" in h:
                pitem = _item(h["ratelimit-policy"])
                if pitem is not None:
                    w = _int(pitem[1].get("w"))
                    policy = RateLimitPolicy(
                        pitem[0],
                        _int(pitem[1].get("q")),
                        None if w is None else timedelta(seconds=w),
                    )
            if remaining is not None:
                return RateLimit(
                    limit=policy.quota if policy is not None else None,
                    remaining=remaining,
                    reset=None if t is None else timedelta(seconds=t),
                    policy=policy,
                )
    names = ("x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset")
    if not any(n in h for n in names):
        return None
    limit, remaining, reset = (_int(h.get(n)) for n in names)
    if any(
        h.get(n) is not None and v is None
        for n, v in zip(names, (limit, remaining, reset), strict=True)
    ):
        return None
    return RateLimit(
        limit=limit,
        remaining=remaining,
        reset=None if reset is None else timedelta(seconds=reset),
    )
