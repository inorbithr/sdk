"""The InOrbit API for Python: the runtime, and the public surface.

The surface here is generated from the API's public document; a surface cut to your own
credentials comes from `iohr sdk generate --lang python`.

Example:
    ```python
    from inorbithr import Public

    api = Public.from_env()
    me = api.me().value
    ```
"""

from inorbithr._generated import *  # noqa: F403 - the public surface, listed in its __all__
from inorbithr._generated import __all__ as _surface
from inorbithr.runtime import *  # noqa: F403 - the runtime, listed in its __all__
from inorbithr.runtime import __all__ as _runtime

__all__ = [*_runtime, *_surface]  # noqa: PLE0604  # pyright: ignore[reportUnsupportedDunderAll]
