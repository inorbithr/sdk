# 0001. One repository for all languages

Status: accepted, 2026-10-01

## Context

Four client libraries for one API. Prior art splits two ways: one repository per language
(Stripe, and the Stainless-generated OpenAI and Anthropic SDKs) or one repository with a
directory per language (Svix `svix-webhooks`, Ory `ory/sdk`). The API is small today and
will grow; one person maintains it, mostly with coding agents.

## Decision

One repository, `github.com/inorbithr/sdk`, with `go/`, `rust/`, `typescript/`, `python/`
beside `spec/`, `conformance/` and `examples/`.

## Consequences

- A contract change updates all four languages in one pull request, reviewed once.
- One set of conformance cases, one CI and security posture, one place for agent
  instructions.
- Each registry page shows a per-language README, so each package directory carries its
  own `README.md`.
- Go needs a module in a subdirectory: module path `github.com/inorbithr/sdk/go`, tags
  `go/vX.Y.Z` (see 0003).
- Issues for all languages share one tracker; labels `lang:go`, `lang:rust`, `lang:ts`,
  `lang:py` keep them apart.

## Sources

- https://github.com/svix/svix-webhooks
- https://github.com/ory/sdk
- https://go.dev/ref/mod#vcs-version
