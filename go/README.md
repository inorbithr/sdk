# InOrbit SDK for Go

Released since 0.1.0, Go 1.26 or later. One dependency,
[`github.com/BurntSushi/toml`](https://github.com/BurntSushi/toml) (no dependencies of
its own), to read the `iohr` config file; everything else is the standard library.

```sh
go get github.com/inorbithr/sdk/go@latest
```

```go
import (
	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

// The environment, the iohr config file and the iohr login, in that order.
api, err := public.Load(ctx)
if err != nil {
	return err // a *inorbit.ConfigError lists every problem, or every source tried
}
me, err := api.Me(ctx)
var apiErr *inorbit.APIError
if errors.As(err, &apiErr) && apiErr.Code == inorbit.CodeForbidden {
	// the token lacks identity:read
}
```

`Load` finds credentials the way the [configuration contract](../docs/config.md) says,
the first source that has any winning:

1. code: `inorbit.WithToken`, `WithKey` (or `WithKeyFile`) with `WithScopes`,
   `WithTokenFile`, or `WithTokenProvider`;
2. the environment: `INORBIT_TOKEN`, `INORBIT_TOKEN_FILE`, or `INORBIT_KEY_ID` with
   `INORBIT_KEY_SECRET` (or `INORBIT_KEY_SECRET_FILE`, read before every token exchange)
   and `INORBIT_SCOPES`;
3. workload identity: reserved until the platform offers it;
4. the profile's table in the `iohr` config file (`token_file`, or `key_id` with
   `key_secret_file`);
5. the developer's `iohr login`, through `iohr auth token`.

After `iohr login`, a program needs nothing else. In production, pin the source:
`INORBIT_CREDENTIAL_SOURCES=env`. Every other setting (timeouts, retries, proxy, CA
bundle, client certificate, logging, rate limits) resolves on its own from code
(`inorbit.With...` options), then `INORBIT_*`, then the config file, then its default.
To see what a client will use and where each value came from, secrets redacted:

```go
doc, _ := json.MarshalIndent(api.Runtime().Config().Describe(), "", "  ")
fmt.Println(string(doc))
```

`iohr sdk config` prints the same document from the command line, and
`inorbit.LoadConfig` returns it without building a client. A bad value fails `Load` with
one `*inorbit.ConfigError` whose `Problems()` list every setting at fault and its source.
`inorbit.WithLoadOptions` hands `Load` an environment, an OS and a home directory, so a
test resolves a configuration without touching the process.
[docs/recipes.md](../docs/recipes.md) has ready setups for CI, Kubernetes, corporate
proxies, mTLS gateways and serverless.

`inorbit.NewClient(...)` reads code only, for libraries and tests; `FromEnv` keeps its
old behaviour (six variables, credentials and URLs) and is superseded by `Load`. A
generated profile has the same: `acmeci.Load(ctx)` reads `INORBIT_ACME_CI_*` (then
`INORBIT_*` for settings) and `[profiles.acme-ci]`, and nothing can point it elsewhere.

## Middleware

Every call goes through a named pipeline: `request_id`, `user_agent`,
`idempotency_key`, `call_tracing` and `deadline` once per call, then `retry`, then
`auth`, `rate_limit`, `attempt_tracing`, `logging`, `hooks` and `timeout` on every
attempt. A middleware is a name and a function that wraps the rest of the pipeline in an
`http.RoundTripper`, so existing wrappers such as `otelhttp.NewTransport` fit as they
are. Add your own at either stage, or before, after or instead of any built-in:

```go
team := inorbit.Middleware{Name: "team", Wrap: func(next http.RoundTripper) http.RoundTripper {
	return inorbit.RoundTripperFunc(func(r *http.Request) (*http.Response, error) {
		r.Header.Set("X-Team", "payments") // sees the finished request, token included
		return next.RoundTrip(r)
	})
}}
api, err := public.Load(ctx, inorbit.WithPipeline(func(p *inorbit.Pipeline) {
	p.AddPerRetry(team)
	p.Remove("rate_limit")
}))
```

`inorbit.CallInfoFrom(r.Context())` says what the call is (operation, path template,
attempt, request id, idempotency key, deadline, stage). `retry`, `auth` and `timeout`
can be replaced but not removed. A middleware must not log secrets or bodies, and must
not read a stream's body (`CallInfo.Stream`). A circuit breaker such as `failsafe-go`
belongs at the per-call slot (`AddPerCall`). See
[examples/go/load](../examples/go/load/main.go).

## Behaviour

- Every call takes a context first; per-call options travel in it:
  `inorbit.WithCallOptions(ctx, inorbit.CallOptions{IdempotencyKey: "order-42",
  Traceparent: ..., Timeout: 5 * time.Second})`.
- Writes whose operation takes an `Idempotency-Key` (the create and trigger operations
  that declare it, such as `Events().CreateEndpoint`) get one per call, sent unchanged on
  every attempt, so they are retried like reads. Pass your own with `CallOptions`; it
  comes back on the result and the error (`RawResponse.IdempotencyKey`,
  `IdempotencyReplayed`, `ConnectionError.IdempotencyKey`).
- Retries: 2 by default, full-jitter backoff, `Retry-After` in seconds or as an HTTP date
  up to `WithRetryAfterMax` (60 s), and a per-client retry budget (a token bucket) that
  turns an outage into fast failures. `inorbit.RetryHook` hears of every retry. Every
  call has a total deadline, `WithTotalTimeout` (120 s); a shorter deadline on `ctx`
  wins, and `CallOptions.Timeout` shortens it.
