# iohr-atlas-core

Atlas core: the deterministic types under InOrbit Atlas. Stored truth is **evidence** and
**claims**; a claim is supported only when a proof tree ends at evidence that exists. Models
propose, this crate decides.

- `evidence`, `observation`, `digest`, `method`, `observer`, `snapshot`, `time`: what evidence is.
- `claim`, `proof`, `derivation`, `assertion`: what evidence proves. The proof rules are sealed
  and `proof::prove` is the only writer of a claim's support.
- `question`, `proposal`: hypotheses ranked by elimination; a reasoner's output is a proposal,
  never truth.

No network, no `unsafe`, no platform dependency. Licensed Apache-2.0.
Design: RFC 0218, ADR 0023 in InOrbit's Decisions.
