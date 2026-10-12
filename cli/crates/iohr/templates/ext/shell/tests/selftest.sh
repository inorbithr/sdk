#!/bin/sh
# The self-test `iohr ext dev --test` runs, with the same token channel, STATE and
# CACHE as `iohr ext dev`: one heartbeat, then checks what it wrote and where.
set -eu

here="$(cd "$(dirname "$0")/.." && pwd)"
before="$(ls -A "$here")"

IOHR_EXT_ONCE=1 "$here/bin/@@NAME@@"

beat="$STATE/heartbeat.json"
[ -f "$beat" ] || { echo "no heartbeat at $beat"; exit 1; }
grep -q '"state":"ok"' "$beat" || { echo "the heartbeat is not ok: $(cat "$beat")"; exit 1; }
grep -q '"id":"@@ID@@"' "$beat" || { echo "the heartbeat names another id"; exit 1; }
# A token was asked for with the declared scope; under iohr ext dev it is granted when
# you are signed in, refused when not. Either way the channel answered.
grep -Eq '"token":"(granted|refused)"' "$beat" || { echo "the token channel did not answer: $(cat "$beat")"; exit 1; }

# Nothing written in the extension's own folder: state belongs in $STATE and $CACHE.
after="$(ls -A "$here")"
[ "$before" = "$after" ] || { echo "the extension wrote into its folder"; exit 1; }
echo "selftest ok"
