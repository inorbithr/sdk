# 0006. Package names and minimum runtimes

Status: proposed, 2026-10-01

## Context

The npm scope `@inorbit` belongs to another company (it publishes `@inorbit/edge-sdk`).
`inorbit` is free on PyPI and crates.io today, but using it there while npm differs would
split the brand. The GitHub organisation is `inorbithr`.

## Decision (proposed)

| Language | Distribution name | Import |
|---|---|---|
| Go | `github.com/inorbithr/sdk/go` | `package inorbit` |
| Rust | crate `inorbithr` | `use inorbithr::Client;` |
| TypeScript | `@inorbithr/sdk` (npm), `@inorbithr/sdk` (JSR) | `import { InOrbit } from "@inorbithr/sdk"` |
| Python | `inorbithr` (PyPI) | `import inorbithr` |

Minimum runtimes, checked against https://endoflife.date on 2026-10-01:

| Language | Minimum | CI matrix | Why |
|---|---|---|---|
| Go | 1.26 | 1.26, 1.27 | Go supports the two newest releases. |
| Rust | MSRV 1.94 (about six months) | MSRV, stable | Recent enough for edition 2024 and the MSRV-aware resolver. |
| TypeScript | Node 22.12, ESM only | Node 22, 24, 26; Bun and Deno smoke tests | `require(esm)` is stable from 22.12, so CommonJS users can still load it. |
| Python | 3.11 | 3.11 to 3.14 | 3.10 reaches end of life on 2026-10-31, before the first release. |

## Open

- Confirm the names, then reserve them: the npm org `inorbithr`, a placeholder release on
  crates.io and PyPI (`docs/releasing.md`).
