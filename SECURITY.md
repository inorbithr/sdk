# Security policy

This policy covers the InOrbit SDKs in this repository: the Go module, the Rust crate,
the TypeScript package and the Python package. For how the SDKs are built to be safe,
see [docs/security/](docs/security/).

## Reporting a vulnerability

Report privately through GitHub:
[Security > Report a vulnerability](https://github.com/inorbithr/sdk/security/advisories/new).
Do not open a public issue, pull request or discussion about a vulnerability.

Include, as far as you can:

- the package, version and runtime;
- what an attacker can do, and under which conditions;
- the smallest reproduction you have;
- whether you know of it being exploited.

Vulnerabilities in the InOrbit API itself (`api.inorbit.hr`, `auth.inorbit.hr`) go
through the same form; we route them.

## What happens next

| Step | Target |
|---|---|
| Acknowledge your report | within 2 business days |
| Triage: confirm, rate severity (CVSS v4), agree a timeline with you | within 5 business days |
| Fix released, Critical | within 7 days of triage |
| Fix released, High | within 30 days |
| Fix released, Medium | within 90 days |
| Fix released, Low | in the next regular release |

The targets are for a confirmed vulnerability in a supported version
([SUPPORT.md](SUPPORT.md)). If a target cannot be met, we tell you why and what the new
date is.

## Coordinated disclosure

- We ask for up to 90 days from your report to publish a fix before details become
  public. We publish sooner when a fix is out, and earlier still if the issue is being
  exploited and users need to act.
- We publish a GitHub Security Advisory for every confirmed vulnerability and request a
  CVE through GitHub, which is a CVE Numbering Authority. Advisories flow into the
  GitHub Advisory Database and OSV, where dependency scanners pick them up.
- We credit you in the advisory unless you ask us not to.

## Safe harbour

We will not pursue legal action against research that:

- stays within this policy and reports privately;
- avoids privacy violations, data destruction and service disruption;
- uses only accounts and API keys you own, and never another customer's data;
- stops and reports as soon as you reach data that is not yours.

## EU Cyber Resilience Act

InOrbit d.o.o. treats itself as the manufacturer of these SDKs under Regulation (EU)
2024/2847. For an actively exploited vulnerability or a severe incident affecting the
security of an SDK, InOrbit reports through ENISA's Single Reporting Platform within the
Article 14 deadlines: an early warning within 24 hours of becoming aware, a notification
within 72 hours, and a final report within 14 days after a fix is available (one month
for a severe incident). Affected users are informed of the issue and of the measures
they can take, through the advisory and the release notes.

## Supported versions

See [SUPPORT.md](SUPPORT.md). Before 1.0, only the latest release of each package
receives fixes.

## What we do on our side

- Releases are published from CI with OIDC trusted publishing; no long-lived registry
  tokens exist. npm and PyPI carry provenance attestations; npm versions wait for a
  maintainer's approval with two-factor authentication before they go live.
- Every GitHub Action is pinned to a full commit SHA and must be on an allowlist;
  workflows are checked by zizmor and CodeQL on every change.
- The SDKs send no telemetry. They never log request or response bodies by default, and
  never print key secrets or access tokens; each language tests this
  ([docs/security/requirements.md](docs/security/requirements.md)).
- How to check that a package you installed is the one we built:
  [docs/security/verifying-releases.md](docs/security/verifying-releases.md).
