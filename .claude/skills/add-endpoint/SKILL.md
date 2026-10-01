---
name: add-endpoint
description: Add a public API operation to all four SDKs (Go, Rust, TypeScript, Python) with a conformance case and an example. Use when the platform exposes a new operation to API keys or when asked to support an endpoint.
argument-hint: "[operationId or METHOD /v1/path]"
---

# Add an endpoint: $ARGUMENTS

Work in this order. Do not skip a language; if one cannot follow, say so in the PR and
open an issue labelled `parity`.

1. **Contract.** Confirm the operation is in `spec/openapi.json` and marked public. If
   not, run `/sync-spec` first. If the platform does not expose it to API keys yet, stop
   and report that; the SDK never wraps an operation a key cannot call (ADR 0004).
2. **Name it.** Pick the SDK name from `docs/design.md` conventions (resource group plus
   verb: `accounts.get_usage`). Write the four spellings down before coding:
   Go `Accounts.GetUsage`, Rust `accounts().get_usage()`, TS `accounts.getUsage()`,
   Python `accounts.get_usage()`.
3. **Behaviour first.** Add `conformance/cases/operations/<name>.yaml` covering the
   request shape (path, query, body) and one error. Add the op to the `action.op` enum in
   `conformance/case.schema.json`. Run `mise run conformance:validate`.
4. **Generate.** `mise run gen`. Never edit the generated files.
5. **Implement**, one language at a time, following that language's `AGENTS.md`:
   a thin method that builds the request, calls the shared transport and converts the
   result to public types. No retry, auth or error logic in the method; that lives in
   the transport.
6. **Examples.** Extend the relevant program under `examples/<lang>/`, or add one if this
   is a new area.
7. **Docs.** Add the operation to the scope table in `docs/design.md` section 1.
8. **Verify.** `mise run <lang>:check` for each language, `mise run conformance`, then
   `mise run ci`. Use the `parity-checker` agent on the diff before calling it done.

Commit per language scope or as one `feat:` commit with a body listing all four; never
mix an unrelated change in.
