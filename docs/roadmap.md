# Roadmap

Milestones, in order. Each ends with `mise run ci` green and nothing half-wired.

| # | Milestone | Done when |
|---|---|---|
| M0 | Repository foundation | Layout, agent setup, CI, release config, design and ADRs in place (2026-10-01) |
| M1 | Contract and harness | `tools/spec-sync.py` writes `spec/`; the replay server runs every case in `conformance/cases/` (2026-10-02) |
| M2 | The generator's shared core | The language-neutral model (`ir`, `context`), six languages registered, Java and C# named (ADR 0013), conformance cases for writes and path encoding (2026-10-03) |
| M3 | Six runtimes and targets | Rust: the runtime, its target and `iohr sdk generate\|check` on it, every case through the generated public surface (2026-10-03). TypeScript next, settling the shared model; then Python, Go, Java and C# on it. Each: a runtime, a target, golden and compile tests, every case passing; `/parity` reports no gaps |
| M4 | First release, 0.1.0 | Rust (crates.io), TypeScript (npm, JSR), Python (PyPI) and Go released at 0.1.0 (2026-10-03 and 2026-10-04); publish guards lifted for those four |
| M4b | C# and Java on their registries (postponed) | NuGet: the trusted-publishing policy for `release.yml` in `release-nuget` and the `InOrbit.` prefix reserved, `IsPackable` lifted, a publish job; Maven Central: a user token and a signing key, a publish job (namespace `hr.inorbit` verified 2026-10-03). Until then both follow every change and carry the same version as the other four through release-please, with nothing uploaded, so publishing is one step |
| M4c | Contract alignment, 0.2.0 | The platform states what the sync used to patch in (core #218: answers' `required`, `Detail`'s discriminator, the server, `Code.x-http-status`); rules N2, N3, N4 and N6 become checks that fail on a regression, N5 is settled behaviour with a timestamp helper in every runtime; request fields are optional and left out when unset; the new operations synced; all six at 0.2.0, the four registries released (2026-10-04) |
| M5 | Streaming | The platform bounds and audits key streams (RFC 0048); 8 server-sent event and 5 socket cases, passed by all six runtimes; stream methods generated per profile; `spec/frames.json` synced; 0.2.1 (2026-10-04) |
| M6 | Enterprise configuration and middleware | [config.md](config.md), [ADR 0015](adr/0015-configuration-and-middleware.md): `load` with one precedence (code, environment, config file, defaults) in six languages, the command line's `config.toml` shared, the credential chain, the named pipeline with its built-ins, transport settings (proxy, CA, mTLS, timeouts), logging, OpenTelemetry; every case in `conformance/cases/{credentials,middleware,transport}` and every vector passing in all six. Ships as 0.2.x patch releases (below) |

Upstream requests that unblock or simplify later milestones are listed in
[spec/README.md](../spec/README.md).

## M6: enterprise configuration and middleware

Designed 2026-10-04 ([ADR 0015](adr/0015-configuration-and-middleware.md), the contract in
[config.md](config.md), environments in [recipes.md](recipes.md)). Everything is additive:
new entry points (`load`), settings, types and pipeline operations; `from_env`, explicit
construction and hooks keep their behaviour. Under the pre-1.0 rule in
[releasing.md](releasing.md) a `feat` is a patch, so each runtime ships it as its next
0.2.x when it passes the suite. Fixes for today's divergences (the socket's request id,
Java's token `Retry-After`, the token exchange's user agent, profile name normalisation,
the attempt timeout covering the body) are `fix` commits. Two observable changes are
called out in each changelog: writes that take `Idempotency-Key` are now retried, and
calls have a 120 s total timeout by default (ADR 0015, "What it ships as").

Shared work first, in this order:

- [ ] `cli/`: keep unknown keys and the `[sdk]` table when rewriting `config.toml`
      (`toml_edit`); `iohr auth token --profile P --format json`; `iohr sdk config`;
      `IOHR_TOKEN` honoured when a default profile exists (today it is ignored, though
      the README and ADR 0009 say it wins)
- [ ] Replay server: header matchers (`*`, `$name`, `~regex`), `headers_absent`, TLS,
      mTLS and CONNECT proxy listeners, the fake `iohr` (config.md section 9.1)
- [ ] Generator: mark operations that take `Idempotency-Key`; move
      `a-write-is-not-retried` to `events.update_endpoint` and teach every driver that
      operation
- [ ] Drivers: run `conformance/vectors/` as unit tests in each language

Per language (each box: settings and `load`, `describe`, the credential chain, the
pipeline and its built-ins, transport, logging, tracing, every M6 case and vector
passing, README showing `load` first):

- [ ] Rust (first, as for every runtime; the command line uses it for `iohr sdk config`)
- [ ] TypeScript
- [ ] Python
- [ ] Go (`go/otel` submodule)
- [ ] Java (`inorbit-sdk-otel` artifact)
- [ ] C#

New runtime dependencies, each named with its reason in the pull request (SR-20):
a TOML parser in every language but Python, `tracing` in Rust,
`Microsoft.Extensions.Logging.Abstractions` in C#; OpenTelemetry only as an optional
integration.

### Platform follow-ups (inorbithr/core)

Not needed for M6; each lifts a limit the SDK documents:

1. **Workload identity.** Let a Kubernetes service account token or a GitHub Actions OIDC
   token be exchanged for an InOrbit access token, bound to a key's scopes by issuer,
   subject and audience. Hydra has no RFC 8693 token exchange (ory/hydra#1218, open
   since 2018), and its RFC 7523 trust relationships hold one static key, not an issuer's
   rotating key set. So this is either an exchange endpoint in the accounts service, or
   a job that keeps Hydra's trust relationships in step with each issuer's JWKS. Until
   then the SDK's `workload` source is reserved.
2. **IETF `RateLimit` and `RateLimit-Policy` fields** (draft 11) next to Envoy's
   `X-RateLimit-*`, and on the token endpoint.
3. **The client's request id.** Envoy does not set `preserve_external_request_id`, so the
   SDK's `x-request-id` is most likely replaced at the edge. Keep a well-formed client id,
   or echo it in another header and record it in the audit log, so the id a customer
   quotes finds the call.
4. **`Retry-After` on every `429` and `503`,** the token endpoint's rate limit included.
5. **`Idempotency-Key` on the remaining writes** (`UpdateConnection`, `UpdateEndpoint`,
   `RecoverEndpoint`, `CreateEnrollment`), or a note in the contract that they are safe
   to repeat.
6. **Docs:** `docs/auth/README.md` says rate limiting is not in Envoy yet (it is) and
   that a machine token lives an hour (keys and the command line get 15 minutes).
