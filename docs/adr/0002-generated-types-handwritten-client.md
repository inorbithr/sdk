# 0002. Generated types, handwritten client

Status: accepted, 2026-10-01

## Context

Fully generated SDKs (openapi-generator, the Ory approach) are cheap to keep current but
read poorly and churn on generator upgrades; Svix moved off openapi-generator for that
reason. Fully handwritten SDKs read well but drift from the API. Stainless, the usual
commercial answer, announced in May 2026 that it is winding down its hosted generator.
The platform's OpenAPI document has quirks a generator must be shielded from
(`spec/README.md`): proto-style dotted schema names, no `required` lists, `oneOf` without
a discriminator.

## Decision

- **Generated:** the request and response models only, from `spec/openapi.json`, into
  `<lang>/.../generated/`. Proposed generators, each pinned in `mise.toml`:
  Go `oapi-codegen` (models only), Rust `typify`, TypeScript `openapi-typescript`
  (types only, no runtime), Python `datamodel-code-generator` (pydantic v2 models).
- **Handwritten:** the client, configuration, token provider, retries, error types,
  streaming, hooks, and the thin per-operation methods that call the transport.
- **Normalisation before generation:** `tools/spec-normalize` turns the platform document
  into `spec/openapi.json` (public operations only, short schema names, `required` added
  where the server always sends the field, discriminators on `type`/`kind`). Each rule
  is listed in `spec/README.md` and also raised upstream; a rule is removed when the
  platform fixes the cause.
- `mise run gen` regenerates everything; CI fails if the committed output differs.
- If the generators become the bottleneck, the fallback is a small template-based
  generator of our own (the Svix route), not a vendor.

## Consequences

- Models never drift from the contract; the part users touch most is written for them.
- Adding an operation is: sync spec, regenerate, write four thin methods, add a case and
  an example. The `/add-endpoint` skill walks through it.
- Generated code is marked `linguist-generated` and excluded from review diffs.

## Sources

- https://www.svix.com/blog/openapi-codegen/
- https://github.com/oapi-codegen/oapi-codegen
- https://github.com/oxidecomputer/typify
- https://openapi-ts.dev
- https://github.com/koxudaxi/datamodel-code-generator
