# InOrbit SDK for .NET

The runtime `InOrbit.Sdk` (`net8.0`) and the public surface generated into it. Built and
tested in the open; the release to NuGet is planned and has not happened yet. Until then,
reference the project from this repository (`dotnet add reference
path/to/sdk/csharp/src/InOrbit.Sdk`). Two runtime dependencies: `Tomlyn` (no dependencies
of its own), to read the `iohr` config file, and `Microsoft.Extensions.Logging.Abstractions`.

```csharp
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

// Code, the environment, the iohr config file and the iohr login, in that order.
using var client = Client.Load();
var me = (await client.MeAsync()).Value;
var page = (await client.Radar().ListDigestsAsync(new RadarListDigestsParams { Limit = 3 })).Value;
```

`Load` finds credentials the way the [configuration contract](../docs/config.md) says,
the first source that has any winning:

1. code: `new ClientOptions { Token = ... }`, `{ KeyId, KeySecret, Scopes }` (or
   `KeySecretFile`), `{ TokenFile }` or a `TokenProvider`;
2. the environment: `INORBIT_TOKEN`, `INORBIT_TOKEN_FILE`, or `INORBIT_KEY_ID` with
   `INORBIT_KEY_SECRET` (or `INORBIT_KEY_SECRET_FILE`, read before every token exchange)
   and `INORBIT_SCOPES`;
3. workload identity: reserved until the platform offers it;
4. the profile's table in the `iohr` config file (`token_file`, or `key_id` with
   `key_secret_file`);
5. the developer's `iohr login`, through `iohr auth token`.

After `iohr login`, a program needs nothing else. In production, pin the source:
`INORBIT_CREDENTIAL_SOURCES=env`. Every other setting (timeouts, retries, proxy, CA
bundle, client certificate, logging, rate limits) resolves on its own from code, then
`INORBIT_*`, then the config file, then its default. An option set in code always wins:

```csharp
using var client = Client.Load(new ClientOptions { Timeout = TimeSpan.FromSeconds(5) });
Console.WriteLine(client.Config); // every setting and where it came from, secrets redacted
```

`iohr sdk config` prints the same document from the command line. A bad value fails
`Load` with one `ConfigException` whose `Problems` list every setting at fault and its
source. `LoadOptions` resolves against an environment, an OS and a home directory you
give, for tests. [docs/recipes.md](../docs/recipes.md) has ready setups for CI,
Kubernetes, corporate proxies, mTLS gateways and serverless.

`new Client<TProfile>(new ClientOptions { ... })` reads code only, for libraries and tests;
`Client.FromEnv()` keeps its old behaviour (six variables, credentials and URLs) and is
superseded by `Load`.

## A surface for your own credentials

`iohr sdk generate --lang csharp --for ci --package Acme.Api --out src/Iohr` writes the
models and the operations profile `ci` may call into your project, in namespace
`Acme.Api`, with `iohr.lock` beside it. Each profile is a type; an operation is an
extension method constrained to the profiles whose cut holds it, so a call a profile may
not make does not compile:

```csharp
using var ci = Client.Load<Ci>();             // INORBIT_CI_*, then INORBIT_*, [profiles.ci], its iohr login
await ci.Accounts().GetUsageAsync("acc_1");   // compiles only if ci's cut holds usage:read
```

A generated profile is its own configuration profile: its credentials come only from
`INORBIT_CI_*`, its table or its login, never from another profile's.

## Middleware

Every call goes through a named pipeline: `request_id`, `user_agent`,
`idempotency_key`, `call_tracing` and `deadline` once per call, then `retry`, then
`auth`, `rate_limit`, `attempt_tracing`, `logging`, `hooks` and `timeout` on every
attempt. Add your own at either stage, or before, after or instead of any built-in:

```csharp
sealed class Team : Middleware
{
    public override string Name => "team";

    public override ValueTask<SdkResponse> SendAsync(SdkRequest request, MiddlewareNext next, CancellationToken cancellationToken)
    {
        request.Headers["x-team"] = "payments"; // sees the finished request, token included
        return next(request, cancellationToken);
    }
}

using var client = Client.Load(new ClientOptions { Pipeline = p => p.AddPerRetry(new Team()).Remove("rate_limit") });
```

`retry`, `auth` and `timeout` can be replaced but not removed. A middleware must not log
secrets or bodies, and must not read a stream's body (`request.Info.Stream`).
`Middleware.FromHandler(name, handler)` puts any `DelegatingHandler` in the pipeline, so a
Polly or `Microsoft.Extensions.Http.Resilience` circuit breaker sits at the per-call slot
(`AddPerCall`). See [examples/csharp/Load](../examples/csharp/Load/Program.cs).

