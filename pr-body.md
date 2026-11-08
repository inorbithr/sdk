Implements: RFC 0213
Study: 0172

## What broke

`iohr stop` inside the panel stops the session host and lifts the panel, but the panel proxy (and the shell around it) keeps running. A second bare `iohr` in that shell only sent the resume sequence and printed "the panel is on; the menu key opens the menu". The panel came back, but nothing started or joined a session host, so the panel ran with no host.

Cause: the host was only started and joined by `enter()` at first launch. After a stop, the re-run is a short-lived process, and the host stops when its last session connection closes, so the re-run cannot be the one that holds the session.

## The fix

The long-lived panel proxy now rejoins. When the shell output shows the panel going from stopped back to resumed, the proxy runs the existing `join_host` through a new `join_watched` helper (shared with `enter()`, which now uses it too) and keeps the session for as long as the panel is up. The existing watcher lifts the panel again if that host goes away. No second start path is added.

If the host cannot start, the same note is printed as at first launch: "no session host: ...; the panel runs without one".

Touched: `commands/session.rs` (`Joined`, `join_watched`), `commands/mod.rs` (module visibility), `panel.rs` and `panel/proxy.rs` (project in the proxy config, `shell_wrote`, `resumed`, `rejoined`).

## Tests

- `tests/runner.rs`: `bare_iohr_after_stop_inside_the_panel_starts_the_host_again` runs the panel in a pty under a temporary HOME, stops the host from inside, re-runs `iohr`, and waits for a live host started by the proxy. It is in `runner.rs` because `panel_live.rs` has no way to start a real host.
- `tests/panel_live.rs`: `a_rerun_after_stop_that_cannot_start_a_host_says_why_again` checks the "no session host:" note appears again on the re-run.

Before (origin/main):

```
test bare_iohr_after_stop_inside_the_panel_starts_the_host_again ... FAILED
panicked at crates/iohr/tests/runner.rs:1981:5:
gave up waiting for a host started by the re-run
test result: FAILED. 0 passed; 1 failed; 35 filtered out
```

After (this branch):

```
PASS iohr::panel_live a_rerun_after_stop_that_cannot_start_a_host_says_why_again
PASS iohr::runner bare_iohr_after_stop_inside_the_panel_starts_the_host_again
Summary 2 tests run: 2 passed, 1487 skipped
```

Whole crate: `cargo nextest run -p iohr` gives 1488 run, 1488 passed, 1 skipped. `cargo clippy -p iohr --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Known limits

- A re-run while the old host is still draining does not rejoin.
- A re-run in a panel that was never stopped, after a failed first start, does not retry.

Semver: patch (pre-1.0 fix). No API, scope, monitor or diagram changes.

## Links

- PRD 0203: https://console.inorbit.hr/decisions/spaces/lab_01M49HTNZHP26N7MAFB5G988XR/documents/ldoc_01M6VV9PTRHQTY4MXRVTNMBBHJ
- RFC 0213: https://console.inorbit.hr/decisions/spaces/lab_01M49HTNZHP26N7MAFB5G988XR/documents/ldoc_01M6VV9Q5W3TH89R101W75S256
- ADR 0226: https://console.inorbit.hr/decisions/spaces/lab_01M49HTNZHP26N7MAFB5G988XR/documents/ldoc_01M6W507F598PJ64VCPF84V3PF
- Study 0172: https://console.inorbit.hr/decisions/spaces/lab_01M49HTNZHP26N7MAFB5G988XR/documents/ldoc_01M6YBN06KJJQTVTCK82MF5M3N
