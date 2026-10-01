# Security requirements

What every SDK must do before 1.0, in all four languages. Each requirement is binding for
code review: a pull request that breaks one is not merged, and a requirement changes only
through an ADR. The client options these create are in [design.md](../design.md)
section 11.

Each entry gives the requirement, why, how it is verified, and the framework items it
serves where the mapping is real:

- **SSDF**: NIST SP 800-218 v1.1 practice ids.
- **CRA**: Regulation (EU) 2024/2847, Annex I. `I.2(x)` is Part I point 2(x), the
  essential product requirements; `II(n)` is Part II point n, vulnerability handling.
- **OSPS**: OpenSSF OSPS Baseline control ids.

Verification is one of: a unit test in each language, a conformance case run by all four
(`conformance/cases/`), or review against this page.

## Transport

**SR-01. TLS 1.2 minimum.** Connections use TLS 1.2 or 1.3; TLS 1.3 is preferred where
the runtime supports it. Older versions are never negotiated.
- Why: older TLS versions have known weaknesses; customers in finance and health audit
  for this.
- Verify: unit test against a TLS 1.1-only server fails the handshake.
- Refs: SSDF PW.9; CRA I.2(e), I.2(f).

**SR-02. Certificate verification is always on.** Server certificates and host names
are verified against the trust store. The public API has no option that turns
verification off. A test-only escape, if a language needs one, lives behind a
non-default feature or build tag and is named `dangerous_*`.
- Why: a verification switch is the most common way client libraries end up insecure
  in production.
- Verify: unit test against a self-signed server fails; review of the public API.
- Refs: SSDF PW.9; CRA I.2(b), I.2(e).

**SR-03. Custom CA bundle.** A caller can add CA certificates (PEM) to verify the API
host, for corporate proxies that re-sign TLS. The system trust store stays the default
(Rust: `rustls-platform-verifier`).
- Verify: unit test with a private CA.
- Refs: SSDF PW.9.

**SR-04. Client certificates (mTLS).** A caller can present a client certificate and
key (PEM; PKCS#12 where the language makes it simple). Where the language's TLS stack
allows it, a signer hook lets the key stay in an HSM or OS key store.
- Verify: unit test against a server that requires a client certificate.
- Refs: SSDF PW.9.

**SR-05. Proxies.** The SDK honours `HTTPS_PROXY` and `NO_PROXY` (case as the language
convention expects) and accepts an explicit proxy option, including proxies that need
authentication. Proxy credentials are treated as secrets (SR-10).
- Verify: unit test through a local CONNECT proxy.

**SR-06. Optional certificate pinning.** A caller can pin the API's public key (SPKI
SHA-256) with at least one backup pin. Pinning is off by default: it breaks on key
rotation and in networks that inspect TLS, which banks often run.
- Verify: unit test with a matching and a mismatching pin.
- Refs: SSDF PW.9.

**SR-07. HTTPS-only base URLs.** `base_url` and `token_url` must be `https://`. Plain
`http://` is refused at construction, except for loopback addresses, which tests and the
conformance replay server use.
- Verify: unit test; conformance runs on loopback.
- Refs: CRA I.2(b), I.2(e).

**SR-08. FIPS-capable modes.** Each SDK can run with FIPS-validated cryptography
supplied by its runtime, and its documentation says how. The SDK does no cryptography of
its own.
- Go: `GOFIPS140` at build time or `GODEBUG=fips140=on`, using the Go Cryptographic
  Module.
- Rust: a `fips` feature that enables rustls's FIPS mode through aws-lc-rs and asserts
  it at client construction.
- TypeScript: Node started with the OpenSSL 3 FIPS provider (`--enable-fips`).
- Python: the system OpenSSL with its FIPS provider enabled.

Documentation says "FIPS-capable", never "FIPS-validated": the validation belongs to the
cryptographic module the customer runs, not to the SDK.
- Verify: review; a CI job builds the Go and Rust variants.
- Refs: SSDF PW.9.

**SR-09. Region selection with no silent fallback.** A `region` option (`eu`, `us`,
when the API offers them) or an explicit `base_url` selects where requests go. The SDK
never retries or fails over to another region on its own.
- Why: data residency is a contractual promise for banks and hospitals; a silent
  fallback breaks it.
- Verify: conformance case: a 503 from the selected region is retried there or
  returned, never sent elsewhere.
- Refs: customers' contractual data residency (DORA Art. 30(2)(b), GDPR Chapter V).

## Secrets and data

