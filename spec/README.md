# spec/

The contract the SDKs implement, vendored from the platform. **Never edit these files by
hand.** `mise run spec:sync` refreshes them; CI checks them; a mistake is fixed upstream.

| File | What it is | Produced by |
|---|---|---|
| `SOURCE` | Where and when the files were synced from, and the API version | `spec:sync` |
| `openapi.json` | The public slice of the platform's OpenAPI 3.1 document, normalised for code generators | `spec:sync` (fetch, filter, normalise, check); the same rules and checks live in `cli/crates/iohr-openapi` for `iohr sdk generate`, kept byte-equal by a golden pair (`mise run cli:normalise-golden`) whose raw document is the live one `spec/` came from |
| `problem.json` | JSON Schema of the error envelope (`code`, `error`, `details`, `request_id`), with the document's code-to-status table (`Code.x-http-status`) | `spec:sync`, extracted from the `Problem`, `Code` and `Detail` components |
| `lab/rules.json`, `lab/conformance.json` | The lab's generic redaction rules and the fixture documents with the findings expected of each (RFC 0035), which `iohr lab check` embeds and is tested against | `spec:sync`, from `https://docs.inorbit.hr/lab/`, checked for shape (no lookaround in a rule); `--only lab` syncs them without touching the contract |
| `frames.json` | JSON Schema of `/v1/ws` client and server frames | added when the socket opens to API keys (ADR 0004) |

## How a sync works

1. Fetch `https://api.inorbit.hr/openapi.json` (no credentials needed). `uv run --script
   tools/spec-sync.py FILE_OR_URL` syncs from another document, such as a local build.
2. Keep only operations marked `x-iohr-public: true`, plus the schemas they reach.
3. Apply the normalisation rules below, and check the facts the platform states itself
   (the former rules N2, N3, N4, N6): a document without one fails the sync.
4. Write the files and `SOURCE`; `mise run gen` regenerates the models; the PR shows both.

The output is deterministic: the same document gives the same files, and `SOURCE` keeps
its `synced` date while the document's `sha256` is unchanged. `--check` writes nothing
and exits 1 when `spec/` would change. `SOURCE` records the URL, the document's
`sha256`, the API version and the number of operations; the platform commit is added
once the document carries it (an upstream ask below).

## Normalisation rules

Each rule exists because of a quirk in the platform document, or because the behaviour
is settled as it is. A quirk the platform fixes leaves this table for the next one: the
sync then checks the fact instead of patching it in, so a regression upstream fails
the sync (and `iohr sdk generate`) rather than being quietly repaired here.

| # | Quirk in the platform document | Rule here | Status |
|---|---|---|---|
| N1 | Schema names are proto full names (`iohr.accounts.v1.GetMeResponse`); dots break several generators | Rename to the last segment (`GetMeResponse`). When two names clash, a schema without a package keeps the short name and the proto one takes its package as a prefix (`iohr.accounts.v1.Key` becomes `AccountsKey`, beside the gateway's own `Key`); any clash left fails the sync | Ours: naming for generators, kept |
| N5 | Unset timestamps are `""`, not absent or `null` | Keep `type: string`; models keep the string, and each runtime's timestamp helper reads `""` as no value (`docs/design.md`, section 12) | Settled behaviour, documented; not an upstream ask |

### Fixed upstream, now checked

Fixed on 2026-10-04 by the platform (core #218, RFC 0033); the rules were deleted and
each is now a check that fails the sync when the fact is missing.

| # | Was | The document now says | Check |
|---|---|---|---|
| N2 | No `required` anywhere; the sync marked every field of a transcoded message required | A message only an answer carries lists as `required` exactly the fields without presence (always sent; held by a platform test against the serializer). A field with presence (a message, `optional`, `oneof`) is never required. A message a request can carry marks nothing required | Some answer message marks `required`, and every name it lists is one of its properties |
| N3 | `Detail` had no `discriminator` | `discriminator: {propertyName: type}` | `Detail` names `type` |
| N4 | `servers` was `/` | `servers[0].url` is `https://api.inorbit.hr` | `servers[0].url` is an absolute https URL |
| N6 | The code-to-status table lived only in code and prose | `Code` carries `x-http-status`, one status per code | Every code has an HTTP status, and no status names a code the enum lacks |

So a generated request type has every field optional (unset fields are left out of the
body), and a generated answer type has exactly the document's required fields present.

## Other upstream asks (not normalised)

- The document's `info.license` is "Proprietary". The public slice should carry a
  licence that matches publishing it.
- Gateway errors (401, 403, 429 from the edge) are plain text, not the envelope. Return
  the envelope from the edge too, with `Retry-After` on 429.
- (Done upstream: every answer carries `x-request-id`, and the envelope its
  `request_id`.)
- The document carries no platform commit or build id; `SOURCE` can only record a hash.
  Add one (an `info.x-iohr-commit`) so a synced spec names the code it came from.

## Accepted breaking changes

`mise run spec:lint` runs `oasdiff breaking` against `main` and fails on a breaking change.
A change the platform made on purpose, and that the runtimes already handle, is recorded in
[`breaking-ignore.txt`](breaking-ignore.txt): one line per change (`METHOD /path` and oasdiff's
description) under a dated comment saying why it is safe. 2026-10-03: the error code
`unprocessable` (422); every runtime keeps a code it does not know. 2026-10-04: 116 answer
fields with presence became optional when N2 was fixed upstream (core #218); the wire did
not change, and the regenerated surfaces read them as optional in the same change.
