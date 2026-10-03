# Roadmap

Milestones, in order. Each ends with `mise run ci` green and nothing half-wired.

| # | Milestone | Done when |
|---|---|---|
| M0 | Repository foundation | Layout, agent setup, CI, release config, design and ADRs in place (2026-10-01) |
| M1 | Contract and harness | `tools/spec-sync.py` writes `spec/`; the replay server runs every case in `conformance/cases/` (2026-10-02) |
| M2 | The generator's shared core | The language-neutral model (`ir`, `context`), six languages registered, Java and C# named (ADR 0013), conformance cases for writes and path encoding (2026-10-03) |
| M3 | Six runtimes and targets | Rust: the runtime, its target and `iohr sdk generate\|check` on it, every case through the generated public surface (2026-10-03). TypeScript next, settling the shared model; then Python, Go, Java and C# on it. Each: a runtime, a target, golden and compile tests, every case passing; `/parity` reports no gaps |
| M4 | First release, 0.1.0 | Rust (crates.io), TypeScript (npm, JSR), Python (PyPI) and Go released at 0.1.0 (2026-10-03 and 2026-10-04); publish guards lifted for those four |
| M4b | C# and Java on their registries | NuGet: the trusted-publishing policy for `release.yml` in `release-nuget` and the `InOrbit.` prefix reserved, `IsPackable` lifted, a publish job; Maven Central: a user token and a signing key, a publish job (namespace `hr.inorbit` verified 2026-10-03) |
| M5 | Streaming | When the API admits API keys on SSE and `/v1/ws`: cases enabled, six implementations, 0.2.0 |

Upstream requests that unblock or simplify later milestones are listed in
[spec/README.md](../spec/README.md).
