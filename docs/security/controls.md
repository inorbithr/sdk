# Controls

What this repository does against common secure-development frameworks, with an honest
status. Status words: **met** (in place and enforced), **partial** (in place with a known
gap), **planned** (decided, not yet in place). Updated in the same pull request as any
change to a control. Last update: 2026-10-03.

This page covers the SDK repository. InOrbit's company-wide programme (policies, the
hosted API, audits) is maintained separately and is not public.

## NIST SSDF (SP 800-218 v1.1)

| Practice | What we do | Status |
|---|---|---|
| PO.1 Security requirements | [requirements.md](requirements.md), SR-01 to SR-28 | met |
| PO.2 Roles and responsibilities | [GOVERNANCE.md](../../GOVERNANCE.md) | partial: one maintainer |
| PO.3 Toolchain | mise-pinned toolchains, CodeQL, zizmor, actionlint, dependency review, Scorecard | met |
| PO.4 Security check criteria | SR-21 vulnerability gate; `ci-ok` required | partial: scanners land with code |
| PO.5 Secure environments | OIDC publishing, environments with approval, actions allowlist with SHA pinning, read-only default token | met |
| PS.1 Protect code | Rulesets on `main` and release tags, pull requests only, no force push | met |
| PS.2 Release integrity | Provenance on npm, PyPI and the Go archive; crates attested via GitHub | partial: crates and SBOM attestations planned |
| PS.3 Archive and protect releases | Immutable tags; per-release SBOMs | planned |
| PW.1 Design for security | [threat-model.md](threat-model.md), ADRs | met |
| PW.2 Review the design | Threat model reviewed per change and yearly | partial: one reviewer |
| PW.4 Reuse vetted components | SR-20 minimal dependencies, Dependabot with 7-day cooldown | met |
| PW.5 Secure coding | [.claude/rules/security.md](../../.claude/rules/security.md), per-language `AGENTS.md`, lint rules | met |
| PW.6 Build configuration | SR-22 reproducible build flags | planned |
| PW.7 Code review | AI review plus checklist | partial: no second human |
| PW.8 Testing | Unit tests, shared conformance cases across four languages | planned with code |
| PW.9 Secure defaults | SR-01 to SR-19 | planned with code |
| RV.1 Identify vulnerabilities | Private reporting, CodeQL, Dependabot, SR-21 scanners | partial |
| RV.2 Assess and fix | [SECURITY.md](../../SECURITY.md) response targets | met |
| RV.3 Root cause | Advisory write-up for each fix | planned |

## OpenSSF OSPS Baseline (2026-08-28)

| Level | Status | Known gaps |
|---|---|---|
| Level 1 | met for the repository; documentation items complete with the first release | user guide arrives with code |
| Level 2 | partial | DCO sign-off, signed release manifests (attestations planned), changelog per release (release-please, from the first release) |
| Level 3 | partial | QA-07.01 non-author approval (one maintainer), VEX for non-affected findings, SBOM per compiled release (planned) |

## EU Cyber Resilience Act, Annex I

| Part | What we do | Status |
|---|---|---|
| I.2(a) no known exploitable vulnerabilities at release | SR-21 | planned with code |
| I.2(b) secure by default | SR-02, SR-07, SR-13, SR-16 | planned with code |
| I.2(c) security updates | [SUPPORT.md](../../SUPPORT.md): 5 years per major | met (policy) |
| I.2(e) to I.2(g) confidentiality, integrity, data minimisation | SR-01 to SR-18 | planned with code |
| I.2(h) to I.2(j) availability, impact, attack surface | SR-19, SR-20 | planned with code |
| I.2(l) logging | SR-17 request ids | planned with code |
| II(1) SBOM | SR-23 | planned |
| II(2), II(3) fix and test | SECURITY.md targets, CI | met (process) |
| II(4) to II(6) disclosure policy and contact | SECURITY.md, private vulnerability reporting | met |
| II(7), II(8) secure, free updates | Trusted publishing; fixes are free | met |
| Article 14 reporting (since 2026-09-11) | ENISA Single Reporting Platform, 24h/72h/14 days | met (process) |

## SLSA v1.2

| Track | Level | Status |
|---|---|---|
| Build | L2 on npm and PyPI today; L3 with one reusable build-and-attest workflow | planned |
| Source | L2 to L3 (rulesets, protected history, immutable tags) | partial |
| Source | L4 needs two-party review | not reachable with one maintainer |