- Rate limits: every result carries `Raw.RateLimit` (`Limit`, `Remaining`, `Reset`), and
  `client.RateLimit()` the latest; `WithRateLimit(inorbit.RateLimitWait)` holds a call
  until an empty window resets.
- Logging is off until `log` is set (`INORBIT_LOG=info`, or `WithLog`); records go to
  `log/slog`, your `WithLogger` or `slog.Default()`. They hold metadata only: never a
  body, a query value, a token or a cookie. Headers appear only with `log_headers`,
  values only from the allowlist. `WithRedact` sees every record last. `SlogHook` stays
  and is superseded by it.
- OpenTelemetry is the optional module `github.com/inorbithr/sdk/go/otel` (`go get
  github.com/inorbithr/sdk/go/otel@latest`), so this one keeps its single dependency:
  `inorbit.Load(ctx, inorbitotel.Pipeline())` makes each call an `INTERNAL` span with one
  `CLIENT` span per attempt, following the HTTP client conventions, sends `traceparent`
  from the attempt, and records the duration, retry and token exchange metrics. `INORBIT_TRACING=false` turns spans off; a `traceparent` you
  pass per call is then sent as is.
- A paged list also has an iterator, `All<Operation>`, returning `iter.Seq2[T, error]`;
  it follows the page tokens and fetches nothing more once the loop breaks:

  ```go
  for d, err := range api.Events().AllListDeliveries(ctx, endpointID, nil) {
  	if err != nil {
  		return err
  	}
  	fmt.Println(d.ID)
  }
  ```

- A streaming operation (the account's events, `StreamEvents`, scope `events:read`)
  returns `iter.Seq2[*T, error]`: each event as it arrives, an error ends the stream,
  and breaking out of the loop or ending `ctx` closes it. It opens through the pipeline
  like any GET (a fresh token after a 401, retries after a 429, 503 or 504). By default
  each stream is one server-sent events request; `inorbit.WithStreams(inorbit.StreamsSocket)`
  carries every stream of the client over one `/v1/ws` socket, whose upgrade goes
  through the pipeline too, which reconnects (drawing on the retry budget) and resumes
  its streams when the server ends it. A stream silent for 45 s (`WithStreamIdleTimeout`)
  fails with a `*TimeoutError`; a revoked key ends it with an `unauthenticated`
  `*APIError`.

  ```go
  for ev, err := range api.Events().StreamEvents(ctx, nil) {
  	if err != nil {
  		return err
  	}
  	fmt.Println(ev.Type, ev.ID)
  }
  ```

- Every request field is optional and left out when unset, so a scalar is a pointer:
  `models.CreateEndpointRequest{URL: inorbit.Ptr("https://example.com/hook")}`. An
  answer's field is a pointer unless the API always sends it. Timestamps stay the
  strings the API sent; `inorbit.ParseTimestamp` reads one, `""` (unset) as the zero
  `time.Time`.
- Errors are `*inorbit.APIError`, `*ConnectionError`, `*TimeoutError`, `*AuthError`,
  `*ConfigError`, `*TooLargeError` and `*DecodeError`, told apart with `errors.As`.
  Credentials never print their secret.

## Transport

`Load` builds its own `http.Transport`: HTTP/2 where the server offers it, gzip, and

- the proxy: `proxy` (`WithProxy`, `INORBIT_PROXY`, the file, then `https_proxy` and
  `HTTPS_PROXY`; `HTTP_PROXY` and `ALL_PROXY` are not read), with the SDK's own
  `no_proxy` rules, CONNECT for every request (the token exchange and the socket
  included), and user-info sent as `Proxy-Authorization`. A proxy from the standard
  variables skips loopback; one set on purpose does not;
- `ca_bundle`, added to the system trust store (`system_trust = false` trusts it
  alone);
- `client_cert` and `client_key` (mTLS; an encrypted PKCS#8 key with
  `INORBIT_CLIENT_KEY_PASSWORD`), read again when they change, checked at most once a
  minute, so cert-manager rotation needs no restart;
- `pinned_keys` and `connect_timeout`.

To use your own transport instead, pass `inorbit.WithHTTPClient` (copied, redirects
off) or `inorbit.WithTransport`; the pipeline still runs in full. Proxy and TLS settings
then belong to it: set in code next to it they are a `*ConfigError`, from the
environment or the file they are ignored and listed in `Describe()`. `NewClient` without
transport options keeps Go's default transport, as before.

## More

- `inorbit` is the runtime: the client, its options, the pipeline, credentials
  (`CachedToken`, `TokenFile`, `CliToken`, `ChainedCredential`, `DefaultCredential`,
  `ClientCredentials`, `StaticToken`), errors and `Int64`. `public` is the surface an
  API token or key may call, generated by `iohr sdk generate`. For a surface cut to your
  own credentials, one package per profile, run
  `iohr sdk generate --lang go --for <profile> --out iohr` inside your module
  ([the command line](../cli/README.md)); a call a profile may not make does not build.
- Examples: [examples/go](../examples/go), and the `Example` functions on pkg.go.dev.
  How the SDKs behave in every language: [docs/design.md](../docs/design.md) and
  [docs/config.md](../docs/config.md).
