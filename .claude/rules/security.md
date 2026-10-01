---
paths:
  - "go/**"
  - "rust/**"
  - "typescript/**"
  - "python/**"
  - "examples/**"
  - "docs/security/**"
---

# Security rules for SDK code

The requirements are in `docs/security/requirements.md`; cite the SR id in code comments
and PR descriptions when a change touches one.

- Credentials (key secret, access token, proxy password, private key) are held in the
  language's `Secret` type and print as `<redacted>` in every formatting path. Add a test
  for each new way a value can be printed or serialised. Rust: `secrecy` / `zeroize`.
- Never log request or response bodies, query values or the `Authorization` header.
  Logging is off by default; when on, log method, path template, status, attempt,
  duration and request id only, and pass every record through the redaction hook.
- Error messages the SDK builds carry the code, status and request id. They never echo
  request values, personal data, health data or card numbers.
- TLS: minimum 1.2, verification always on, no public option to disable it. Use the
  language's maintained TLS stack; never write cryptography.
- Only `https://` base and token URLs, except loopback.
- Every network operation has a finite timeout; streams have an idle timeout. No
  configuration produces an infinite wait.
- No telemetry, update checks or calls to any host other than the configured API and
  token endpoint.
- Retry `POST`/`PATCH` only with an idempotency key reused across attempts.
- Do not add a runtime dependency without the reason in the PR and a check against the
  language's allowed list.
- If you find a vulnerability, stop and tell the maintainer privately; do not describe
  it in a commit message, issue or PR.
