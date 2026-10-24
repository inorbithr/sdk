# Examples

Small, complete programs, one directory per language. CI compiles every example
(`mise run examples:check`) and runs it (`mise run examples:run`), and the package READMEs
quote them, so an example that no longer builds or no longer works fails the PR.

## One example per call type

Each file is short, starts with a comment saying what it shows, and is run by
`mise run examples:run` against the conformance replay server with the case of the same
name in `conformance/cases/examples/`. The run writes `examples-results.json`
(language, example, status, duration_ms, commit per example) and fails if any example fails.
Run one language with `python3 tools/run-examples.py --lang go`, one call type with
`--only retries`. `examples.toml` lists the call types, the credentials each one uses and
how each language is built and run.

| Call type | Rust | TypeScript | Go | Python | Java | C# |
|---|---|---|---|---|---|---|
| `unary-get`: One unary call, GET /v1/me | [`unary-get.rs`](rust/src/bin/unary-get.rs) | [`unary-get.ts`](typescript/src/unary-get.ts) | [`unary-get`](go/unary-get/main.go) | [`unary_get.py`](python/unary_get.py) | [`UnaryGet.java`](java/src/main/java/example/UnaryGet.java) | [`UnaryGet.cs`](csharp/Calls/UnaryGet.cs) |
| `list-pagination`: A paged list, every item then one page by hand | [`list-pagination.rs`](rust/src/bin/list-pagination.rs) | [`list-pagination.ts`](typescript/src/list-pagination.ts) | [`list-pagination`](go/list-pagination/main.go) | [`list_pagination.py`](python/list_pagination.py) | [`ListPagination.java`](java/src/main/java/example/ListPagination.java) | [`ListPagination.cs`](csharp/Calls/ListPagination.cs) |
| `stream-sse`: The account's events over server-sent events | [`stream-sse.rs`](rust/src/bin/stream-sse.rs) | [`stream-sse.ts`](typescript/src/stream-sse.ts) | [`stream-sse`](go/stream-sse/main.go) | [`stream_sse.py`](python/stream_sse.py) | [`StreamSse.java`](java/src/main/java/example/StreamSse.java) | [`StreamSse.cs`](csharp/Calls/StreamSse.cs) |
| `stream-socket`: The same events over the /v1/ws socket | [`stream-socket.rs`](rust/src/bin/stream-socket.rs) | [`stream-socket.ts`](typescript/src/stream-socket.ts) | [`stream-socket`](go/stream-socket/main.go) | [`stream_socket.py`](python/stream_socket.py) | [`StreamSocket.java`](java/src/main/java/example/StreamSocket.java) | [`StreamSocket.cs`](csharp/Calls/StreamSocket.cs) |
| `typed-error`: A refused call as a typed error with a code | [`typed-error.rs`](rust/src/bin/typed-error.rs) | [`typed-error.ts`](typescript/src/typed-error.ts) | [`typed-error`](go/typed-error/main.go) | [`typed_error.py`](python/typed_error.py) | [`TypedError.java`](java/src/main/java/example/TypedError.java) | [`TypedError.cs`](csharp/Calls/TypedError.cs) |
| `auth-token`: A fixed API token from the environment | [`auth-token.rs`](rust/src/bin/auth-token.rs) | [`auth-token.ts`](typescript/src/auth-token.ts) | [`auth-token`](go/auth-token/main.go) | [`auth_token.py`](python/auth_token.py) | [`AuthToken.java`](java/src/main/java/example/AuthToken.java) | [`AuthToken.cs`](csharp/Calls/AuthToken.cs) |
| `auth-custom-token`: A token provider of your own | [`auth-custom-token.rs`](rust/src/bin/auth-custom-token.rs) | [`auth-custom-token.ts`](typescript/src/auth-custom-token.ts) | [`auth-custom-token`](go/auth-custom-token/main.go) | [`auth_custom_token.py`](python/auth_custom_token.py) | [`AuthCustomToken.java`](java/src/main/java/example/AuthCustomToken.java) | [`AuthCustomToken.cs`](csharp/Calls/AuthCustomToken.cs) |
| `retries`: A 429 with Retry-After, retried | [`retries.rs`](rust/src/bin/retries.rs) | [`retries.ts`](typescript/src/retries.ts) | [`retries`](go/retries/main.go) | [`retries.py`](python/retries.py) | [`Retries.java`](java/src/main/java/example/Retries.java) | [`Retries.cs`](csharp/Calls/Retries.cs) |
| `concurrent-calls`: Several calls at once on one client | [`concurrent-calls.rs`](rust/src/bin/concurrent-calls.rs) | [`concurrent-calls.ts`](typescript/src/concurrent-calls.ts) | [`concurrent-calls`](go/concurrent-calls/main.go) | [`concurrent_calls.py`](python/concurrent_calls.py) | [`ConcurrentCalls.java`](java/src/main/java/example/ConcurrentCalls.java) | [`ConcurrentCalls.cs`](csharp/Calls/ConcurrentCalls.cs) |
| `config-load`: A client from load, with where each setting came from | [`config-load.rs`](rust/src/bin/config-load.rs) | [`config-load.ts`](typescript/src/config-load.ts) | [`config-load`](go/config-load/main.go) | [`config_load.py`](python/config_load.py) | [`ConfigLoad.java`](java/src/main/java/example/ConfigLoad.java) | [`ConfigLoad.cs`](csharp/Calls/ConfigLoad.cs) |

The Python run uses `uv run --frozen`, so `examples/python/uv.lock` is not rewritten (uv
bumps the `inorbithr` version in it otherwise, and a dirty tree fails the gate). If you run
`uv run` by hand, revert the lock before attesting.

## Earlier examples

These predate the table above and stay. Some show the same thing as a call type
(`whoami` is `unary-get`, `load` is `config-load`, `events` is `stream-sse`,
`custom_token` is `auth-custom-token`); they are compiled by `examples:check` but not
replayed.

| Shows | Rust | TypeScript | Go | Python | Java | C# |
|---|---|---|---|---|---|
| Who the API thinks you are (`GET /v1/me`), with the credentials in the environment | [`whoami.rs`](rust/src/bin/whoami.rs) | [`whoami.ts`](typescript/src/whoami.ts) | [`whoami`](go/whoami/main.go) | [`whoami.py`](python/whoami.py), [`whoami_async.py`](python/whoami_async.py) | [`Whoami.java`](java/src/main/java/example/Whoami.java) | [`Whoami`](csharp/Whoami/Program.cs) |
| A client built with `load`, where each setting came from, a middleware of your own | [`load.rs`](rust/src/bin/load.rs), [`middleware.rs`](rust/src/bin/middleware.rs) | [`load.ts`](typescript/src/load.ts) | [`load`](go/load/main.go) | [`configured.py`](python/configured.py) | [`Configured.java`](java/src/main/java/example/Configured.java) | [`Load`](csharp/Load/Program.cs) |
| The account's events as they happen (`events:read`), over server-sent events or the `/v1/ws` socket | [`stream_events.rs`](rust/src/bin/stream_events.rs) | [`events.ts`](typescript/src/events.ts) | [`events`](go/events/main.go) | [`stream_events.py`](python/stream_events.py) | |
| A token provider of your own, and matching on error codes | [`custom_token.rs`](rust/src/bin/custom_token.rs) | | | | |

Running one needs an API token or key: create one in the console (API tokens and keys) or
with `iohr token create`, or sign in with `iohr login` for the `load` examples. Examples
never print the secret or the token.
