# 0015. Configuration, credentials and the middleware pipeline

Status: accepted, 2026-10-04. The normative contract is [config.md](../config.md); this
record holds the decisions, what they are based on, and what they cost. Amends
`design.md` sections 2, 6 and 8, and SR-18. Adds SR-29 and SR-30.

## Context

Each of the six runtimes is configured differently (survey of `main` on 2026-10-04):

- **Six construction styles:** a builder (Rust, Java), an options object (TypeScript,
  C#), keyword arguments (Python), functional options (Go). This is acceptable, because
  each is the language's idiom.
- **`from_env` disagrees with itself.** Rust upper-cases the profile name and turns `-`
  into `_`; the other five use it verbatim. Python and Go take overrides, the other four
  do not. Every runtime reads only six variables: token, key id, secret, scopes, base URL,
  token URL. Five docstrings say the base URL is shared by every profile, while the code
  reads the profile's own first.
- **Nothing else is configurable from outside the code.** Timeouts, retries, proxies,
  CAs and logging have no environment variable or file. Proxy, CA and mTLS exist only by
  passing a whole HTTP client. In Rust there is none, and its socket ignores the system
  proxy. `design.md` section 2 promised `ca_bundle`, `client_cert`, `proxy`,
  `pinned_keys`, `logger` and `redact`. None is built.
- **Hooks observe, nothing can intervene.** Go alone has built-in logging (`SlogHook`).
  No runtime propagates `traceparent` or sends `Idempotency-Key`, though the platform
  takes the header on 16 operations since RFC 0033.
- **Small divergences everywhere.**
  - Java's token exchange ignores `Retry-After`.
  - Four token exchanges send their HTTP library's user agent.
  - The attempt timeout covers the body in four languages, only the headers in Java,
    and each read in Python.
  - The socket's request id is missing in TypeScript and C#, and in another form in
    Python.
- **The developer's login is unused.** `iohr login` stores a session in the OS keychain
  and a profile in `config.toml`. The SDK reads neither, so a developer who has signed
  in still has to export a key to run a script.

The owner's request: one serious, enterprise-grade model for configuration and
middleware, the same in six languages, with defaults on and overrides available, for
the cases where companies run software: CI, Kubernetes, proxies, private CAs,
compliance logging, existing OpenTelemetry.

## Decision

### 1. One resolution order: code, environment, config file, defaults

Each setting resolves on its own, in that order. This is AWS's order without the JVM
properties and the separate credentials file. It is Azure's rule that client options
beat global configuration, and Google's for `universe_domain`. Twelve-factor puts
deploy-time configuration in the environment. Putting the environment above the file
lets a deployment override a developer's file without editing it.

Trade-off: a stray `INORBIT_*` variable overrides a profile the user selected on
purpose. `describe()` shows the source of every value, and `credential_sources` pins the
credential in production.

### 2. A new `load` entry point; `from_env` unchanged

`load` reads everything; explicit construction reads nothing; `from_env` keeps exactly
today's behaviour, soft-deprecated. Changing `from_env` in place would make a client
that failed yesterday succeed today with a credential from a file or the keychain. That
surprise is acceptable in a new function and not in an old one.

Trade-off: three ways to build a client. Every README shows `load`, and the other two
are documented as the "nothing ambient" and "legacy" choices.

### 3. Share the command line's file; secrets never in it

The SDK reads `iohr`'s `config.toml`: the same location (its `etcetera` strategy,
`IOHR_CONFIG_DIR`), the same format (TOML), the same profiles. SDK settings go in each
profile's table and in an `[sdk]` table for all of them. `INORBIT_CONFIG_FILE` overrides
the location, as `AWS_CONFIG_FILE` does.

Secrets are refused in the file, with a pointer to `key_secret_file`, `token_file`, the
environment or `iohr login`. AWS's plain-text `credentials` file is the pattern most
security reviews flag. The command line already promises that the file never holds a
secret (SR-24), and one shared file must keep that promise.

Trade-off: a TOML parser becomes a runtime dependency in five languages, the first for
TypeScript, Go and C#. The parsers chosen have no dependencies of their own
(`config.md` section 8). The other options were a hand-written subset parser in each
language, six chances to get TOML wrong, or JSON, which would mean a second file next to
the command line's.

### 4. The credential chain: code, env, workload, file, cli

The order follows decision 1. It is AWS's default chain and `DefaultAzureCredential`
reduced to what the platform accepts:

