# spec/

The contract the SDKs implement, vendored from the platform. **Never edit these files by
hand.** `mise run spec:sync` refreshes them; CI checks them; a mistake is fixed upstream.

| File | What it is | Produced by |
|---|---|---|
| `SOURCE` | Where and when the files were synced from, and the API version | `spec:sync` |
| `openapi.json` | The public slice of the platform's OpenAPI 3.1 document, normalised for code generators | `spec:sync` (fetch, filter, normalise); the same rules live in `cli/crates/iohr-openapi` for `iohr sdk generate`, kept byte-equal by a golden pair (`mise run cli:normalise-golden`) |
| `problem.json` | JSON Schema of the error envelope (`code`, `error`, `details`) | `spec:sync`, extracted from the `Problem`, `Code` and `Detail` components (rule N6) |
| `frames.json` | JSON Schema of `/v1/ws` client and server frames | added when the socket opens to API keys (ADR 0004) |

## How a sync works

1. Fetch `https://api.inorbit.hr/openapi.json` (no credentials needed). `uv run --script
   tools/spec-sync.py FILE_OR_URL` syncs from another document, such as a local build.
2. Keep only operations marked `x-iohr-public: true`, plus the schemas they reach.
3. Apply the normalisation rules below.
4. Write the files and `SOURCE`; `mise run gen` regenerates the models; the PR shows both.

The output is deterministic: the same document gives the same files, and `SOURCE` keeps
its `synced` date while the document's `sha256` is unchanged. `--check` writes nothing
and exits 1 when `spec/` would change. `SOURCE` records the URL, the document's
`sha256`, the API version and the number of operations; the platform commit is added
once the document carries it (an upstream ask below).

## Normalisation rules

Each rule exists because of a quirk in the platform document. Each is also an upstream
request; when the platform fixes the cause, the rule is deleted.

| # | Quirk in the platform document | Rule here | Upstream ask |
|---|---|---|---|
| N1 | Schema names are proto full names (`iohr.accounts.v1.GetMeResponse`); dots break several generators | Rename to the last segment (`GetMeResponse`). When two names clash, a schema without a package keeps the short name and the proto one takes its package as a prefix (`iohr.accounts.v1.Key` becomes `AccountsKey`, beside the gateway's own `Key`); any clash left fails the sync | Emit short, unique names |
| N2 | Transcoded schemas declare no `required` fields although the server always sends every field | Mark every property of a transcoded (proto) schema `required`; the gateway's own schemas (`Me`, `Key`, `Problem`) keep theirs | Emit `required` |
| N3 | `oneOf` unions (`Detail`) have no `discriminator` though each variant fixes a required `type` or `kind` | Add `discriminator.propertyName`; a union with `null` (an optional field) is left alone | Emit the discriminator |
| N4 | `servers` is `/` | Set `https://api.inorbit.hr` | Use the production server URL in the served document |
| N5 | Unset timestamps are `""`, not absent or `null` | Keep `type: string`; the SDKs map `""` to "no value" | Document it, or emit `null` |
| N6 | The code-to-status table lives only in code and prose (docs.inorbit.hr/docs/errors) | Add it to `problem.json` as `x-http-status`, from the table in `tools/spec-sync.py`; a code the table lacks, or a code the document dropped, fails the sync | Put the table in the document |

A finding from the first live run (2026-10-03): the gateway does send every scalar
field, but a field that is itself a message (`Change.example`) is left out when it was
never set. N2 stays, because the generated SDKs read such a field as optional
(`Option<T>` in Rust, see `cli/crates/iohr-codegen`); the upstream ask is for the
document to say `required` only for what is always sent.

## Other upstream asks (not normalised)

- The document's `info.license` is "Proprietary" and its title is the internal name. The
  public slice should carry a public title and a licence that matches publishing it.
- Gateway errors (401, 403, 429 from the edge) are plain text, not the envelope. Return
  the envelope from the edge too, with `Retry-After` on 429.
- No `x-request-id` or trace id comes back on responses; return one so users can quote
  it in a support request.
- The document carries no platform commit or build id; `SOURCE` can only record a hash.
  Add one (an `info.x-iohr-commit`) so a synced spec names the code it came from.
