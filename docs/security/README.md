# Security documentation

| Page | What it covers |
|---|---|
| [requirements.md](requirements.md) | SR-01 to SR-24: what every SDK and the command line must do before 1.0, and how it is verified |
| [threat-model.md](threat-model.md) | What we protect, the trust boundaries, threats and mitigations, residual risks |
| [controls.md](controls.md) | How the repository maps to NIST SSDF, OpenSSF OSPS Baseline, CRA Annex I and SLSA |
| [verifying-releases.md](verifying-releases.md) | How to check that a package you installed is the one we built |

Policies at the repository root:

- [SECURITY.md](../../SECURITY.md): reporting vulnerabilities, response targets,
  disclosure, CRA reporting.
- [SUPPORT.md](../../SUPPORT.md): supported versions and runtimes, the 5-year security
  support period.
- [GOVERNANCE.md](../../GOVERNANCE.md): maintainers, decisions, review controls, access
  continuity.

The decision behind all of this is [ADR 0007](../adr/0007-security-baseline.md).