- **code**, then **env**: a static token, a token file, or a key with its secret or
  secret file.
- **workload**: reserved. Kubernetes projected tokens and GitHub Actions OIDC need the
  platform to exchange an outside token, and Hydra cannot yet: RFC 8693 token exchange
  has been an open Hydra issue since 2018, and its RFC 7523 trust relationships hold a
  static key, not a key set that rotates. The slot and the variable names are fixed now
  so the chain does not reorder later. It is skipped with a reason until platform
  follow-up 1 lands.
- **file**, then **cli** (the `iohr` login).

Pieces of a credential never mix across sources. Two kinds in one source is an error,
not a precedence question. The chain is decided at `load`, and the failure message lists
every source and why it was skipped, as `DefaultAzureCredential`'s does.
`credential_sources` narrows the chain, after `AZURE_TOKEN_CREDENTIALS`.

### 5. The login through `iohr auth token`, not the keychain

The `cli` source runs `iohr auth token --profile P --format json`, as Azure's
`AzureCliCredential` runs `az account get-access-token`. Reading the keychain directly
would need a keychain binding in six languages, two with no maintained one (Node's
`keytar` is archived; Java has none in the JDK). Each would also have to reimplement the
command line's refresh-token rotation, including re-reading the store before refreshing
and the `invalid_grant` retry. A second refresher racing the first would cost the user
their session.

Trade-off: a process start per refresh (about four an hour with 15-minute tokens) and a
dependency on `iohr` being installed. The process start is small. Without `iohr` the
source is skipped with a reason.

### 6. Rotation without restart, by re-reading files

A key secret file is read before every exchange. A token file is re-read when it
changes and after a 401. This is how Kubernetes delivers rotated Secrets and projected
tokens: the kubelet rewrites the file, and the application must read it again.
Refresh-ahead at 20 % of the lifetime, single flight and the one fresh token after a 401
stay as `design.md` section 3 has them. A soft expiry (keep a still-valid token when a
refresh fails) avoids turning a brief token-endpoint outage into failed calls.

### 7. An ordered, named pipeline with per-call and per-retry stages

The pipeline is Azure Core's `HttpPipeline` model with its two positions (`PerCall`,
`PerRetry`). The built-ins are the policies Azure's guidelines require (telemetry, unique
request id, retry, authentication, tracing, logging), plus idempotency keys, a deadline,
rate limits and today's hooks, all on by default. Users add at the two slots, or before
or after any name, and can replace or remove by name. `retry`, `auth` and `timeout` can be
replaced but not removed.

One concept, six idioms, each chosen for what plugs into it:

- Go's `RoundTripper`, so `otelhttp` fits unchanged;
- C#'s `DelegatingHandler` adapter, so Polly fits;
- Java's OkHttp-style interceptor;
- a Rust trait of our own, so the runtime needs no `tower`;
- plain functions in TypeScript;
- callables in Python.

Trade-off: an Azure-style pipeline is more machinery than hooks. Hooks cannot add a
header, short-circuit a call or wrap an attempt in a resilience library, and that is
what enterprise users ask for first.

### 8. Writes with an idempotency key are retried

Operations whose contract declares `Idempotency-Key` get a UUID v4 per call, the same on
every attempt, and are retried like reads. This is how Stripe's libraries make writes
safe to retry. It replaces ADR 0004's "no key, no retry of POST" now that the platform
supports the header (RFC 0033). Writes without the header keep the old rule.

### 9. A retry budget; no circuit breaker

Retries draw from a token bucket per client: capacity 500, cost 5 after throttling and
10 otherwise, a refund on success. This follows AWS's standard retry mode (its 2026
update raises transient retries to 14) and gRPC's retry throttling (gRFC A6). It is the
SDK-level answer to retry storms.

A circuit breaker is not built in. No major cloud SDK's core ships one. The retry budget
already fails fast under a sustained outage. Companies that use breakers have a standard
one (Polly, resilience4j, failsafe-go, cockatiel), and the per-call slot takes it. A
seventh implementation would be ours to keep consistent and would duplicate theirs.

### 10. Logging off, allowlisted, redacted; OpenTelemetry optional and conventional

- **Logging.** Off by default, as today. It goes to each language's standard facility.
  Headers are logged only from an allowlist, as the Azure guidelines require. A fixed
  never-log set (credentials and cookies), no bodies, no query values. `redact` is the
  last step.