**SR-10. Secrets are typed and redacted.** Key secrets, access tokens, proxy passwords
and private keys are held in a `Secret` type. Printing, formatting, serialising or
logging it shows `<redacted>`: Go `String`/`GoString`/`Format`, Rust `Debug`/`Display`,
TypeScript `toString`/`toJSON`/`util.inspect`, Python `__repr__`/`__str__`. The key id
(`ak_...`) is not secret and may be shown.
- Verify: a unit test per language for every way to print; conformance case
  `secret-never-in-message`.
- Refs: SSDF PW.5; CRA I.2(e); OSPS BR-07.

**SR-11. Secrets are wiped where the language allows.** Rust zeroes secret memory on
drop (`zeroize`, `secrecy`). Go, TypeScript and Python cannot guarantee it; their docs
say so and keep secrets in as few copies as possible.
- Verify: review; Rust unit test on the type.
- Refs: CRA I.2(e).

**SR-12. Tokens live in memory only.** The token cache is in process memory. The SDK
never writes a token or a secret to disk, environment variables or shared storage.
- Verify: review.
- Refs: CRA I.2(e).

**SR-13. No bodies in logs.** Logging is off by default. When the caller turns it on,
the SDK logs method, path template, status, attempt, duration and request id. It never
logs request or response bodies, query values, or the `Authorization` header.
- Verify: unit test capturing debug logs during a call that carries a body.
- Refs: SSDF PW.5; CRA I.2(e), I.2(g).

**SR-14. Redaction hook.** A caller can register a function that sees every log record
and error message before it leaves the SDK, and can mask fields their own policy marks
as sensitive.
- Verify: unit test.
- Refs: CRA I.2(e), I.2(g).

**SR-15. No personal data in URLs or errors.** SDK methods never place values that could
be personal data, health data (PHI) or card numbers in URL paths or query strings, except
opaque identifiers the API defines (`org_id`). Error messages built by the SDK include
the code, status and request id, and never echo request values.
- Why: URLs end up in proxy logs, access logs and browser history; messages end up in
  bug trackers.
- Verify: review of every operation; conformance cases check messages.
- Refs: CRA I.2(g); GDPR Art. 25 (data protection by design).

**SR-16. No telemetry.** The SDK sends nothing anywhere except the requests the caller
makes to the API and the token endpoint: no usage pings, no crash reports, no update
checks.
- Verify: review; conformance runs with only the replay server reachable.
- Refs: CRA I.2(g).

## Reliability and traceability

**SR-17. Request ids both ways.** Every request carries a client-generated request id
header; every result and every error exposes the client id and, when the API returns
one, the server's request id.
- Why: customers correlate SDK calls with their own audit logs and quote the id in
  support requests.
- Verify: unit test; conformance case.
- Refs: CRA I.2(l).

**SR-18. Idempotency before retrying writes.** A non-idempotent request (`POST`,
`PATCH`) is retried automatically only when it carries an idempotency key, generated
once per logical call and reused on every attempt. Until the API supports the header,
such requests are not retried (ADR 0004).
- Verify: conformance case.
- Refs: CRA I.2(f).

**SR-19. Bounded time everywhere.** Every operation has a connect timeout and a total
deadline; the defaults are finite (design.md section 2). Streams have a connect timeout
and an idle timeout: no data and no keep-alive for 45 s ends the stream with a timeout
error. No configuration can set an infinite wait; the caller can only choose a longer
finite one.
- Verify: unit test against a server that never answers.
- Refs: CRA I.2(h), I.2(i).

## Supply chain

**SR-20. Minimal dependencies.** Runtime dependencies are limited to the lists in each
language's `AGENTS.md`. Each added dependency needs a reason in the pull request.
Lockfiles are committed for CI and examples.
- Verify: review; dependency review on every pull request.
- Refs: SSDF PW.4; CRA I.2(j); OSPS DO-06.

**SR-21. Known-vulnerability gate.** A release cannot ship with a known High or Critical
vulnerability in its dependencies unless a VEX statement explains why the SDK is not
affected. The check runs in CI (`govulncheck`, `cargo audit`, `pip-audit`, `npm audit`,
OSV-Scanner).
- Verify: CI.
- Refs: SSDF RV.1; CRA I.2(a), II(2); OSPS VM-05.

**SR-22. Reproducible builds.** Builds use the flags that make output depend only on
source and lockfile: Go `-trimpath`, Rust `--locked`, Python `SOURCE_DATE_EPOCH` for
sdist and wheel, npm packed from a clean checkout.
- Verify: CI rebuilds and compares digests (planned).
- Refs: SSDF PW.6, PS.2.

**SR-23. Every release is attested.** Each published artifact has build provenance and
a CycloneDX SBOM attached and attested, so users can check what they installed
([verifying-releases.md](verifying-releases.md)).
- Verify: release workflow; the verification guide is tested on each release.
- Refs: SSDF PS.2, PS.3; CRA II(1), II(7); SLSA Build L3 (target).
