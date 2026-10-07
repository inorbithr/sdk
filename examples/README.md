# Examples

Small, complete programs, one directory per language. CI compiles every example
(`mise run examples:check`) and the package READMEs quote them, so an example that no
longer builds fails the PR.

| Shows | Rust | TypeScript | Go | Python | Java | C# | Swift |
|---|---|---|---|---|---|---|---|
| Who the API thinks you are (`GET /v1/me`), with the credentials in the environment | [`whoami.rs`](rust/src/bin/whoami.rs) | [`whoami.ts`](typescript/src/whoami.ts) | [`whoami`](go/whoami/main.go) | [`whoami.py`](python/whoami.py), [`whoami_async.py`](python/whoami_async.py) | [`Whoami.java`](java/src/main/java/example/Whoami.java) | [`Whoami`](csharp/Whoami/Program.cs) | [`Whoami`](swift/Sources/Whoami/main.swift) |
| A client built with `load`, where each setting came from, a middleware of your own | [`load.rs`](rust/src/bin/load.rs), [`middleware.rs`](rust/src/bin/middleware.rs) | [`load.ts`](typescript/src/load.ts) | [`load`](go/load/main.go) | [`configured.py`](python/configured.py) | [`Configured.java`](java/src/main/java/example/Configured.java) | [`Load`](csharp/Load/Program.cs) | |
| The account's events as they happen (`events:read`), over server-sent events or the `/v1/ws` socket | [`stream_events.rs`](rust/src/bin/stream_events.rs) | [`events.ts`](typescript/src/events.ts) | [`events`](go/events/main.go) | [`stream_events.py`](python/stream_events.py) | | | |
| A token provider of your own, and matching on error codes | [`custom_token.rs`](rust/src/bin/custom_token.rs) | | | | | | |
| Every item of a paged list, then one page of another | | | | | | | [`ListMonitors`](swift/Sources/ListMonitors/main.swift) |

Running one needs an API token or key: create one in the console (API tokens and keys) or
with `iohr token create`, or sign in with `iohr login` for the `load` examples. Examples
never print the secret or the token.
