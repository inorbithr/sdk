# 0004. REST first; SSE, socket and MCP when the platform opens them

Status: accepted, 2026-10-01; amended by [0008](0008-key-scopes-and-iohr-identifiers.md) (scopes and identifiers); server-sent events and the socket shipped in 0.3.0 (2026-10-04) once the platform opened them to keys (platform RFC 0048)

## Context

The platform serves REST, server-sent events, a multiplexed WebSocket (`/v1/ws`) and MCP.
An API key's token carries the `tbd.public` scope, and the gateway admits that scope only
on three `GET` routes (`docs/design.md` section 1). The socket, SSE routes and MCP refuse
a key today, and MCP gives a key no tools. The platform has no `Idempotency-Key` support
yet.

## Decision

- 0.x ships the client, authentication, errors, retries, hooks and the three REST
  operations, in all four languages.
- SSE, the socket and MCP are designed in `docs/design.md` section 7 and get conformance
  cases ahead of time, but no public API until the platform admits `tbd.public` on them.
- No idempotency key is sent until the platform supports it; `POST` is not retried
  automatically meanwhile.

## Consequences

- Nothing in the SDK promises a transport a key cannot use.
- When the platform opens a transport, the work is: sync spec, enable the cases,
  implement in four languages, one minor release each.
