# 0008. Per-key scopes and the iohr identifiers

Status: accepted, 2026-10-01. Amends [0004](0004-transport-scope.md) and the
authentication part of `docs/design.md`.

## Context

ADR 0004 described a single `tbd.public` scope admitted on three `GET` routes. The
platform has since changed in two ways:

- An API key now holds the scopes ticked when it was made: `identity:read`,
  `account:read`, `usage:read`, `radar:read`. A token asks for a subset in the client
  credentials exchange; each route needs one scope; asking for a scope the key lacks is
  `invalid_scope` (docs.inorbit.hr, Authentication, 2026-10-01).
- The platform's outside names moved from `tbd` to `iohr` on 2026-10-01: audience
  `iohr-api`, OpenAPI extensions `x-iohr-*`, protobuf packages `iohr.<svc>.v1`.

## Decision

- The client takes a `scopes` option (env `INORBIT_SCOPES`) with no default and sends it
  as the exchange's `scope`; the audience is always `iohr-api`.
- The SDK wraps the operations listed in `docs/design.md` section 1 with their scopes.
- 0004's transport rule stands: REST only until the platform admits API keys on SSE, the
  socket and MCP.

## Consequences

- A misconfigured client fails at construction or with an `AuthError` that names the
  scope, not with an opaque 403 on the first call.
- Conformance cases use `scope: identity:read` and `audience: iohr-api`; the driver's
  default client asks for `identity:read`.
