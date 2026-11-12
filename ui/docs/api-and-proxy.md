# API and proxy

The page never holds a credential. This guide shows how it still reads your account.
Back to the [UI overview](../README.md). The server side, with every refusal it makes, is
in [`iohr ui`](../../cli/docs/commands.md#iohr-ui).

## The model

`iohr ui serve` signs in the way `iohr login` does (same profile, same credential store)
and calls the API for the page. The page talks only to its own origin:

```text
page  --  x-iohr-token  -->  iohr ui serve (loopback)  --  your credential  -->  the API
```

- The per-serve token is a 64-character hex string the server puts in
  `<meta name="iohr-token">`. `startToken()` in `lib/panels.ts` reads it and returns
  `null` when it is missing or malformed.
- Reaching the page at all takes a one-time launch secret and a session cookie. Open the
  page with `iohr ui open`.
- Every call sends the token as the `x-iohr-token` header, with `cache: "no-store"`.

## Client functions

All are in `src/lib/api.ts` and `src/lib/panels.ts`. Each takes the token and a `fetch`
as optional arguments, so tests pass fakes. None throws; each returns a value that says
what happened.

| Function | Request | Returns |
|---|---|---|
| `readSession()` | `GET /api/session` | `signed-in` (profile, kind, account, subject, scopes, login phase), `signed-out` or `unreachable`. Never a token |
| `signIn()` | `POST /api/session/login` | `true` once the server started `iohr login --web` (your browser opens) |
| `signOut()` | `POST /api/session/logout` | `true` once it did, as `iohr logout` |
| `apiGet(path, query)` | `GET /api/proxy/v1/...` | `{state: "ok", data}`, `{state: "signed-out"}` or `{state: "error", status, reason}` |
| `loadExtensions()` | `GET /api/extensions` | The installed extensions that declare a `[panel]` |
| `startPanel(name)`, `stopPanel(name)` | `POST /api/extensions/<name>/start` and `/stop` | A loopback port, or the reason it did not start |

## The proxy rules

`proxyPath(path, query)` builds the proxy URL and returns `null` for anything the server
would refuse, so the page fails early and in words:

- GET only, and only under `/v1/`.
- No `..`, `//`, `?`, `#` or `\` in the path; a query goes in the `query` argument and is
  percent-encoded.
- The API's own status comes back. A `401` becomes `signed-out`; any other failure keeps
  the API's error message when there is one, else `The API answered HTTP <n>.`

There is no write through the proxy. A page that needs one is a change to the server first
(see [contributing](contributing.md)).

## What Home reads

Home makes five reads as the signed-in person, and each stands alone, so one failure leaves
the others (`lib/home-data.ts`, refreshed every 30 seconds):

| Read | Path |
|---|---|
| Who you are | `/v1/accounts/me` |
| Reviews waiting | `/v1/decisions/reviews?status=waiting&account_id=<account>` |
| Spaces | `/v1/decisions/tree?account_id=<account>` |
| Monitors | `/v1/accounts/orgs/<account>/monitors` |
| Incidents | `/v1/accounts/orgs/<account>/incidents` |

A monitor's recent checks come from `.../monitors/<id>/runs`. Settings, Team reads
`.../members`. Home and Settings show what these answered, or a line that says a part is
not built here; nothing is filled in to look like data.

## Errors

Text the page can see passes through the same redaction as `iohr ask` output (values after
a secret's name, bearer values, JWTs, tokens and URLs are replaced), so an error the page
shows is safe to paste.

## Live updates

Home polls on a timer today. A live (streaming) client is in progress with the open UI
pull requests and is not documented as shipped here.
