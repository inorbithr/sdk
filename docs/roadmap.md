# Roadmap

Milestones, in order. Each ends with `mise run ci` green and nothing half-wired.

| # | Milestone | Done when |
|---|---|---|
| M0 | Repository foundation | Layout, agent setup, CI, release config, design and ADRs in place (2026-10-01) |
| M1 | Contract and harness | `tools/spec-sync.py` writes `spec/`; the replay server runs every case in `conformance/cases/` |
| M2 | Core in Go and Python | Client, token provider, errors, retries, hooks and the 3 operations pass every case |
| M3 | Core in Rust and TypeScript | Same cases pass; `/parity` reports no gaps |
| M4 | First release, 0.1.0 | Names reserved, trusted publishing configured, release PRs merged for all four |
| M5 | Streaming | When the API admits API keys on SSE and `/v1/ws`: cases enabled, four implementations, 0.2.0 |

Upstream requests that unblock or simplify later milestones are listed in
[spec/README.md](../spec/README.md).
