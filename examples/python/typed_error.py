"""Typed error handling.

A refused call raises `ApiError` with the HTTP status, a stable code to branch on, the
problem text and the request id to quote to support.
"""

from inorbithr import ApiError, Public


def main() -> None:
    """Calls the API and reports a refusal."""
    api = Public.from_env()
    try:
        api.me()
    except ApiError as e:
        if e.code == "forbidden":
            print(f"refused: status {e.status}, code {e.code}, request {e.raw.request_id}")
        else:
            raise


if __name__ == "__main__":
    main()
