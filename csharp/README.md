# InOrbit SDK for .NET

The runtime `InOrbit.Sdk` (`net8.0`) and the public surface generated into it. Not on
NuGet yet: until the first release (milestone M4), reference the project from this
repository.

```csharp
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

using var client = Client.FromEnv(); // INORBIT_TOKEN, or INORBIT_KEY_ID, _KEY_SECRET and _SCOPES
var me = (await client.MeAsync()).Value;
var page = (await client.Radar().ListDigestsAsync(new RadarListDigestsParams { Limit = 3 })).Value;
```

[examples/csharp](../examples/csharp) holds this program in full.

## A surface for your own credentials

`iohr sdk generate --lang csharp --for ci --package Acme.Api --out src/Iohr` writes the
models and the operations profile `ci` may call into your project, in namespace
`Acme.Api`, with `iohr.lock` beside it. Each profile is a type; an operation is an
extension method constrained to the profiles whose cut holds it, so a call a profile may
not make does not compile:

```csharp
using var ci = Client.FromEnv<Ci>();          // INORBIT_CI_TOKEN, or INORBIT_CI_KEY_ID ...; nothing else
await ci.Accounts().GetUsageAsync("acc_1");   // compiles only if ci's cut holds usage:read
```

## What the runtime does

- Credentials: an API token, an API key exchanged for 15-minute tokens (one exchange
  however many calls wait, refreshed at a fifth of its life and after a 401), or your
  own `ITokenProvider`. `ToString()` never shows a secret.
- Retries on 429, 503, 504 and connection failures, for idempotent calls only, honouring
  `Retry-After` up to a minute; 30-second attempts; answers capped at 16 MiB.
- Errors: `InOrbitException` and its kinds; `ApiException` carries the `Code` (unknown
  codes kept as the API wrote them), the HTTP status, typed details and the raw answer.
- 64-bit integers are `long`, sent and read as decimal strings.
- A paged list has an `All<Operation>Async` beside its page method, an
  `IAsyncEnumerable<T>` that follows the next-page token:
  `await foreach (var d in client.Radar().AllListDigestsAsync(cancellationToken: ct))`;
  breaking out fetches nothing more, and the token stops it between pages.
- No dependency beyond the framework (`HttpClient`, `System.Text.Json`); `https` only,
  plain `http` only to this machine.

- API documentation: <https://docs.inorbit.hr>
- How the SDKs behave, in every language: [docs/design.md](../docs/design.md)
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md)
