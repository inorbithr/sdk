## What and why

<!-- One or two sentences. Link the issue. -->

<!-- One line per platform RFC this implements (RFC 0041); only these lines link the PR to an RFC:
Implements: RFC 0040.1
-->

## Languages

- [ ] Go
- [ ] Rust
- [ ] TypeScript
- [ ] Python
- [ ] Not a public change (docs, CI, tooling)

If a language is not included, why, and the `parity` issue that tracks it:

## Checklist

- [ ] `mise run ci` passes locally
- [ ] A conformance case covers new or changed behaviour
- [ ] Examples and `docs/design.md` updated if the public surface changed
- [ ] No hand edits under `spec/` or any `generated/` directory
- [ ] Breaking change? The title has `!` and the body explains the migration
