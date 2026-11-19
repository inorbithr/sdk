# 0018. A passing `ci:changed` leaves the platform's gate record

Status: proposed, 2026-11-19

## Context

The platform's PR guard (platform PRD 0221, RFC 0153, ADR 0193) holds a ready pull
request until a passing `mise run ci:changed` is on record for HEAD's tree. In
inorbithr/core, `devops/ci/gates.py` writes that record; the SDK's `ci:changed`
(`tools/ci-changed.sh`, ADR 0014) wrote nothing, so an SDK pull request could not show
that the local gate had passed, and a check that had already passed ran again.

## Decision

- On a pass, `tools/ci-changed.sh` calls `tools/gate-record.py`, which writes core's
  record exactly: `${XDG_STATE_HOME:-~/.local/state}/tbd/gates/<HEAD^{tree}>/<key>.json`,
  where `<key>` is the first 32 hex digits of sha256 of `1`, `$CI`, `mise`, `run`,
  `ci:changed` joined by NUL bytes. The fields are `tree`, `cmd`, `commit`, `passed_at`
  (UTC), `seconds`, `by`, `host` and `checkout`. The file is written to a temporary name,
  then renamed, and copied to `GATES_MIRROR` over ssh when that is set (best effort).
- A record is written only when every step passed and the checkout was clean
  (`git status --porcelain` empty) before the first step and still clean, at the same
  tree, after the last one. Otherwise the run prints one line saying why and writes
  nothing. `--dry-run` and a run with `CI_CHANGED_BASE` set to anything but
  `origin/main` write nothing.
- `tools/test_gate_record.py`, run by `repo:check`, checks the path, the key and the
  fields against a temporary repository.

## Consequences

- The record format lives in two repositories, core's `gates.py` and the SDK's
  `gate-record.py`. A change to it changes both, and the test here pins the key and the
  field list so a drift shows up.
- A pass from a dirty checkout counts for nothing; commit first, then run.
- Not done here: adding the SDK to the platform's remote-gate allow-list (it lives
  outside this repository), and topping up an early `asyncio.sleep` in the Python
  runtime (a runtime fix with its own pull request, as #581 was for TypeScript).

## Sources

- inorbithr/core `devops/ci/gates.py` (read 2026-11-19)
- Platform PRD 0221, RFC 0153, ADR 0193
- ADR 0014 (what CI runs where)
