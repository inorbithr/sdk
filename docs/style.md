# Writing style

For READMEs, docs, doc comments, changelog-facing commit subjects and every error message
the SDK shows a user. Adapted from the platform's own style guide.

## Voice

- Reference docs and doc comments: neutral, contract first. Say what a function does,
  what it returns, what can fail. No humour, no selling.
- READMEs: short and practical. Install, authenticate, first call, errors, links. A
  README that needs scrolling to reach the first call is too long.
- Error messages: what failed, in the reader's words, then what to do.
  `token exchange failed: the key was revoked (unauthenticated). Create a new key in the
  console.` Not `An error occurred`.

## Rules

- Every number carries its unit and context: "15-minute tokens", "2 retries by default".
- Absolute dates (2026-10-01), never "recently" or "soon".
- Name things the way the API names them. If the API says `org_id`, the docs do too.
- One example per concept, and it compiles: examples come from `examples/`, never typed
  freehand into a README.
- Say what is not supported yet, plainly: "Server-sent events are not available to API
  keys yet."
- Headings in sentence case, short. No emoji. No em dashes. Bold only for terms being
  defined.

## Words to avoid

seamless, robust, powerful, cutting-edge, leverage, unlock, empower, best-in-class,
effortless, simply, just (as in "just call"), "it's worth noting", "let's dive in".

## Doc comments per language

- Go: full sentences starting with the identifier (`// GetMe returns ...`), as `go doc`
  expects.
- Rust: a one-line summary, then `# Errors` for every fallible public function and
  `# Examples` that compile as doctests.
- TypeScript: TSDoc with `@param`, `@returns`, `@throws`; one `@example`.
- Python: Google-style docstrings with `Args`, `Returns`, `Raises`.