- **Tracing.** One span per call and one per attempt, as Azure's tracing guidelines say
  and as the OpenTelemetry HTTP client conventions require ("SHOULD create an HTTP span
  for each attempt"). Names, attributes and `http.client.request.duration` follow the
  stable semantic conventions.
- **No dependency forced.** OpenTelemetry stays optional. In .NET, `ActivitySource` makes
  it free.

### 11. Transport settings the SDK owns, the same everywhere

- **Proxy.** `HTTPS_PROXY`/`NO_PROXY` are honoured, but the `no_proxy` matcher is ours,
  because the libraries disagree. GitLab's 2021 survey found that leading dots, `*`,
  CIDR and case are handled differently by curl, wget, Python, Ruby and Go. Go itself
  switched to lower case first in July 2026. The SDK takes curl's reading, which most
  operators expect, and the vectors pin it.
- **Trust.** `ca_bundle` adds to the system store rather than replacing it. AWS's
  replacing `AWS_CA_BUNDLE` makes a corporate CA break the public endpoint. It is also
  unsupported in four AWS SDKs, a gap we do not repeat.
- **Timeouts.** Connect, attempt, total and stream idle are four separate timeouts. The
  new 120 s default total timeout makes SR-19's "total deadline" true for the first time.
- **Caller-supplied HTTP client.** Supported in all six languages. When the caller
  supplies one, its own network settings win, and setting both is an error.

### 12. Conformance: cases for behaviour, vectors for pure functions

Middleware order, credentials, idempotency reuse, redaction, `traceparent` and rate
limits are replay cases. They need new matchers (`*`, `$name`, `~regex`), TLS, mTLS and
proxy listeners, and a fake `iohr`. Precedence, paths, `no_proxy` and header parsing are
vectors that each runtime runs as unit tests. Driving resolution through a live server
would test the server, not the rules. Every new case and vector is `pending` for all six
languages until each passes it.

## What it ships as

Additive throughout: new functions, options, settings and types. Existing constructors,
`from_env`, hooks and errors keep their behaviour. Under the pre-1.0 rule
(`releasing.md`), `feat` commits release it as **0.2.x patch versions**, one runtime at a
time. Behaviour changes that fix a divergence from the documented contract are `fix`
commits:

- the request id on the socket;
- `Retry-After` on Java's token exchange;
- the token exchange's user agent;
- profile name normalisation in five runtimes;
- the attempt timeout covering the body.

Two changes alter what a working program observes, and are called out in the changelog:

- **Writes with an idempotency key are now retried.** Safe by construction, and only on
  the 16 operations that declare the header.
- **The default `total_timeout` of 120 s.** A call that today takes longer through
  retries and long `Retry-After` waits now ends with a `TimeoutError`. That is only
  possible with three 30 s attempts and two 60 s waits.

Neither changes a signature, so neither is a breaking change under SemVer. If a reviewer
judges the timeout to be one, it is isolated in its own `feat!` commit and becomes 0.3.0
on its own. Go's `Hook` interface is not extended (`RetryHook` is separate), so no
implementation breaks.

## Consequences

- `cli/` changes before any SDK reads the file:
  - `iohr` must keep unknown keys when it rewrites `config.toml`;
  - add `iohr auth token` and `iohr sdk config`;
  - fix `IOHR_TOKEN` being ignored once a default profile exists.
- The generator marks operations that take `Idempotency-Key`. The conformance case
  `a-write-is-not-retried` moves to an operation without the header.
- SR-18 now says what M6 does. SR-29 (no secrets in the config file) and SR-30 (the
  logging allowlist and never-log set) are added.
- Platform follow-ups, none blocking M6, are listed in `roadmap.md` under M6:
  1. workload identity exchange;
  2. the IETF `RateLimit` fields;
  3. keeping or echoing the client's `x-request-id` (Envoy regenerates it today);
  4. `Retry-After` on every `429` and `503`, the token endpoint included;
  5. `Idempotency-Key` on the remaining writes;
  6. two stale lines in the platform's auth docs.

## Sources

- AWS SDKs and Tools Reference Guide: settings and precedence,
  https://docs.aws.amazon.com/sdkref/latest/guide/settings-reference.html; file locations,
  https://docs.aws.amazon.com/sdkref/latest/guide/file-location.html; credential providers,
  https://docs.aws.amazon.com/sdkref/latest/guide/standardized-credentials.html; retry
  behaviour and quota, https://docs.aws.amazon.com/sdkref/latest/guide/feature-retry-behavior.html
  and the 2026-05-20 announcement,
  https://aws.amazon.com/blogs/developer/announcing-updated-retry-behavior-for-aws-sdks-and-tools/;
  `AWS_CA_BUNDLE` support, https://docs.aws.amazon.com/sdkref/latest/guide/feature-gen-config.html;
  endpoints, https://docs.aws.amazon.com/sdkref/latest/guide/feature-ss-endpoints.html;
  web identity, https://docs.aws.amazon.com/sdkref/latest/guide/feature-assume-role-credentials.html;
  `credential_process`, https://docs.aws.amazon.com/sdkref/latest/guide/feature-process-credentials.html
- Azure SDK guidelines: https://azure.github.io/azure-sdk/general_azurecore.html,
  https://azure.github.io/azure-sdk/general_implementation.html,
  https://azure.github.io/azure-sdk/general_design.html; pipeline positions,
  https://learn.microsoft.com/en-us/dotnet/api/azure.core.httppipelineposition; the .NET
  pipeline order, https://github.com/Azure/azure-sdk-for-net/blob/main/sdk/core/Azure.Core/src/Pipeline/HttpPipelineBuilder.cs;
  credential chains and `AZURE_TOKEN_CREDENTIALS`,
  https://learn.microsoft.com/en-us/azure/developer/python/sdk/authentication/credential-chains
- Google Cloud: Application Default Credentials,
  https://docs.cloud.google.com/docs/authentication/application-default-credentials;
  AIP-4110 and AIP-4117, https://google.aip.dev/auth/4110, https://google.aip.dev/auth/4117;
  `ClientOptions`, https://googleapis.dev/python/google-api-core/latest/client_options.html
- Stripe: https://github.com/stripe/stripe-node/blob/master/README.md,
  https://github.com/stripe/stripe-python/blob/master/README.md,
  https://docs.stripe.com/error-low-level. Twilio:
  https://github.com/twilio/twilio-node/blob/main/README.md
- OpenTelemetry HTTP semantic conventions: spans,
  https://opentelemetry.io/docs/specs/semconv/http/http-spans/; metrics,
  https://opentelemetry.io/docs/specs/semconv/http/http-metrics/. W3C Trace Context,
  https://www.w3.org/TR/trace-context/
- IETF: RateLimit header fields, draft 11,
  https://datatracker.ietf.org/doc/draft-ietf-httpapi-ratelimit-headers/; RFC 9110
  `Retry-After`, https://www.rfc-editor.org/rfc/rfc9110.html#name-retry-after;
  Idempotency-Key draft 07 (expired),
  https://datatracker.ietf.org/doc/draft-ietf-httpapi-idempotency-key-header/; RFC 8693,
  https://www.rfc-editor.org/rfc/rfc8693.html; RFC 7523,
  https://www.rfc-editor.org/rfc/rfc7523.html
- Twelve-factor config, https://12factor.net/config
- Kubernetes projected volumes, https://kubernetes.io/docs/concepts/storage/projected-volumes/;
  GitHub Actions OIDC, https://docs.github.com/en/actions/reference/security/oidc
- Ory Hydra: JWT bearer grant, https://www.ory.com/docs/hydra/guides/jwt; token exchange
  (open issue), https://github.com/ory/hydra/issues/1218
- Proxies: "We need to talk: Can we standardize NO_PROXY?",
  https://about.gitlab.com/blog/we-need-to-talk-no-proxy/; Go `httpproxy`,
  https://pkg.go.dev/golang.org/x/net/http/httpproxy and
  https://github.com/golang/net/commit/a02ddfa7ea
- FIPS: Go, https://go.dev/doc/security/fips140; rustls,
  https://docs.rs/rustls/latest/rustls/manual/_06_fips/index.html; .NET,
  https://learn.microsoft.com/en-us/dotnet/standard/security/fips-compliance
- Resilience: gRPC retry throttling, https://github.com/grpc/proposal/blob/master/A6-client-retries.md;
  .NET resilience handler, https://learn.microsoft.com/en-us/dotnet/core/resilience/http-resilience
- The platform: RFC 0016 (API tokens), RFC 0033 (the response contract, `Idempotency-Key`),
  RFC 0048 (key streams), its `docs/auth/README.md` and `docs/accounts/README.md`
