#!/usr/bin/env bash
# PostToolUse on Edit|Write: format the file Claude just changed, with the repo's own
# formatter for its language. Never blocks: a formatter failure is reported, not fatal.
set -uo pipefail

file=$(jq -r '.tool_input.file_path // empty')
[ -n "$file" ] && [ -f "$file" ] || exit 0
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

run() { mise exec -- "$@" >/dev/null 2>&1 || echo "format-file: '$*' failed on $file" >&2; }

case "$file" in
  */generated/*|*/_generated/*|*/_sync/*) exit 0 ;;
  *.go)               run gofmt -w "$file" ;;
  *.rs)               run rustfmt --edition 2024 "$file" ;;
  *.ts|*.tsx|*.mts|*.js|*.mjs|*.json)
                      case "$file" in */typescript/*|*/examples/typescript/*) run biome format --write "$file" ;; esac ;;
  *.py)               run uv run --directory python ruff format "$file" ;;
esac
exit 0
