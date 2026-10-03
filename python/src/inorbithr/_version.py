"""This package's version, as the user agent reports it."""

from importlib.metadata import PackageNotFoundError, version


def _installed() -> str:
    try:
        return version("inorbithr")
    except PackageNotFoundError:  # pragma: no cover - a source tree that is not installed
        return "0.0.0"


SDK_VERSION: str = _installed()