Per-call options go through a view of the client that shares its connections and token:

```csharp
await client.WithOptions(new CallOptions { IdempotencyKey = "order-42" }).Events().CreateEndpointAsync(body);
```

## Logging, tracing and metrics

Logging is off until `Log` (or `INORBIT_LOG`) sets a level; records go to the
`LoggerFactory` you give, category `InOrbit.Sdk`, as structured `LogRecord`s (operation,
method, path, status, attempts, durations, request ids). Bodies, query values,
credentials and cookies never reach a record; header values only from the allowlist, with
`LogHeaders`. `Redact` sees every record last.

Spans and metrics need no package: an `ActivitySource` and a `Meter` named `InOrbit.Sdk`,
inert until a listener subscribes. With OpenTelemetry .NET, `AddSource("InOrbit.Sdk")`
and `AddMeter("InOrbit.Sdk")`. Each call is an internal span named for the operation, with
a client span per attempt (`GET /v1/radar/digests/{digest_id}`); `traceparent` is sent from
the attempt span.

## What the runtime does

- Credentials: an API token, a token file (read again when it changes and after a 401),
  an API key exchanged for 15-minute tokens (one exchange however many calls wait,
  refreshed at a fifth of its life and after a 401; a still-valid token is used when a
  refresh fails), the `iohr` login, or your own `ITokenProvider` (wrap it in
  `CachedToken` for the same rules). `ToString()` never shows a secret.
- Timeouts: 10 s to connect, 30 s per attempt (the whole answer included), 120 s per call
  (every attempt and wait included); a per-call `Timeout` or cancellation shortens them.
- Retries on 429, 503, 504 and connection failures, for idempotent calls and for writes
  whose operation takes an `Idempotency-Key` (one key per call, sent on every attempt),
  honouring `Retry-After` (seconds or a date) up to a minute, within a per-client retry
  budget; answers capped at 16 MiB.
- Rate limits: every result carries the edge's `RateLimit` snapshot;
  `RateLimit = RateLimitMode.Wait` holds a call until an empty window resets.
- Network: the SDK's own `SocketsHttpHandler` with the proxy (`INORBIT_PROXY`,
  `HTTPS_PROXY`, the shared `no_proxy` grammar), a CA bundle added to the system's trust,
  mTLS from PEM files or `ClientCertificates`, and pinned keys. Give an `HttpClient` or an
  `HttpMessageHandler` and those settings belong to it; the pipeline still runs.
- Errors: `InOrbitException` and its kinds, with the call's `RequestId` and
  `IdempotencyKey`; `ApiException` carries the `Code` (unknown codes kept as the API wrote
  them), the HTTP status, typed details and the raw answer.
- 64-bit integers are `long`, sent and read as decimal strings.
- Every request property is nullable and left out when unset; an answer's property is
  nullable unless the API always sends it. Timestamps stay the strings the API sent;
  `Timestamps.Parse` reads one, `""` (unset) as `null`.
- A paged list has an `All<Operation>Async` beside its page method, an
  `IAsyncEnumerable<T>` that follows the next-page token:
  `await foreach (var d in client.Radar().AllListDigestsAsync(cancellationToken: ct))`;
  breaking out fetches nothing more, and the token stops it between pages.
- A stream (`events:read`: the account's events) is an `IAsyncEnumerable<T>` of its
  events, opened on the first step of the loop and closed by leaving it or by the token:
  `await foreach (var ev in client.Events().StreamEventsAsync(cancellationToken: ct))`.
  Server-sent events by default, through the pipeline; `Streams = StreamTransport.Socket`
  puts every stream of the client on one `/v1/ws` socket, which reconnects (drawing on
  the retry budget) and issues its calls again when the server ends it, and ends them
  with the error when the key was revoked. The socket's upgrade goes through the client's
  handler (proxy, CA, mTLS) with the request id and user agent, but `ClientWebSocket`
  takes no pipeline, so your middleware does not see it. A stream silent for
  `StreamIdleTimeout` (45 s) fails with a timeout; an `error` event or frame is an
  `ApiException` with its code and status. On .NET 9 and later the socket's pings close a
  dead connection after the idle timeout; on .NET 8 that is left to TCP.
- `https` only, plain `http` only to this machine; redirects are never followed.

- API documentation: <https://docs.inorbit.hr>
- How the SDKs behave, in every language: [docs/design.md](../docs/design.md) and
  [docs/config.md](../docs/config.md)
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md)
