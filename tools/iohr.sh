#!/usr/bin/env bash
# The command line for the `<lang>:gen` tasks: the prebuilt binary CI hands over in
# IOHR_BIN (an absolute path), otherwise built from cli/ on the spot, as on a laptop.
set -euo pipefail
if [ -n "${IOHR_BIN:-}" ]; then
  exec "$IOHR_BIN" "$@"
fi
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
exec cargo run -q --manifest-path "$root/cli/Cargo.toml" -p iohr -- "$@"
