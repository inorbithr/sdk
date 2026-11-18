# 0018: The runner obeys the build host's rules

Date: 2026-11-18. Status: accepted.

## Asked

`iohr runner` checks job checkouts with the repository's CI on a shared build host. It
ignored three rules of that host (core `docs/ci.md`, "Build caps"; `devops/ci/gate_slots.py`).

## Found

1. One `CARGO_TARGET_DIR` per repository: cargo locks it, so jobs of a repository queued
   on "Blocking waiting for file lock". The host rule is one target per checkout.
2. The environment scrub dropped `CARGO_BUILD_JOBS`, `NEXTEST_TEST_THREADS` and
   `GATE_SLOTS`, and every sign-in, so the gate-slot lease was silently skipped (the
   client falls back to no slot) and the caps were lost.
3. The slot count (`taken`) left out jobs in their check, so a runner at capacity 1 could
   accept a second lease while the check filled the host.

## Decided

1. The target is `runner/target/<owner>/<repo>/<job_id>`, covered by the existing
   `Kind::Target` sweep in `hygiene.rs` (no second sweep; core's `target-sweep.sh` and
   `wt-reap.py` serve core's worktrees and are not reusable for a runner state directory).
2. The caps and `GATE_SLOTS` pass. For the lease, the profile name `IOHR_PROFILE` passes;
   tokens stay denied. Trade-off: the check's child can use the runner's profile session
   for the gate-slot call (and anything else `iohr` allows that profile), where a narrow
   lease-only token would be tighter. No such token exists yet; adding one is the follow-up.
3. `taken` counts `gating` jobs.

Control change: runner checks now take the gate-slot lease and the host's job caps.
