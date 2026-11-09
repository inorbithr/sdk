# Domains and connections

How to prove your account controls a domain, and how to connect outside apps to your
account and grant their actions to products, keys and agents. Back to the
[iohr overview](../README.md).

## Domains

```sh
iohr domains add acme.hr                 # prints the TXT record to add
iohr domains verify acme.hr --wait       # looks it up from several resolvers until seen
iohr domains confirm acme.hr             # marks it verified for the account
iohr domains list
iohr domains rm acme.hr
```

One DNS TXT record proves the account controls a domain.

- `--subdomain` on `add` proves only a subdomain's subtree (`staging.acme.hr`), not its
  parent.
- `verify --wait` checks every 10 seconds for up to `--timeout` seconds (default 600).
- The token expires after 7 days unless the domain is verified, and a verified domain is
  checked again every day.
- The calls need the `domains:read` and `domains:write` scopes.

## Connections

```sh
iohr connectors list --category incident        # what can be connected
iohr connectors show pagerduty                  # its modes, fields, settings and actions
iohr connections add incident-io                # asks for the API key without echo
iohr connections add pagerduty --mode api_key --config region=eu.pagerduty.com \
  --secret-file api_key=./pd.key                # or --secret-stdin api_key < pd.key
iohr connections add slack                      # opens Slack in a browser, waits until done
iohr connections grant incident-io --to product:reliability --actions create_incident --expires 90d
iohr connections history incident-io --status failed
iohr connections delete incident-io             # asks first; --yes in a script
```

`iohr connectors list [--category C]` and `show ID` read the catalogue of apps a connection
can be made from: their sign-in modes and fields, settings, actions, and the hosts each may
call. A connector for an AI model is labelled as one.

`iohr connections` works on the account's connections: `list`, `show`, `add`, `test`,
`history`, `pause`, `resume`, `rename`, `delete` (asks first unless `--yes`) and
`reconnect`, plus `grant`, `grants` and `revoke-grant`.

### Connecting with a key

A secret field (an API key, a bot token, a password) is never an argument. `iohr` asks for
it without echo on a terminal, or reads it from `--secret-file FIELD=PATH` or
`--secret-stdin FIELD`. It is held in memory that is wiped when dropped, sent once in the
body of the call that makes or reconnects the connection, and never printed, logged or
written anywhere. Settings that are not secret go in `--config KEY=VALUE`.

The platform tests the key before it stores anything. A refused key exits 1 with the reason
and nothing is kept; a passing one prints who the account is at the provider
("Connected incident-io as ... on incident.io").

### Connecting by signing in

A mode that signs in (OAuth) starts a connect session on the platform and opens the
provider's page in your browser, or prints the link over SSH and without a display. You
finish on the provider's page and on the console page it returns to. `iohr` asks the
session every 2 seconds until it completes, fails or expires (10 minutes), then prints the
connection. `iohr connections reconnect NAME` gives a connection a new credential the same
way.

### Grants

`iohr connections grant` lets a product, an API key or an agent use named actions of a
connection until an expiry; `grants` lists them and `revoke-grant` ends one.

### Who may do what

| Action | Needs |
|---|---|
| Connect, reconnect, delete | An owner or admin of the account, and the `connections:write` scope |
| List and show | The `connections:read` scope |
| Grant | A signed-in person, not a token |

An AI model's connector says so: what you send in prompts goes to that provider under your
own agreement with them.
