# Threat model

What could go wrong with the SDKs, who could make it go wrong, and what stops them.
Method: STRIDE (spoofing, tampering, repudiation, information disclosure, denial of
service, elevation of privilege) over each part of the system. Reviewed on every change
that adds a transport, an option, a dependency or a release step, and at least once a
year. Last review: 2026-10-01, before any SDK code exists.

## What we protect

| Asset | Why it matters |
|---|---|
| API key secret (`key_secret`) | Anyone holding it can act as the customer's account until the key is revoked. |
| Access tokens | Bearer credentials, valid for 15 minutes. |
| Request and response data | Customer data; may include personal or health data the customer chose to send. |
| Integrity of the published packages | Thousands of installs run whatever we publish. |
| The customer's process | The SDK runs inside it with its privileges. |

## Trust boundaries

```
 customer process                       | network            | InOrbit
 +------------------------------------+ |                    |
 | application code                   | |                    |
 |   | calls                          | |   TLS 1.2+         |  auth.inorbit.hr
 | SDK: config, token cache, retries -+-+--------------------+-> /oauth2/token
 |   | reads                          | |                    |
 | env vars, CA bundle, client cert   | |   TLS 1.2+         |  api.inorbit.hr
 +------------------------------------+-+--------------------+-> /v1/...
              ^ installed from
 package registries (npm, PyPI, crates.io, Go proxy, JSR)
              ^ published by
 GitHub Actions release workflow (OIDC) <- main branch <- pull requests <- contributors
```

Boundaries: (1) application to SDK, (2) SDK to network, (3) registry to the customer's
build, (4) CI to registry, (5) contributor to `main`.

## Threats and mitigations

### The SDK inside the customer's process

| STRIDE | Threat | Mitigation |
|---|---|---|
| I | Secret printed in a log, error, crash dump or debugger view | SR-10 typed secrets, SR-13 no bodies in logs, SR-14 redaction hook |
| I | Token persisted to disk and picked up later | SR-12 memory only |
| I | Personal data copied into URLs or error messages | SR-15 |
| I | SDK reports usage or data somewhere else | SR-16 no telemetry |
| D | Hung call ties up the customer's threads | SR-19 bounded time |
| D | Retry storm against the API after an outage | Design section 6: capped retries, full jitter, `Retry-After` honoured |
| T | Retried write applied twice | SR-18 idempotency keys |

### Configuration and credentials

| STRIDE | Threat | Mitigation |
|---|---|---|
| S | Attacker sets `INORBIT_BASE_URL` or `INORBIT_TOKEN_URL` to their host and harvests the key | SR-07 HTTPS only; residual risk: control of the environment is control of the process (see below) |
| I | Key committed to source control by a user | Out of SDK scope; docs show environment variables; GitHub secret scanning recognises the key format once registered (planned) |
| E | Overly broad key used where a narrow one would do | Keys carry scopes on the API side; docs recommend one key per workload |

### Network path

| STRIDE | Threat | Mitigation |
|---|---|---|
| S | Man in the middle impersonates the API or token endpoint | SR-01 TLS 1.2+, SR-02 verification always on, SR-06 optional pinning |
| T | Response altered in transit | TLS integrity (SR-01) |
| I | Traffic read by a corporate proxy that re-signs TLS | Customer's own choice; SR-03 lets them trust their CA explicitly instead of disabling checks |
| I | Data sent to the wrong region | SR-09 no silent fallback |
| R | Customer cannot show which call did what | SR-17 request ids on both sides |

### Package registries and installation

| STRIDE | Threat | Mitigation |
|---|---|---|
| S | Typosquatted package with a similar name | Names reserved on all four registries (`inorbithr`, `@inorbithr/sdk`); docs link the exact names |
| T | Registry serves a modified package | npm and PyPI provenance, GitHub attestations for crates and Go archives, Go checksum database (SR-23) |
| T | Compromised dependency pulled into a release | SR-20 minimal dependencies, SR-21 vulnerability gate, Dependabot cooldown of 7 days, dependency review |

### CI and release pipeline

| STRIDE | Threat | Mitigation |
|---|---|---|
| T | Malicious or compromised GitHub Action | Actions allowlist, full-SHA pinning required by repository policy, zizmor on every change |
| T | Workflow injection from pull request titles, branch names or comments | No untrusted input in `run:`; zizmor; no `pull_request_target` or `workflow_run` |
| E | Fork pull request reads secrets | Secrets are withheld from forks; outside contributors' workflows wait for approval |
| S | Stolen registry token publishes a release | No long-lived tokens exist; OIDC trusted publishing only; crates.io accepts trusted publishing only; npm versions are staged for approval with 2FA |
| T | Release tag moved to other code | Tag ruleset: only the release app creates `*/v*` tags; nobody can move or delete one |

### Contributors and `main`

| STRIDE | Threat | Mitigation |
|---|---|---|
| T | Malicious change merged | Pull requests only, required CI, AI review; residual risk below |
| E | Maintainer account takeover | 2FA on GitHub and registries; release approval needs 2FA |
| R | Unclear who changed what | Squash merges keep one attributed commit per pull request; signed by GitHub |

## Residual risks

- **One maintainer.** No second person approves changes yet. Compensating controls are
  listed in [GOVERNANCE.md](../../GOVERNANCE.md); a second reviewer is planned.
- **Environment control.** Whoever controls the process environment can redirect the
  SDK (SR-07 prevents plain HTTP, not a malicious HTTPS host). This is inherent to
  configuration by environment; pinning (SR-06) narrows it.
- **Memory in garbage-collected languages.** Go, TypeScript and Python cannot reliably
  wipe secrets (SR-11).
- **crates.io provenance.** crates.io has no native signatures yet; we attest crates
  through GitHub instead, which users must check themselves.
