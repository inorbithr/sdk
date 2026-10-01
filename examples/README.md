# Examples

Small, complete programs, one directory per language. CI compiles every example
(`mise run examples:check`) and the package READMEs quote them, so an example that no
longer builds fails the PR.

Planned, in each language:

| Example | Shows |
|---|---|
| `whoami` | Build a client from `INORBIT_KEY_ID` and `INORBIT_KEY_SECRET`, call `me` |
| `usage` | Read the last 30 days of usage for the key's account |
| `errors` | Match on error codes, read `details`, respect `retry` |
| `custom-token` | Plug in a `token_provider` |

Running one needs a key from <https://console.inorbit.hr/keys/>. Examples never print the
secret or the token.
