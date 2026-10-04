# InOrbit SDK for Java

The InOrbit API client for Java 17 and newer: one runtime (`hr.inorbit:inorbit-sdk`), and
the operations your credential may call. Built and tested in the open; the release to
Maven Central (`hr.inorbit`, namespace verified) is planned and has not happened yet.
Until then, build it from this repository (`mise run java:check`, or `mvn -f java install`
to put it in your local Maven repository) and depend on it as it will be published:

```xml
<dependency>
  <groupId>hr.inorbit</groupId>
  <artifactId>inorbit-sdk</artifactId>
  <version>0.2.0</version>
</dependency>
```

## First call

[`examples/java`](../examples/java/src/main/java/example/Whoami.java), with `INORBIT_TOKEN`
set (an API token from the console or `iohr token create`):

```java
Public api = Public.fromEnv();
Me me = api.me().value();
ListDigestsResponse page = api.radar()
        .listDigests(RadarListDigestsParams.builder().limit(3).build())
        .value();
```

`Public` is the public surface, in `hr.inorbit.sdk.generated`. For an API key, set
`INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` and `INORBIT_SCOPES` instead; the client exchanges
the key for 15-minute tokens and refreshes them. Every call has an `...Async` twin that
returns a `CompletableFuture`.

## A surface cut to your account

```sh
iohr sdk generate --lang java --for ci --package com.example.inorbit \
  --out src/main/java/com/example/inorbit
```

writes one class per profile (`Ci`) with only the operations that profile's credential may
call, records for the models, and `iohr.lock` beside the directory. A call the profile may
not make does not compile. `Ci.fromEnv()` reads `INORBIT_CI_TOKEN` (or `INORBIT_CI_KEY_ID`,
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
`ConfigException`, `TooLargeException`, `DecodeException`. Idempotent calls are retried on
429, 503, 504 and connection failures, honouring `Retry-After`; writes are not.

- API documentation: <https://docs.inorbit.hr>
- How the SDKs behave, in every language: [docs/design.md](../docs/design.md)
- Contributing: [CONTRIBUTING.md](../CONTRIBUTING.md)
