# Sign in

How `iohr` signs you in, which account it acts for, how API tokens and `IOHR_TOKEN` work,
how profiles keep several sign-ins apart, and how other programs on your machine use your
login. Back to the [iohr overview](../README.md).

## Sign in as a person

```sh
iohr login
iohr whoami
```

`iohr login` signs you in on InOrbit's own sign-in pages:

- **With a browser** on the machine, it opens one and waits up to 5 minutes for it to come
  back to `http://127.0.0.1:<port>`.
- **Over SSH, in a container or without a display**, it prints a link and a code instead.
  Open the link in any browser, on any device, and enter the code. Only enter a code you
  started yourself.

`--web` and `--device` choose one. `IOHR_NO_BROWSER=1` (any non-empty value) makes every
command behave as if no browser can be opened and print its link instead; the test suites
set it.

`--scope S,...` asks for scopes besides the defaults, such as
`coordinator:runner,coordinator:read` for [`iohr runner`](runner.md). The sign-in service
refuses a scope the client may not ask for.

A signed-in session holds a 15-minute access token and a 30-day refresh token that is
replaced on every use; `iohr` refreshes on its own. `iohr logout` revokes the session at
the sign-in service before it removes it from this machine. The console's list of sign-in
devices shows the session and can end it from anywhere.

## Which account a sign-in acts for

The sign-in always starts on your own account. `iohr login` then picks the account the
profile acts for; the first match wins:

1. `--account <id|slug>`: a team you belong to;
2. the account this profile was already pointed at (with `--account` or
   `iohr profile account`), which is kept across every later sign-in;
3. your default account, the `default_account_id` preference
   (`iohr api PUT /v1/accounts/me/preferences/default -f account_id=<id>`);
4. your own account.

It prints the account it chose and how to change it, and `iohr profile show` says whether
the choice is kept. A default account you no longer belong to is skipped with a note.

```sh
iohr accounts list                 # the accounts this credential can see
iohr profile account default acme  # point the profile "default" at the team acme
```

## API tokens

An API token works too. Create one in the console (API tokens and keys) or with
`iohr token create`, then pipe it in. A token is never accepted as an argument, where shell
history and the process list would keep it.

```sh
iohr token create --name ci --scope decisions:read --days 30 > token.txt
iohr login --with-token --profile ci < token.txt
iohr token list --status active
iohr token revoke TOKEN_ID     # the id is in token list
```

`token create` needs a signed-in person. `--days` is 7, 30 (the default), 90 or 365, and
the new token is printed once, alone on stdout. `token list` takes `--status`
(`active`, `expiring`, `expired`, `revoked`), `--query` (a name or id prefix) and `--limit`
(default 100); `--account` works on all three. A revoked token is refused within seconds.

## `IOHR_TOKEN` for CI and one-off use

`IOHR_TOKEN` is used in memory and nothing is written. It wins over the default profile;
a profile named with `--profile` or `IOHR_PROFILE` wins over it, and a named profile reads
only `IOHR_TOKEN_<PROFILE>` from the environment.

```sh
IOHR_TOKEN="$(cat token.txt)" iohr api GET /v1/radar/digests -f page_size=5
```

## Profiles

A profile is one sign-in: a person or a token, for one account. Every command takes
`--profile NAME` (or `IOHR_PROFILE`).

```sh
iohr profile list          # the profiles on this machine and the default
iohr profile use work      # make "work" the default
iohr profile show          # the active profile; never shows a secret
iohr logout --profile ci   # forget a profile and its credential here
```

The credential is kept in the operating system's credential store. On a machine without
one, `iohr login --insecure-storage` keeps it in a file with mode 0600 instead. Where each
thing lives: [Files, network and dependencies](files-and-dependencies.md).

## Your login in other programs

`iohr auth token` prints the active profile's access token, refreshing a signed-in session
first when less than a minute of it is left. The SDKs' `cli` credential source runs it
([docs/config.md](../../docs/config.md) section 5.4), so `iohr login` is enough for a
program on your machine to call the API as you:

```sh
iohr auth token --profile work --format json
# {"access_token":"<token>","expires_at":"2026-10-04T10:15:00Z","profile":"work","account":"<account id>"}
```

- `expires_at` is RFC 3339 in UTC, or `null` for a token without an expiry.
- `profile` is `null` when the token came from `IOHR_TOKEN`.
- Without `--format json` the token is printed alone on one line.
- The refresh token never leaves the credential store.
- Exit 3 when there is no such profile, no credential, or the session has ended
  (`iohr login` again); exit 1 when the sign-in service cannot be reached.
