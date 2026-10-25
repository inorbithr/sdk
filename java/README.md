# InOrbit SDK for Java

The InOrbit API client for Java 17 and newer: one runtime (`hr.inorbit:inorbit-sdk`), and
the operations your credential may call. Built and tested in the open; the release to
Maven Central (`hr.inorbit`, namespace verified) is planned and has not happened yet.
Until then, build it from this repository (`mise run java:check`, or `mvn -f java install`
to put it in your local Maven repository) and depend on it as it will be published:

<!-- x-release-please-start-version -->
```xml
<dependency>
  <groupId>hr.inorbit</groupId>
  <artifactId>inorbit-sdk</artifactId>
  <version>0.3.0</version>
</dependency>
```
<!-- x-release-please-end -->

## First call

Run `iohr login` once (or set `INORBIT_TOKEN`), then:

```java
Client client = Client.load();
Public api = new Public(client); // or Public.load()
Me me = api.me().value();
ListDigestsResponse page = api.radar()
        .listDigests(RadarListDigestsParams.builder().limit(3).build())
        .value();
```

`Client.load()` takes each setting from code, then the environment (`INORBIT_*`), then
your profile in the `iohr` config file (`~/.config/iohr/config.toml` on Linux), then the
default ([docs/config.md](../docs/config.md)). Credentials come from the first source that
has any: code, `INORBIT_TOKEN` or `INORBIT_TOKEN_FILE` or `INORBIT_KEY_ID` with
`INORBIT_KEY_SECRET` (or `_KEY_SECRET_FILE`) and `INORBIT_SCOPES`, the config file's
profile, then your `iohr login` (it runs `iohr auth token`). `INORBIT_CREDENTIAL_SOURCES=env`
pins a production service to the environment. A key is exchanged for 15-minute tokens,
refreshed ahead of expiry; secret and token files are read again when they rotate. A bad
value fails `load` with one `ConfigException` that lists every problem with its source
(`problems()`), and `client.config().describe()` shows what was chosen and from where, as
`iohr sdk config` does. Options set in code win:

```java
Client client = Client.builder()
        .timeout(Duration.ofSeconds(5))
        .totalTimeout(Duration.ofSeconds(60))
        .log("info")                       // System.Logger "hr.inorbit.sdk"; off by default
        .load();
```

Every call has an `...Async` twin that returns a `CompletableFuture`. `Client.builder()...build()`
and `Client.fromEnv()` keep working as before and read nothing else (no config file).
[`examples/java`](../examples/java/src/main/java/example/) has `Whoami.java` and
`Configured.java`; [docs/recipes.md](../docs/recipes.md) has ready-made setups for
Kubernetes, CI, a corporate proxy and more.

## Middleware

Every call goes through a pipeline of named middlewares: `request_id`, `user_agent`,
`idempotency_key`, `call_tracing`, `deadline`, then `retry`, then per attempt `auth`,
`rate_limit`, `attempt_tracing`, `logging`, `hooks`, `timeout`. Add your own, or change the
built-ins by name:

```java
Middleware timing = Middleware.of("timing", (request, chain) -> {
    long started = System.nanoTime();
    var response = chain.proceed(request);
    System.out.println(request.info().operation() + " " + response.status() + " "
            + (System.nanoTime() - started) / 1_000_000 + " ms");
    return response;
});
Client client = Client.builder()
        .pipeline(p -> p.addPerRetry(timing).remove("rate_limit"))
        .load();
```

`addPerCall` runs once per call, `addPerRetry` once per attempt (it sees the finished
request, `Authorization` included: never log it); `insertBefore`, `insertAfter`, `replace`
and `remove` take a name. `retry`, `auth` and `timeout` can be replaced, not removed.
Per-call options go through a view of the client:
`client.withOptions(CallOptions.builder().idempotencyKey("order-42").build())`; a write
whose operation takes `Idempotency-Key` gets one key per call, sent on every attempt, and
is retried like a read. Retries draw from a per-client budget, so an outage fails fast.

Proxy (`INORBIT_PROXY`, `HTTPS_PROXY`, `NO_PROXY` by the SDK's own rules), an added CA
bundle, client certificates (PEM, or a `KeyStore`) and pinned keys are settings; a
caller-supplied `HttpClient` takes them over. With `opentelemetry-api` on your class path
the client makes one span per call and per attempt and sends `traceparent`; the SDK never
pulls OpenTelemetry in itself.

## A surface cut to your account

```sh
iohr sdk generate --lang java --for ci --package com.example.inorbit \
  --out src/main/java/com/example/inorbit
```

writes one class per profile (`Ci`) with only the operations that profile's credential may
call, records for the models, and `iohr.lock` beside the directory. A call the profile may
not make does not compile. `Ci.load()` resolves the profile `ci` as its own (`[profiles.ci]`, `INORBIT_CI_*`); `Ci.fromEnv()` reads `INORBIT_CI_TOKEN` (or `INORBIT_CI_KEY_ID`,
`_KEY_SECRET`, `_SCOPES`) and nothing else. Regenerate it, never edit it; `iohr sdk check`
in CI says when the API's cut has moved.

A paged list has an `all<Operation>` beside its page method: a lazy `Pages<T>`, an
`Iterable` with `stream()`, that follows the next-page token and fetches nothing more once
the loop stops (`for (Digest d : api.radar().allListDigests()) { ... }`).

Every request field is optional (unset in the builder, left out of the body); an answer's
field is `null` when the API may leave it out. Timestamps stay the strings the API sent;
`Timestamps.parse` reads one, `""` (unset) as an empty `Optional`.

## Streams

A stream operation (the account's events, `streamEvents`, scope `events:read`) answers an
`EventStream<T>`: an `Iterator`, an `Iterable` and an `AutoCloseable`. It opens on the first
step of the loop, with the same token and retry rules as any `GET`, and closing it (or
leaving the `try`) stops it:

```java
try (EventStream<StreamEventsResponse> events = api.events().streamEvents()) {
    for (StreamEventsResponse ev : events) {
        System.out.println(ev.type() + " " + ev.id());
    }
}
```

By default each stream is server-sent events. `Client.builder().streams(StreamTransport.SOCKET)`
runs every stream of the client as a call on one `/v1/ws` WebSocket instead
(`java.net.http.WebSocket`, no extra dependency); when the server ends that socket the client
opens a new one and issues the calls again, and a revoked key ends them with
`ApiException` `unauthenticated`. A stream silent for `streamIdleTimeout` (45 s; the server
keeps it alive every 15 s) fails with `TimeoutException`. An `error` event or frame ends the
stream with its `ApiException`. A stream ended by the server is not resumed: open it again.

## Errors

Every failure is an unchecked `InOrbitException`: `ApiException` (with `code()`,
`status()`, `details()`), and `ConnectionException`, `TimeoutException`, `AuthException`,
`ConfigException`, `TooLargeException`, `DecodeException`; `requestId()` and
`idempotencyKey()` name the call. Idempotent calls, and writes that take an idempotency
key, are retried on 429, 503, 504 and connection failures, honouring `Retry-After` (up to
`retryAfterMax`, 60 s) within the call's total timeout (120 s); other writes are not.

- API documentation: <https://docs.inorbit.hr>
- How the SDKs behave, in every language: [docs/design.md](../docs/design.md)
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md)
