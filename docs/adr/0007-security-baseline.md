# 0007. Security baseline before 1.0

Status: accepted, 2026-10-01

## Context

The SDKs are meant for customers in finance and healthcare. Those customers assess
suppliers against SOC 2, ISO 27001, DORA (EU banks), HIPAA (US healthcare) and NIST
SSDF-style questionnaires, and ask for SBOMs, provenance, a disclosure policy and a
support period. Since 2026-09-11 the EU Cyber Resilience Act (Regulation (EU) 2024/2847)
requires manufacturers of software products to report actively exploited
vulnerabilities to ENISA; its other obligations apply from 2027-12-11. The SDKs are free
and Apache-2.0, but they exist to sell access to a paid API, which the CRA's Recital 15
treats as monetisation. The repository has one maintainer.

## Decision

- **Requirements before code.** The SDKs meet
  [docs/security/requirements.md](../security/requirements.md) (SR-01 to SR-23) before
  1.0. Each requirement names its verification; code review enforces them.
- **CRA manufacturer.** InOrbit d.o.o. acts as the manufacturer of the SDKs, not as an
  open-source steward: it reports under Article 14 now and prepares the Annex I
  technical documentation for 2027-12-11. Legal confirmation of this position is
  pending; acting as manufacturer is the safe side.
- **Support period.** Each major version gets security fixes for at least 5 years from
  its first release ([SUPPORT.md](../../SUPPORT.md)).
- **Disclosure.** [SECURITY.md](../../SECURITY.md) sets response targets (acknowledge in
  2 business days, Critical fixed in 7 days) and 90-day coordinated disclosure, with CVEs
  through GitHub.
- **One maintainer, stated openly.** Review by a second person is not possible yet. The
  compensating controls are in [GOVERNANCE.md](../../GOVERNANCE.md); adding an
  independent reviewer is planned, and then one approval becomes required.
- **Public crosswalk.** [docs/security/controls.md](../security/controls.md) states the
  status against SSDF, OSPS Baseline, CRA Annex I and SLSA, and changes with the
  controls.

## Consequences

- Design choices with security impact (TLS options, secret types, request ids,
  idempotency, timeouts) are fixed in [design.md](../design.md) section 11 before any
  language implements them.
- Every pull request that changes a requirement, the threat model or a control updates
  `docs/security/` in the same pull request.
- Supply-chain work (SBOMs and attestations for every package, vulnerability gates,
  immutable releases) is required before the first release.

## Sources

- https://eur-lex.europa.eu/eli/reg/2024/2847/oj
- https://digital-strategy.ec.europa.eu/en/policies/cra-reporting
- https://csrc.nist.gov/pubs/sp/800/218/final
- https://baseline.openssf.org/versions/2026-08-28
- https://slsa.dev/spec/v1.2/
