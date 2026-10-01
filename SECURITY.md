# Security policy

## Reporting a vulnerability

Report privately through GitHub:
[Security > Report a vulnerability](https://github.com/inorbithr/sdk/security/advisories/new).
Do not open a public issue, pull request or discussion.

Please include the language and version, what an attacker can do, and the smallest
reproduction you have. You will get an acknowledgement within 3 working days and a
first assessment within 10.

Vulnerabilities in the InOrbit API itself (not the SDK) go through the same form; we
route them.

## Supported versions

Before 1.0, only the latest release of each package receives fixes. After 1.0, the
latest minor release of the current major version does.

## What we do on our side

- Releases are published from CI with OIDC trusted publishing and carry provenance
  attestations; no long-lived registry tokens exist.
- Every GitHub Action is pinned to a commit; workflows are audited by zizmor and CodeQL
  on every change.
- The SDKs never log or print key secrets or access tokens; each language tests this.
