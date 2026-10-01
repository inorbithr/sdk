# spec/

The contract the SDKs implement, vendored from the platform. **Never edit these files by
hand.** `mise run spec:sync` refreshes them; CI checks them; a mistake is fixed upstream.

| File | What it is | Produced by |
|---|---|---|
| `SOURCE` | Where and when the files were synced from, and the API version | `spec:sync` |
| `openapi.json` | The public slice of the platform's OpenAPI 3.1 document, normalised for code generators | `spec:sync` (fetch, filter, normalise) |
| `problem.json` | JSON Schema of the error envelope (`code`, `error`, `details`) | `spec:sync`, extracted from the `Problem`, `Code` and `Detail` components (rule N6) |
| `frames.json` | JSON Schema of `/v1/ws` client and server frames | added when the socket opens to API keys (ADR 0004) |

## How a sync works

1. Fetch `https://api.inorbit.hr/openapi.json` (no credentials needed).
2. Keep only operations marked `x-tbd-public: true`, plus the schemas they reach.
3. Apply the normalisation rules below.
4. Write the files and `SOURCE`; `mise run gen` regenerates the models; the PR shows both.

## Normalisation rules

Each rule exists because of a quirk in the platform document. Each is also an upstream
request; when the platform fixes the cause, the rule is deleted.

| # | Quirk in the platform document | Rule here | Upstream ask |
|---|---|---|---|
| N1 | Schema names are proto full names (`tbd.accounts.v1.GetMeResponse`); dots break several generators | Rename to the last segment (`GetMeResponse`), refusing collisions | Emit short, unique names |
| N2 | Transcoded schemas declare no `required` fields although the server always sends every field | Mark every property `required` (the server emits defaults) | Emit `required` |
| N3 | `oneOf` unions (`Detail`, `EventBody`) have no `discriminator` though each variant has a `type` or `kind` enum | Add `discriminator.propertyName` | Emit the discriminator |
| N4 | `servers` is `/` | Set `https://api.inorbit.hr` | Use the production server URL in the served document |
| N5 | Unset timestamps are `""`, not absent or `null` | Keep `type: string`; the SDKs map `""` to "no value" | Document it, or emit `null` |
| N6 | The code-to-status table lives only in prose (docs.inorbit.hr/docs/errors) | Add it to `problem.json` as `x-http-status` | Put the table in the document |

## Other upstream asks (not normalised)

- `/openapi.json` on the API host serves all operations (179 on 2026-10-01), not only
  the 3 a key may call. Serve the public slice there, or confirm the full surface is
  meant to be public.
- The document's `info.license` is "Proprietary" and its title is the internal name. The
  public slice should carry a public title and a licence that matches publishing it.
- Gateway errors (401, 403, 429 from the edge) are plain text, not the envelope. Return
  the envelope from the edge too, with `Retry-After` on 429.
- No `x-request-id` or trace id comes back on responses; return one so users can quote
  it in a support request.
