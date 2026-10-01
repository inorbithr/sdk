#!/usr/bin/env bash
# Stop: before Claude finishes a turn, run the quick checks for every language whose
# files changed. On failure, exit 2 so Claude sees the output and keeps working.
# The same set of changes is only blocked once, so a check Claude cannot fix does not
# trap the session in a loop; it gets reported instead.
set -uo pipefail

input=$(cat)
session=$(jq -r '.session_id // "unknown"' <<<"$input")
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0
git rev-parse --git-dir >/dev/null 2>&1 || exit 0

changed=$( { git diff --name-only HEAD 2>/dev/null; git ls-files --others --exclude-standard; } | sort -u)
[ -n "$changed" ] || exit 0

tasks=()
grep -q '^go/'          <<<"$changed" && tasks+=("go:check")
grep -q '^rust/'        <<<"$changed" && tasks+=("rust:check")
grep -q '^typescript/'  <<<"$changed" && tasks+=("ts:check")
grep -q '^python/'      <<<"$changed" && tasks+=("py:check")
grep -q '^conformance/' <<<"$changed" && tasks+=("conformance:validate")
grep -q '^\.github/'    <<<"$changed" && tasks+=("repo:check")
[ ${#tasks[@]} -gt 0 ] || exit 0

state="${TMPDIR:-/tmp}/sdk-stop-check-$session"
fingerprint=$( { git diff HEAD; git ls-files --others --exclude-standard | xargs -r cat; } 2>/dev/null | sha256sum | cut -c1-16)

out=$(for t in "${tasks[@]}"; do mise run "$t" 2>&1 || echo "FAILED: $t"; done)
if grep -q '^FAILED: ' <<<"$out"; then
  if [ "$(cat "$state" 2>/dev/null)" = "$fingerprint" ]; then
    echo "stop-check: same failures as last time; stopping anyway. Tell the user what still fails." >&2
    exit 0
  fi
  echo "$fingerprint" > "$state"
  { echo "stop-check: checks failed for the files you changed. Fix them, or explain why not."; tail -n 60 <<<"$out"; } >&2
  exit 2
fi
rm -f "$state"
exit 0
