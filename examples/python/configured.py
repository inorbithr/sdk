"""A client built with `load`, a middleware of your own, and logging.

`Public.load()` takes each setting from code, then the `INORBIT_*` variables, then your
profile in the `iohr` config file, then the default (docs/config.md). Run `iohr login`
once, or set INORBIT_TOKEN.
"""

import logging
import time

from inorbithr import CallNext, Middleware, Pipeline, Public, SdkRequest, SdkResponse


class Timing:
    """Prints how long each attempt took. Per retry, it sees the finished request."""

    name = "timing"

    def __call__(self, request: SdkRequest, call_next: CallNext, /) -> SdkResponse:
        """Times the rest of the pipeline."""
        started = time.monotonic()
        response = call_next(request)
        ms = (time.monotonic() - started) * 1000
        print(
            f"{request.info.operation} attempt {request.info.attempt}: {response.status}, {ms:.0f} ms"
        )
        return response


def pipeline(p: Pipeline[Middleware]) -> None:
    """Adds `Timing` to every attempt, just before the attempt's timeout."""
    p.add_per_retry(Timing())


def main() -> None:
    """Prints the effective configuration, then the caller."""
    logging.basicConfig(level=logging.INFO)
    api = Public.load(timeout=10, log="info", pipeline=pipeline)
    print(api.client.config().to_json())
    me = api.me()
    print(f"subject {me.value.subject}, request id {me.request_id}, rate limit {me.rate_limit}")


if __name__ == "__main__":
    main()
