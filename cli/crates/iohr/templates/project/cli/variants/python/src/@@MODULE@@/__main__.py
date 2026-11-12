"""`python -m @@MODULE@@` and the `@@NAME@@` script."""

import sys

from .cli import run


def main() -> int:
    return run(sys.argv[1:], sys.stdout, sys.stderr)


if __name__ == "__main__":
    sys.exit(main())
