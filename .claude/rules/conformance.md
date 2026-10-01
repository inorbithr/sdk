---
paths:
  - "conformance/**"
  - "**/conformance/**"
---

# Conformance cases

- A case describes behaviour the user can observe, named for the behaviour
  (`refresh-once-on-401`), never for a bug or a language.
- File name equals `name`; directory equals `area`. `mise run conformance:validate`
  checks both and the schema.
- Exchanges are exact and ordered: the replay server fails the case on any request the
  case does not list. Request matching is a subset match on headers, query, form and
  JSON; keep matchers to what the behaviour needs.
- Use the default client (`ak_test` / `s3cr3t`, 2 retries) unless the case is about an
  option.
- Never weaken a case to make one language pass. Mark it `pending: [lang]` with an issue
  link in `notes` instead.
- A driver translates a case into calls on the public API only; it never reaches into
  internals, so a passing case proves what users get.
