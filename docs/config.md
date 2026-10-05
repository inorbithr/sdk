# Configuration, credentials and the middleware pipeline

The normative contract for how a client is configured, where it finds credentials and how
a request travels through it, in all six languages. The decisions and the research
behind them are in [ADR 0015](adr/0015-configuration-and-middleware.md). Ready-made
setups for common environments are in [recipes.md](recipes.md). Where this page and
`design.md` disagree on configuration, this page wins and `design.md` is corrected in the
same pull request.

Status: designed 2026-10-04, milestone M6 in [roadmap.md](roadmap.md). Today the six
runtimes implement section 2 of `design.md` only: explicit options and `from_env`.
Everything below lands one language at a time. The conformance cases and vectors are
written now and marked `pending` until each runtime passes them.

Words used here:

- A **setting** is one configurable value (`timeout`, `base_url`).
- A **source** is where a setting's value comes from: code, the environment, the config
  file, or the default.
- A **profile** is a named table in the config file and, for a generated surface, a type
  (ADR 0011). The two are the same name.
- A **credential source** is a place the client may find credentials. The ordered list of
  them is the **credential chain**.
- A **middleware** is one named step a request passes through. The ordered list of them
  is the **pipeline**.

## 1. Building a client

Every runtime has three ways to build a client. The first two exist today and keep
working unchanged.

| Way | Reads | Use |
|---|---|---|
| Explicit options (`Client::builder()...build()`, `new Client({...})`, `Client(...)`, `NewClient(...)`, `Client.builder()...build()`, `new Client<P>(new ClientOptions{...})`) | code only | Libraries that must not pick up ambient configuration, tests |
| `from_env` | code (where the language takes overrides today) and the environment, credentials only | Existing code; kept, soft-deprecated in favour of `load` |
| **`load`** (new) | code, the environment, the config file, the `iohr` login, defaults | Applications. The default answer in every README |

`load` takes the same options as explicit construction. An option set in code always
wins. The names in each language:

| Language | Load | With overrides |
|---|---|---|
| Rust | `Client::<P>::load()?` | `Client::<P>::builder().timeout(..).load()?` |
| TypeScript | `Client.load()` | `Client.load({ timeout: 5_000 })` |
| Python | `Client.load()`, `AsyncClient.load()` | `Client.load(timeout=5.0)` |
| Go | `inorbit.Load()` | `inorbit.Load(inorbit.WithTimeout(5 * time.Second))` |
| Java | `Client.load()` | `Client.builder().timeout(Duration.ofSeconds(5)).load()` |
| C# | `Client.Load()`, `Client.Load<TProfile>()` | `Client.Load(new ClientOptions { Timeout = TimeSpan.FromSeconds(5) })` |

A generated surface gets the same entry point: `Public::load()`, `Public.load()`,
`public.Load()` and so on, next to its `from_env`.

`load` does its I/O at construction: it reads the environment, the config file and any
files the settings name, and it checks that `iohr` exists when the chain reaches it.
It contacts no host. The first token is fetched on the first call, as today.

`from_env` keeps today's exact behaviour: credentials and URLs from `INORBIT_[<P>_]*`,
nothing else. Its doc comment points to `load`. A compiler-level deprecation waits for a
minor release before 1.0 and is announced in the changelog first.

## 2. Resolution

### 2.1 Precedence

Each setting is resolved on its own. The first source that sets it wins:

| Rank | Source | Notes |
|---|---|---|
| 1 | Code | An option passed to `load` or the builder |
| 2 | Environment | `INORBIT_<P>_<NAME>` for a typed profile, then `INORBIT_<NAME>` (section 2.3) |
| 3 | Config file | The selected profile's table, then the file's `[sdk]` table |
| 4 | Default | The catalogue in section 3 |

Rules every runtime applies:

- **An empty environment variable is unset.** `INORBIT_TIMEOUT=` behaves as if the
  variable did not exist. Kubernetes and CI templates produce empty values often.
- **Values never merge across sources.** `scopes` from code replaces `scopes` from the
  file; it is not a union.
- **Credentials are chosen whole.** The pieces of one credential (an id and its secret, or
  a token) come from one source, never an id from the file and a secret from the
  environment (section 5). `scopes` is a setting, not a credential piece, and resolves
  normally.
- **The `iohr` keychain is a credential source, not a settings source.** It never sets a
  timeout or a URL. It sits at the end of the credential chain.

### 2.2 Choosing the profile

The **public client** (the published package's `Public` profile, and every client built
without a generated profile type) chooses its file profile in this order:

1. `profile` in code;
2. `INORBIT_PROFILE`;
3. `default` at the top of the config file (the profile `iohr login` and
   `iohr profile use` set);
4. none: only the file's `[sdk]` table applies, and the `cli` credential source is
   skipped.

A profile chosen by code or `INORBIT_PROFILE` that the file does not have is a
`ConfigError`. The file's own `default` naming a missing profile is a `ConfigError` too.

A **typed profile** (`Client<AcmeCi>`, `AcmeCi.load()`) is its own profile: the file
table `[profiles.acme-ci]`, the environment prefix `INORBIT_ACME_CI_`. Neither code nor
`INORBIT_PROFILE` can point it elsewhere: ADR 0011's rule that a named profile calls its
own account or nothing holds.

`IOHR_PROFILE`, the command line's switch, is not read by the SDK. One variable per tool
keeps a shell set up for the command line from silently changing what a service calls.

### 2.3 Environment names

- Profile names follow the command line's rule: 1 to 64 characters from `a-z`, `0-9`, `_`
  and `-`, starting with a letter or digit. The environment form is upper case with `-`
  turned into `_`: `acme-ci` reads `INORBIT_ACME_CI_*`. Every runtime normalises the same
  way. Rust does this today; the other five use the name verbatim, which is fixed in M6.
- A **typed profile** reads each setting as `INORBIT_<P>_<NAME>` first and `INORBIT_<NAME>`
  second, **except credentials** (`key_id`, `key_secret`, `key_secret_file`, `token`,
  `token_file`), which are read only with the prefix. This keeps today's fallback for
  `base_url` and `token_url` and the rule that credentials never fall back.
- The **public client** reads `INORBIT_<NAME>` only. Credentials in the bare environment
  belong to the public client, whatever `INORBIT_PROFILE` selects. This is the same as
  AWS, where `AWS_ACCESS_KEY_ID` wins over `AWS_PROFILE`. `credential_sources` narrows it
  when that is not wanted (section 5).
- Standard variables that are not ours keep their own names: `HTTPS_PROXY`, `NO_PROXY`
  (section 6.2), `XDG_CONFIG_HOME`, `APPDATA` and `IOHR_CONFIG_DIR` (section 4.1).

### 2.4 Value syntax

| Type | Environment | Config file | Code |
|---|---|---|---|
| Duration | `500ms`, `30s`, `2m`, `1h`: digits, then `ms`, `s`, `m` or `h`. No bare numbers, no fractions, no spaces | the same, as a string | the language's duration type (`Duration`, `time.Duration`, `TimeSpan`, `timedelta` or float seconds in Python, milliseconds in TypeScript as today) |
| Integer | decimal digits | integer | integer |
| Boolean | `true`, `false`, `1`, `0`, any case | boolean | boolean |
| List | `scopes`: separated by spaces (as today). Every other list: separated by commas, spaces around items trimmed | array of strings | the language's list |
| URL | absolute URL | string | string (C# `Uri`) |
| Path | absolute, or relative to the working directory | absolute, `~/`-prefixed, or relative **to the config file's directory** | string or the language's path type |

A duration must be greater than zero unless the catalogue says otherwise. `max_retries`
may be `0`. Every timeout is finite: there is no "never" value (SR-19).

### 2.5 Errors

A bad value fails `load` with one `ConfigError` that lists **every** problem found, in
catalogue order, each naming the setting, the source and what to do:

```
configuration is invalid (2 problems):
  timeout: "30" is not a duration; write it with a unit, such as 30s (from env INORBIT_TIMEOUT)
  key_secret: secrets are not allowed in the config file; use key_secret_file, the
    environment, or iohr login (from file /home/ana/.config/iohr/config.toml [profiles.work])
```

The error exposes the list as data (`problems`: setting, source, message) so tests and
tools can read it without parsing text. Source labels are fixed strings:

| Source | Label |
|---|---|
| Code | `code` |
| Environment | `env INORBIT_TIMEOUT` (the variable actually read) |
| Config file, profile table | `file <path> [profiles.<name>]` |
| Config file, `[sdk]` table | `file <path> [sdk]` |
| Config file, top level | `file <path>` |
| Default | `default` |

A message never contains a secret's value. A bad secret value is named, never echoed.

### 2.6 Inspecting the effective configuration

Every client and every `load` result can describe itself, the SDK's equivalent of
`chaos config`: `client.config()` returns a `ResolvedConfig` whose `describe()` (`Describe`,
`toJSON`) gives this JSON document. It is stable within a major version:

```json
{
  "profile": { "name": "work", "source": "env INORBIT_PROFILE" },
  "config_file": "/home/ana/.config/iohr/config.toml",
  "settings": {
    "base_url":    { "value": "https://api.inorbit.hr", "source": "default" },
    "timeout":     { "value": "10s", "source": "env INORBIT_TIMEOUT" },
    "key_id":      { "value": "ak_7f3c", "source": "file /home/ana/.config/iohr/config.toml [profiles.work]" },
    "key_secret_file": { "value": "/run/secrets/inorbit", "source": "file /home/ana/.config/iohr/config.toml [profiles.work]" },
    "scopes":      { "value": ["identity:read", "radar:read"], "source": "file /home/ana/.config/iohr/config.toml [profiles.work]" },
    "proxy":       { "value": "http://<redacted>@proxy.corp:3128", "source": "env HTTPS_PROXY" }
  },
  "credential": {
    "source": "file",
    "kind": "client_credentials",
    "tried": [
      { "source": "code", "result": "skipped", "reason": "none set" },
      { "source": "env", "result": "skipped", "reason": "INORBIT_TOKEN, INORBIT_TOKEN_FILE and INORBIT_KEY_ID are not set" },
      { "source": "workload", "result": "skipped", "reason": "not offered by the platform yet" },
      { "source": "file", "result": "used" }
    ]
  },
  "pipeline": ["request_id", "user_agent", "idempotency_key", "call_tracing", "deadline", "retry", "auth", "rate_limit", "attempt_tracing", "logging", "hooks", "timeout"],
  "ignored": [
    { "key": "colour", "source": "file /home/ana/.config/iohr/config.toml [profiles.work]", "reason": "unknown key" }
  ]
}
```

- Only settings with a value appear, defaults included. A secret's value is always
  `<redacted>`; the key id is not secret and is shown. A URL's user-info is redacted.
- `ignored` lists what was read and not used: unknown file keys, settings a caller-supplied
  HTTP client makes meaningless (section 6.6), scopes a `cli` or token credential does not
  use.
- The command line prints the same document with `iohr sdk config [--profile NAME]
  [--for TYPE]` (iohr 0.1.0-alpha.8 and later), so "what will my service see" has a
  one-line answer. `--profile` stands for `profile` in code (source `code`), `--for`
  resolves for a typed profile, and `IOHR_PROFILE` is not read. A configuration `load`
  would refuse exits 1 with the `ConfigError`'s text on stderr. Until the Rust runtime
  has `load`, the command line resolves with its own implementation, held to the
  `config`, `config-path` and `durations` vectors; it then calls the runtime.
- When no source has credentials, the `ConfigError` holds one problem with the setting
  `credential` and an empty source, whose message is the chain's (section 5.1). When it
  is the only problem, the error's text is that message alone.

For tests, `load` accepts its environment and file system as inputs (section 9.2), so a
test can resolve a configuration without touching the process environment.

## 3. Settings catalogue

Scope: **profile** settings belong to one account and are credential or endpoint
settings; **process** settings are about how this program talks to the network. Both
follow section 2.3. "File" says whether the key may appear in the config file.

### 3.1 Endpoint and profile

| Setting | Environment | File | Type | Default | Meaning |
|---|---|---|---|---|---|
| `profile` | `INORBIT_PROFILE` | top-level `default` | name | none | The file profile (section 2.2); public client only |
| `config_file` | `INORBIT_CONFIG_FILE` | no | path or `off` | the OS location (section 4.1) | Which file to read; `off` reads none |
| `base_url` | `INORBIT_BASE_URL` | yes | URL | `https://api.inorbit.hr` | The API origin. `https://`, or `http://` on loopback only (SR-07) |
| `token_url` | `INORBIT_TOKEN_URL` | yes | URL | `https://auth.inorbit.hr/oauth2/token` | The token endpoint; same rule |
| `region` | `INORBIT_REGION` (reserved) | reserved | `eu`, `us` | none | Reserved until the API offers regions (SR-09); setting it today is a `ConfigError` |

### 3.2 Credentials

| Setting | Environment | File | Type | Default | Meaning |
|---|---|---|---|---|---|
| `key_id` | `INORBIT_KEY_ID` | yes | string | none | An API key's id (`ak_...`), the OAuth client id |
| `key_secret` | `INORBIT_KEY_SECRET` | **never** | secret | none | The key's secret |
| `key_secret_file` | `INORBIT_KEY_SECRET_FILE` | yes | path | none | A file holding the secret, read before every token exchange (rotation) |
| `scopes` | `INORBIT_SCOPES` | yes | list | none | Scopes to ask for with a key; required with a key |
| `token` | `INORBIT_TOKEN` | **never** | secret | none | A bearer token used as is (an API token, RFC 0016) |
| `token_file` | `INORBIT_TOKEN_FILE` | yes | path | none | A file holding a bearer token, re-read when it changes |
| `credential_sources` | `INORBIT_CREDENTIAL_SOURCES` | yes | list of `env`, `workload`, `file`, `cli` | all four | Which credential sources the chain may use (section 5.1). Code is always allowed |
| `cli_path` | `INORBIT_CLI_PATH` | yes | path | `iohr` on `PATH` | The command line the `cli` source runs |

### 3.3 Time and retries

| Setting | Environment | File | Type | Default | Meaning |
|---|---|---|---|---|---|
| `connect_timeout` | `INORBIT_CONNECT_TIMEOUT` | yes | duration | 10 s | DNS, TCP and TLS, per connection |
| `timeout` | `INORBIT_TIMEOUT` | yes | duration | 30 s | One attempt: from sending to the last byte of the answer |
| `total_timeout` | `INORBIT_TOTAL_TIMEOUT` | yes | duration | 120 s | One call, every attempt and every wait included. A caller's own deadline that is shorter wins |
| `stream_idle_timeout` | `INORBIT_STREAM_IDLE_TIMEOUT` | yes | duration | 45 s | A stream silent this long ends (design.md section 7) |
| `max_retries` | `INORBIT_MAX_RETRIES` | yes | integer, 0 or more | 2 | Retries, not attempts; `0` disables |
| `retry_base_delay` | `INORBIT_RETRY_BASE_DELAY` | yes | duration | 500 ms | Exponential backoff base, full jitter |
| `retry_max_delay` | `INORBIT_RETRY_MAX_DELAY` | yes | duration | 8 s | Backoff cap |
| `retry_after_max` | `INORBIT_RETRY_AFTER_MAX` | yes | duration | 60 s | The longest `Retry-After` the client waits; a longer one ends the call with the error |
| `retry_budget` | `INORBIT_RETRY_BUDGET` | yes | boolean | `true` | The per-client retry quota (section 7.4) |
| `streams` | `INORBIT_STREAMS` | yes | `sse`, `socket` | `sse` | How streams open (design.md section 7) |

### 3.4 Network

| Setting | Environment | File | Type | Default | Meaning |
|---|---|---|---|---|---|
| `proxy` | `INORBIT_PROXY`, then `https_proxy`, then `HTTPS_PROXY` | yes, without user-info | URL or `off` | none | Section 6.2. User-info in the URL is a secret: code and environment only |
| `no_proxy` | `INORBIT_NO_PROXY`, then `no_proxy`, then `NO_PROXY` | yes | list | none | Section 6.2 |
| `ca_bundle` | `INORBIT_CA_BUNDLE` | yes | path | none | PEM certificates **added** to the trust store (SR-03) |
| `system_trust` | `INORBIT_SYSTEM_TRUST` | yes | boolean | `true` | `false` trusts `ca_bundle` only, for a private gateway |
| `client_cert` | `INORBIT_CLIENT_CERT` | yes | path | none | PEM client certificate chain for mTLS (SR-04) |
| `client_key` | `INORBIT_CLIENT_KEY` | yes | path | none | PEM private key for `client_cert` |
| `client_key_password` | `INORBIT_CLIENT_KEY_PASSWORD` | **never** | secret | none | For an encrypted key |
| `pinned_keys` | `INORBIT_PINNED_KEYS` | yes | list of base64 SPKI SHA-256 | none | Opt-in pinning, at least two pins (SR-06) |

### 3.5 Observability and behaviour

| Setting | Environment | File | Type | Default | Meaning |
|---|---|---|---|---|---|
| `log` | `INORBIT_LOG` | yes | `off`, `error`, `warn`, `info`, `debug` | `off` | Section 7.9 |
| `log_headers` | `INORBIT_LOG_HEADERS` | yes | boolean | `false` | Log allowlisted header values at `debug` |
| `log_allow_headers` | `INORBIT_LOG_ALLOW_HEADERS` | yes | list | none | Header names added to the allowlist; the never-log list still applies |
| `tracing` | `INORBIT_TRACING` | yes | boolean | `true` when an OpenTelemetry integration is installed | Section 7.10 |
| `metrics` | `INORBIT_METRICS` | yes | boolean | same as `tracing` | Section 7.10 |
| `rate_limit` | `INORBIT_RATE_LIMIT` | yes | `observe`, `wait`, `off` | `observe` | Section 7.8 |
| `user_agent_suffix` | `INORBIT_USER_AGENT_SUFFIX` | yes | string | none | Appended to the user agent: RFC 9110 product tokens, at most 128 characters |

Code-only options (no environment variable, no file key, because they hold objects):
`token_provider` (a credential), `http_client`, `pipeline`, `hooks`, `logger`, `redact`,
`tracer_provider` and `meter_provider`, `executor` (Java), and per-call options
(`timeout`, cancellation, `idempotency_key`, `traceparent`).

## 4. The config file

### 4.1 Location

The SDK reads the command line's file, so `iohr login` is enough for a working client.
The first rule that applies wins:

1. `config_file` in code (`off` reads no file);
2. `INORBIT_CONFIG_FILE` (`off` reads no file);
3. `$IOHR_CONFIG_DIR/config.toml`, the command line's own override;
4. the command line's default directory (its `etcetera` native strategy, `cli/README.md`):

| OS | File |
|---|---|
| Linux and other Unix | `$XDG_CONFIG_HOME/iohr/config.toml`, or `~/.config/iohr/config.toml` when `XDG_CONFIG_HOME` is unset or not absolute |
| macOS | `~/Library/Application Support/hr.InOrbit.iohr/config.toml` |
| Windows | `%APPDATA%\InOrbit\iohr\config\config.toml` |

- A file named by rule 1 or 2 that does not exist is a `ConfigError`. A file at rule 3 or
  4 that does not exist is normal: there is no file layer.
- No home directory (a serverless sandbox, a distroless container) is the same as no file.
- Where the runtime has no file system (browsers, edge workers), the file layer and the
  `cli` source are skipped and `describe()` says so.
- The file is read once, at `load`. At most 1 MiB; larger is a `ConfigError`.

### 4.2 Format

TOML 1.0, the command line's format. One file holds the command line's keys and the
SDK's:

```toml
default = "work"                    # set by iohr login / iohr profile use

[sdk]                               # every profile, below its own table
log = "warn"
proxy = "http://proxy.corp.example:3128"
no_proxy = "localhost,.corp.example"
ca_bundle = "~/certs/corp-root.pem"

[profiles.work]                     # written by iohr login
kind = "person"
account = "acc_8d2e"
storage = "keyring"

[profiles.ci]                       # written by hand or by configuration management
key_id = "ak_7f3c"
key_secret_file = "/run/secrets/inorbit-ci"
scopes = ["identity:read", "radar:read"]
timeout = "10s"
max_retries = 4
```

Rules:

- The file layer for a profile is its `[profiles.<name>]` table over the `[sdk]` table.
  With no profile chosen, only `[sdk]` applies.
- The command line's keys (`kind`, `account`, `storage`, `issuer`, `client_id`, and the
  top-level `ext`) are read by the `cli` credential source and otherwise left alone.
- **Secrets are never in the file.** `key_secret`, `token`, `client_key_password` and a
  `proxy` URL with user-info are a `ConfigError` naming the file and the table, with the
  way out: `key_secret_file`, `token_file`, the environment, or `iohr login`. A secret
  that sits in a file the SDK reads, and that dotfile backups and support bundles copy,
  is the leak SR-24 exists to prevent.
- Unknown keys are ignored and listed in `describe()`'s `ignored`, so a newer command
  line or SDK can add keys without breaking an older reader, and a typo is still visible.
- A value of the wrong type is a `ConfigError`, the same as a bad environment value.

The command line keeps what it does not own. It changes only `default`, its own profile
keys (`kind`, `account`, `storage`, `issuer`, `client_id`) and the `[ext]` table, and keeps
every other key, table and comment (`toml_edit`). A profile without `kind` holds SDK keys
only: `iohr` does not list it as one of its profiles. `iohr logout` removes the profile's
command-line keys and leaves its SDK keys; the table goes when nothing is left.

## 5. Credentials

### 5.1 The chain

`load` looks for credentials in this order and uses the first source that has any. The
order mirrors the precedence of section 2.1: code, then the environment, then files, then
the developer's login.

| # | Source | Name | Present when | Credential |
|---|---|---|---|---|
| 1 | Code | `code` | `token_provider`, `token` or `key`/`key_id` set in code | As given |
| 2 | Environment | `env` | `INORBIT_[<P>_]TOKEN`, `_TOKEN_FILE` or `_KEY_ID` is set | Static token, token file, or client credentials with `_KEY_SECRET` or `_KEY_SECRET_FILE` |
| 3 | Workload identity | `workload` | `INORBIT_[<P>_]WEB_IDENTITY_TOKEN_FILE`, or `ACTIONS_ID_TOKEN_REQUEST_URL` with `INORBIT_[<P>_]FEDERATED_KEY_ID` | **Reserved.** The platform cannot exchange an outside identity token yet (section 5.5). The source is skipped with "not offered by the platform yet" |
| 4 | Config file | `file` | the profile's table sets `token_file` or `key_id` | Token file, or client credentials with `key_secret_file` |
| 5 | The `iohr` login | `cli` | the chosen profile exists in the file with a `kind` (the command line made it) | A token from `iohr auth token` (section 5.4) |

When no source has credentials, `load` fails with a `ConfigError` that lists each source
and why it was skipped, and ends with what to do:

```
no credentials found for profile "default"; tried:
  env: INORBIT_TOKEN, INORBIT_TOKEN_FILE and INORBIT_KEY_ID are not set
  workload: not offered by the platform yet
  file: no config file at /home/ana/.config/iohr/config.toml
  cli: skipped, no profile chosen
Set INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES, or run `iohr login`.
```

Rules:

- **A source with two kinds of credential is an error, not a choice.** `INORBIT_TOKEN`
  and `INORBIT_KEY_ID` set together, or `INORBIT_KEY_SECRET` and
  `INORBIT_KEY_SECRET_FILE` together, is a `ConfigError`. A source with half a credential
  (`INORBIT_KEY_ID` without a secret, a key without `scopes`) is a `ConfigError` too; the
  chain does not move on to the next source, because a half-set source is a mistake to
  report, not an absence.
- **`credential_sources` narrows the chain.** `INORBIT_CREDENTIAL_SOURCES=env` in
  production means a developer's login on a shared host is never picked up; `cli` only
  on a laptop. The order is fixed; the setting filters it. An unknown name is a
  `ConfigError`. Code is always allowed. This is Azure's `AZURE_TOKEN_CREDENTIALS` and
  Microsoft's advice to pin the credential in production.
- **The decision is made at `load`.** Which source is used, and the credential's shape,
  is decided once. A file named by `token_file` or `key_secret_file` must exist and be
  readable at `load`; its contents are read again later (section 5.3).
- `scopes` applies to client credentials only. A token, a token file and the `cli`
  source carry their own scopes; a `scopes` setting next to them is listed in `ignored`.

### 5.2 Custom providers

The `TokenProvider` interface of `design.md` section 3 stays the extension point: one
"give me a valid token" call, and an optional "this token was refused" call. Any
provider can be placed in code (rank 1). The built-in sources are public types, so a
program can build its own chain:

| Type | Purpose |
|---|---|
| `ClientCredentials` | Key id, secret (or secret file), scopes, token URL |
| `StaticToken` | A token used as is |
| `TokenFile` | A token read from a file and re-read on change |
| `CliToken` | The `iohr auth token` source |
| `CachedToken` | Wraps any provider with the caching rules of section 5.3 |
| `ChainedCredential` | Tries providers in order; `DefaultCredential` is the chain of section 5.1 |

A provider that fetches a token from a vault or a secrets manager is a `TokenProvider`
wrapped in `CachedToken`. The SDK does not run commands from the config file (AWS's
`credential_process`); ADR 0015 explains why.

### 5.3 Caching, refresh and rotation

The rules every built-in provider follows; `CachedToken` gives them to custom ones:

- A token is cached in memory only (SR-12).
- **Refresh ahead**: a token is refreshed when less than 20 % of its lifetime is left,
  the lifetime being `expires_in` at the exchange. With the platform's 15-minute tokens
  that is three minutes early.
- **Single flight**: one refresh at a time per provider. Concurrent callers wait for the
  same result; a caller that gives up does not cancel the refresh for the others.
- **Soft expiry**: when a refresh fails and the cached token is still valid, the call
  uses it, a `warn` record is logged, and the next refresh is tried no sooner than 5 s
  later. When the token has expired and the refresh fails, the call fails with
  `AuthError`.
- **A 401 from the API** invalidates the cached token and the request is sent once more
  with a fresh one (`design.md` section 3). That extra attempt is not a retry and draws
  nothing from the retry budget.
- **Rotation without restart**:
  - `key_secret_file` is read before every token exchange, so a rotated Kubernetes Secret
    or Vault agent file is used at the next refresh.
  - `token_file` is read at first use, then again when its modification time or size
    changes (checked at most once every 60 s), and immediately after a 401. When the
    content is a JWT, its `exp` claim (read, not verified) is the token's expiry;
    otherwise the token has none.
  - A file that disappears or becomes unreadable keeps the cached token until it is
    refused, then fails with `AuthError` naming the file.
- **Static tokens** (`INORBIT_TOKEN`) are never refreshed. A 401 fails the call with an
  `AuthError` that says the token was refused and how to get a new one.

### 5.4 The `iohr` login

The `cli` source runs the command line rather than reading the keychain itself:

```
<cli_path> auth token --profile <name> --format json
→ {"access_token": "...", "expires_at": "2026-10-04T10:15:00Z", "profile": "work", "account": "acc_8d2e"}
```

- It runs without a shell, with standard input closed, a 10 s limit and the inherited
  environment. A non-zero exit is an `AuthError` carrying the first line of standard
  error (at most 200 characters); standard output is never logged.
- The token is cached by section 5.3's rules; the command runs again in the refresh
  window, which with 15-minute tokens is about four times an hour.
- `iohr auth token` (iohr 0.1.0-alpha.8 and later) refreshes the stored session with the
  command line's own rules (re-reading the store, rotation, SR-24) when less than 60 s of
  the access token is left, and prints the access token only, never the refresh token.
- Its output is one line of JSON on standard output, with exactly these fields:
  `access_token`; `expires_at`, RFC 3339 in UTC, or `null` for a token without an
  expiry (an API token may have none); `profile`, the profile's name; `account`, the
  account id the profile acts for. Standard error is empty on success.
- Exit codes are the command line's: `0` printed; `2` a usage error (a malformed profile
  name); `3` not signed in (no such profile, no credential stored, an expired API token,
  or a session the sign-in service ended: `iohr login` again); `1` anything else, such as
  the sign-in service being unreachable. On a non-zero exit standard output is empty and
  standard error carries one line saying what to do.
- `--profile` is always passed. Without it, `iohr` uses `IOHR_PROFILE`, then
  `IOHR_TOKEN` (and prints `"profile": null`), then its default profile; the SDK never
  relies on that.

Running the command line, as Azure's `AzureCliCredential` runs `az`, keeps the keychain,
refresh-token rotation and the session file format in one program instead of six. The
cost is a process start per refresh and a dependency on `iohr` being installed. ADR 0015
weighs the alternative.

### 5.5 What the platform accepts today

| Credential | Platform today | SDK |
|---|---|---|
| API key: client credentials (`ak_...` + secret, Basic auth, `audience=iohr-api`) | yes | `env`, `file`, code |
| API token (RFC 0016, a long-lived JWT) | yes | `env` (`TOKEN`, `TOKEN_FILE`), `file` (`token_file`), code |
| Person login (authorization code + PKCE or device code, client `iohr-cli`) | yes | `cli` |
| Workload identity: a Kubernetes service account token or a GitHub Actions OIDC token exchanged for an InOrbit token | **no**: Hydra has no token exchange (RFC 8693), and its RFC 7523 trust relationships hold static keys, not an issuer's rotating key set | `workload` reserved; platform follow-up 1 |
| mTLS-bound tokens (RFC 8705), DPoP | no | not designed |

Until workload identity exists, a Kubernetes workload uses a key whose secret is mounted
from a Kubernetes Secret (`INORBIT_KEY_SECRET_FILE`), and GitHub Actions uses an API
token or a key in a repository secret. [recipes.md](recipes.md) shows both.

## 6. Transport

### 6.1 Timeouts

| Timeout | Covers | Default |
|---|---|---|
| `connect_timeout` | DNS, TCP and the TLS handshake, per new connection | 10 s |
| `timeout` | one attempt: request sent, headers and the whole body read | 30 s |
| `total_timeout` | one call: every attempt, every backoff and every rate-limit wait | 120 s |
| `stream_idle_timeout` | a stream with no event, comment or ping | 45 s |
| a stream's opening | connect, then headers, by `timeout` | 30 s |
| the `cli` source | the `iohr` process | 10 s |

- A per-call timeout or cancellation replaces `timeout` and shortens `total_timeout` for
  that call; it never extends `total_timeout`.
- A retry whose delay would pass `total_timeout` is not made: the call ends with the
  last error.
- In M6 the attempt timeout covers the body in every language. Today Java's covers only
  the headers and Python's applies per read.

### 6.2 Proxies

- **Which proxy.** `proxy` from code, then `INORBIT_PROXY`, then the file, then
  `https_proxy`, then `HTTPS_PROXY`. The lower-case standard variable wins over the
  upper-case one, as in curl and, since 2026-07, Go's `httpproxy`.
  `HTTP_PROXY` and `ALL_PROXY` are not read: every request is HTTPS except to loopback.
  `off` (in code, `INORBIT_PROXY` or the file) turns every proxy off, the standard
  variables included.
- **Scheme.** `http://` (CONNECT over plain TCP) and `https://` (CONNECT over TLS to the
  proxy) where the runtime supports it. SOCKS is out of scope; it is reachable through a
  caller-supplied HTTP client.
- **Proxy authentication.** User-info in the proxy URL is sent as Basic
  `Proxy-Authorization` on CONNECT. It is a secret (SR-10): redacted in `describe()`,
  never in a log, refused in the file.
- **Loopback.** `localhost`, `127.0.0.0/8` and `::1` never use a proxy that came from
  the standard variables. A proxy set explicitly (code, `INORBIT_PROXY`, the file)
  applies to loopback too, so it can be tested.
- **`no_proxy` grammar.** It is the same in every runtime, and is implemented by the SDK,
  not left to the language's library. The libraries disagree (the GitLab survey in ADR
  0015). Comma separated, whitespace trimmed, case-insensitive:
  - `*` alone: no proxy for any host;
  - `example.com` or `.example.com`: the host itself and every subdomain (a leading dot
    changes nothing, as in curl and Python);
  - `host:port`: only that port;
  - an IPv4 or IPv6 literal (`10.1.2.3`, `[::1]`, `::1`): that address;
  - CIDR (`10.0.0.0/8`, `fd00::/8`): hosts written as an IP literal in that range.
    Names are never resolved to check a range;
  - empty entries are skipped; an entry that fits none of these is a `ConfigError`.

  `INORBIT_NO_PROXY` wins over `no_proxy`, which wins over `NO_PROXY`. The vectors in
  `conformance/vectors/no-proxy/` are the contract.
- Streams and the `/v1/ws` socket use the same proxy as every other request. Today the
  Rust, Python and C# sockets ignore it; that is fixed in M6.

### 6.3 Trust

- The system trust store is the default in every runtime (Rust: `rustls-platform-verifier`).
- `ca_bundle` **adds** the PEM certificates in the file to it. A TLS-inspecting proxy's
  root needs this, and keeping the public roots means the public API still verifies. This
  differs from `AWS_CA_BUNDLE`, which replaces the bundle.
- `system_trust = false` with a `ca_bundle` trusts only the bundle: a private gateway
  with its own CA. `system_trust = false` without a bundle is a `ConfigError`.
- `SSL_CERT_FILE` and `SSL_CERT_DIR` are not read by the SDK. Some runtimes read them
  themselves (OpenSSL, Go); recipes say which.
- Certificate verification cannot be turned off (SR-02).

### 6.4 Client certificates (mTLS)

- `client_cert` and `client_key` are PEM files; `client_key_password` decrypts an
  encrypted PKCS#8 key. Java and C# also accept PKCS#12 in code
  (`clientCertificate(KeyStore)`, `ClientCertificates`), and a signer hook where the TLS
  stack has one (SR-04), so the key can stay in an HSM or the OS store.
- Both files are read at `load`. A runtime that can rebuild its TLS configuration without
  dropping open connections (Rust, Go) re-reads them when their modification time
  changes, checked at most once a minute, so cert-manager rotation needs no restart.
  Others document that a new client is needed.
- The InOrbit edge does not ask for client certificates today. mTLS is for a customer's
  own egress gateway or a private endpoint.

### 6.5 Connections, HTTP/2 and compression

- Connections are kept alive and pooled per client. HTTP/2 is negotiated by ALPN where
  the runtime supports it. Pool sizes and idle timeouts are the runtime's defaults; a
  caller who needs others supplies an HTTP client.
- Answers may be compressed: the SDK sends `Accept-Encoding: gzip` where the runtime
  decodes it, and the 16 MiB cap applies to the **decoded** body, so a small compressed
  body cannot inflate past the cap. Request bodies are not compressed.
- Redirects are never followed. A `3xx` is returned as an `ApiError`, and the
  `Authorization` header is never sent to another origin. A caller-supplied client gets
  its redirect policy overridden where the language allows (Go copies the client today);
  where it does not, the SDK checks the answer and refuses a `3xx`.

### 6.6 A caller-supplied HTTP client

| Language | Type |
|---|---|
| Rust | `reqwest::Client` |
| TypeScript | `fetch` function (an `undici` dispatcher is passed through the caller's own `fetch`) |
| Python | `httpx.Client` / `httpx.AsyncClient` |
| Go | `*http.Client` (copied; its `Transport` is the innermost `RoundTripper`) |
| Java | `java.net.http.HttpClient` |
| C# | `HttpClient` or an `HttpMessageHandler` (`SocketsHttpHandler`) |

With a caller-supplied client the pipeline still runs in full; only the transport
changes. Proxy, trust, mTLS, pinning, `connect_timeout` and pool settings belong to the
caller's client then:

- set in **code** next to `http_client`, they are a `ConfigError` ("configure the proxy
  on your HTTP client, or leave `http_client` out");
- set in the environment or the file, they are ignored and listed in `ignored`.

The token exchange and the socket upgrade use the same client wherever the runtime
allows, so a proxy or private CA configured there applies everywhere.

## 7. The middleware pipeline

### 7.1 The model

A call goes through an ordered list of middlewares, each a function of the request and
the rest of the pipeline. A middleware can change the request, short-circuit with a
response or an error, or inspect what comes back. The list has two stages, as in Azure
Core:

- **per call**: runs once per call, outside the retry loop;
- **per retry**: runs on every attempt, inside the retry loop.

`retry` sits between them. After the last per-retry middleware comes the transport.

### 7.2 Built-ins

Outermost first:

| # | Name | Stage | Default | What it does |
|---|---|---|---|---|
| 1 | `request_id` | per call | on | Generates the call's id, `iohr-` and 16 lowercase hex digits, and sends it as `x-request-id` on every attempt of the call. Reads the server's `x-request-id` into `server_request_id` on the result or error (SR-17) |
| 2 | `user_agent` | per call | on | `inorbithr-sdk-<lang>/<version> <runtime>/<version> <os>/<arch>[ <suffix>]` (section 7.6) |
| 3 | `idempotency_key` | per call | on | Section 7.5 |
| 4 | `call_tracing` | per call | on when tracing is on | One span per call (section 7.10) |
| 5 | `deadline` | per call | on | Enforces `total_timeout` and the caller's deadline |
| | *per-call slot* | per call | | Where `add_per_call` inserts |
| 6 | `retry` | boundary | on | Section 7.4 |
| 7 | `auth` | per retry | on | `Authorization: Bearer <token>` from the credential; one fresh token after a 401 |
| 8 | `rate_limit` | per retry | `observe` | Section 7.8 |
| 9 | `attempt_tracing` | per retry | on when tracing is on | One HTTP client span per attempt; sends `traceparent` and `tracestate` |
| 10 | `logging` | per retry | on, silent until `log` is set | Section 7.9 |
| 11 | `hooks` | per retry | on | Calls today's `Hook` methods with the attempt (section 7.7) |
| | *per-retry slot* | per retry | | Where `add_per_retry` inserts |
| 12 | `timeout` | per retry | on | Enforces `timeout` on the attempt |
| | transport | | | The HTTP client |

Why this order: the request id and the idempotency key are fixed before the first
attempt, so every attempt carries the same ones. Tracing wraps the deadline, so a call
that times out still has a span. Auth is per retry because a token can expire between
attempts. Logging comes after auth and tracing, so a record carries the trace id and the
attempt's final headers (redacted). A user's per-retry middleware sees the finished
request, the `Authorization` header included, and its own time counts against the
attempt's timeout.

### 7.3 Changing the pipeline

One `Pipeline` value per client, edited by name at construction, the same operations in
every language:

| Operation | Effect |
|---|---|
| `add_per_call(m)` | Insert at the per-call slot (just before `retry`), after earlier additions |
| `add_per_retry(m)` | Insert at the per-retry slot (just before `timeout`), after earlier additions |
| `insert_before(name, m)`, `insert_after(name, m)` | Insert next to a named middleware, built-in or not |
| `replace(name, m)` | Swap a middleware, keeping its position; `m` takes the name |
| `remove(name)` | Drop a middleware |

- Names are unique. Inserting a name that exists, or naming one that does not, is a
  `ConfigError` at construction.
- `retry`, `auth` and `timeout` can be replaced but not removed: without them a client
  would retry nothing, send no credentials or wait forever, which the settings already
  say better (`max_retries = 0`). Removing them is a `ConfigError`.
- Settings switch built-ins off without touching the pipeline: `rate_limit = "off"`,
  `tracing = false`, `log = "off"`. A switched-off built-in stays in the list and does
  nothing, so positions relative to it stay valid.
- `client.config().describe()` lists the final pipeline by name.

### 7.4 Retry

`design.md` section 6 stays the rule. It is now configurable through the settings of
section 3.3, and gains:

- **Writes with an idempotency key are retried.** A `POST` or `PATCH` whose operation
  accepts `Idempotency-Key` (section 7.5) is retried on the same conditions as a `GET`.
  Any other `POST` or `PATCH` is still never retried automatically.
- **`Retry-After`** may be delay-seconds or an HTTP date (RFC 9110 section 10.2.3).
  Today only seconds are read. A `retry` detail in the error envelope is used when the
  header is absent. A value over `retry_after_max` ends the call with the error instead
  of waiting.
- **The retry budget.** A token bucket per client, after the AWS standard retry mode:
  capacity 500. A retry after a `429`, or a `503` with `Retry-After`, costs 5; any other
  retry (connection error, timeout, `503`, `504`) costs 10. When the bucket cannot pay,
  the call ends with its last error. A call that succeeds after retries returns the cost
  of its last retry; a call that succeeds on the first attempt adds 1, up to the
  capacity. The first attempt never waits on the bucket. Stream reconnects draw from the
  same bucket. Under a real outage this turns a storm of retries into fast failures,
  which is why there is no separate circuit breaker (section 7.11). In tests the capacity
  can be set in code (`retry_budget_capacity`).
- Every retry fires `on_retry` (section 7.7) and a `warn` log record with the attempt,
  the reason and the delay.

### 7.5 Idempotency keys

The platform takes `Idempotency-Key` on every public create or trigger `POST` and one
`PATCH` (RFC 0033). A repeat with the same key and the same body within 24 hours answers
what the first call did, with `Idempotency-Replayed: true`. The OpenAPI document lists
the header on exactly those operations.

- The generator marks an operation that declares the header, or says
  `x-iohr-idempotency-key: true` (`idempotency_key: true` in the shared model and in
  every target's context, `iohr-codegen` `context::Op`; the marks per fixture are in
  `cli/crates/iohr-codegen/tests/golden/<case>/expected/operations.json`). Each target
  passes it to the runtime's `Operation` the way it passes `idempotent` today, in the
  same pull request that gives the runtime the field. The runtime reads the mark from
  `Operation`.
- On such an operation the call uses the caller's `idempotency_key` (a per-call option and
  a generated parameter), or generates a random UUID v4. It is generated once per call
  and sent unchanged on every attempt. It is generated even with `max_retries = 0`, so a
  caller who repeats the call themselves can reuse it from the error.
- The key is exposed on the result and the error (`idempotency_key`), and
  `Idempotency-Replayed: true` as `idempotency_replayed`.
- A `409` (the same key still in flight) and a `422` (the same key with another body)
  are `ApiError`s and are not retried.
- On an operation without the mark, the middleware does nothing, and a caller-supplied
  key is a `ConfigError` at the call: the API would ignore it, and the caller would
  believe the call was safe to repeat.

### 7.6 User agent

`inorbithr-sdk-<lang>/<version> <runtime>/<version> <os>/<arch>`, then the suffix after a
space. Normalised in M6 so analytics on the platform side see one vocabulary:

- `<lang>`: `rust`, `typescript`, `python`, `go`, `java`, `csharp`;
- `<runtime>`: `rust`, `node`, `deno`, `bun`, `browser`, `python`, `go`, `java`, `dotnet`;
- `<os>`: `linux`, `macos`, `windows`, `freebsd`, `android`, `ios`, or `other`;
- `<arch>`: `x86_64`, `aarch64`, `x86`, `arm`, `riscv64`, or `other`.

The token exchange sends the same user agent. Today four runtimes send their HTTP
library's default there.

### 7.7 Hooks

Today's `Hook` (`on_request`, `on_response`, `on_error`, with an `Attempt`) stays as it is
and is run by the `hooks` middleware. M6 adds:

- `on_retry(attempt, reason, delay)`, called before each retry's wait;
- `attempt.idempotency_key` and `attempt.stage`.

In Go, adding a method to the `Hook` interface would break every implementation, so
`on_retry` is a separate optional interface (`RetryHook`) found by type assertion, as
`Invalidator` is today. Hooks observe; a hook that needs to change a request is a
middleware.

### 7.8 Rate limits

The edge sends `X-RateLimit-Limit`, `X-RateLimit-Remaining` and `X-RateLimit-Reset`
(seconds until the window resets), Envoy's draft-03 headers. The IETF draft's `RateLimit`
and `RateLimit-Policy` (draft 11, structured fields) are a platform follow-up. The
middleware reads both; when both are present, the IETF fields win.

- `observe` (default): every result, error and raw response carries a `rate_limit`
  snapshot (`limit`, `remaining`, `reset` as a duration, `policy` when sent), and the
  client keeps the latest one (`client.rate_limit()`). Nothing waits.
- `wait`: as `observe`, and before an attempt, when the latest snapshot says `remaining`
  is 0 and its reset has not passed, the attempt waits until the reset, plus up to 100 ms
  of jitter. A wait that would pass `total_timeout` ends the call with a `TimeoutError`
  that says it was waiting for the rate-limit window. Concurrent calls of one client
  share the snapshot.
- `off`: headers are not read.
- A `429` is retried by `retry` as today, whatever the mode.

### 7.9 Logging

Off by default (SR-13). `log` sets the level. Records go to the language's standard
facility:

| Language | Sink |
|---|---|
| Rust | `tracing` events, target `inorbithr` (behind the default feature `tracing`), or a `logger` callback |
| TypeScript | a `logger` option with `debug`, `info`, `warn` and `error` methods; `console` when only `log` is set |
| Python | `logging.getLogger("inorbithr")` |
| Go | `*slog.Logger` (`WithLogger`; `slog.Default()` when only `log` is set). `SlogHook` stays and is deprecated in favour of it |
| Java | `System.Logger` named `hr.inorbit.sdk`, which SLF4J and Log4j bridge to |
| C# | `ILoggerFactory` (`Microsoft.Extensions.Logging.Abstractions`), category `InOrbit.Sdk` |

What is logged, with these field names in every language:

| Level | Record (`event`) | Fields |
|---|---|---|
| `debug` | `request`, `response` | `operation`, `method`, `path`, `attempt`, `request_id`, `status`, `duration_ms`, `server_request_id`, `headers` (only with `log_headers`) |
| `info` | `call` (one per call) | `operation`, `status` or `error_kind` and `error_code`, `attempts`, `duration_ms`, `request_id`, `server_request_id` |
| `warn` | `retry`, `token_refresh_failed`, `rate_limit_wait` | `attempt`, `reason`, `delay_ms`, `request_id` |
| `error` | `call_failed` | `operation`, `error_kind`, `error_code`, `status`, `request_id` |

Every record also carries `profile` and, with tracing on, `trace_id` and `span_id`.

Redaction:

- **Never logged, whatever the settings:** bodies, query values (the path is logged
  without its query string), and the headers `authorization`, `proxy-authorization`,
  `cookie` and `set-cookie`. Secrets of section 3 never appear in any record.
- **Headers** are logged only with `log_headers = true`, and only from the allowlist.
  Every other header appears by name with the value `REDACTED`, so its presence is
  visible. The default allowlist:
  - request: `accept`, `content-type`, `content-length`, `user-agent`, `x-request-id`,
    `traceparent`, `idempotency-key`;
  - response: `content-type`, `content-length`, `date`, `retry-after`, `x-request-id`,
    `idempotency-replayed`, `x-ratelimit-limit`, `x-ratelimit-remaining`,
    `x-ratelimit-reset`, `ratelimit`, `ratelimit-policy`.

  `log_allow_headers` adds names; the never-logged headers cannot be added.
- **`redact`** (SR-14) sees every record, after the rules above, and returns it changed
  or drops it. It is the last step before the sink.
- The path is logged as sent. It holds only the opaque identifiers the API defines
  (SR-15).

### 7.10 Tracing and metrics

OpenTelemetry is optional and never a hard dependency of the core package
(`design.md` section 8):

| Language | Integration |
|---|---|
| Rust | feature `otel` (`opentelemetry` API crate) |
| TypeScript | `@opentelemetry/api` as an optional peer dependency, used when installed |
| Python | extra `inorbithr[otel]` (`opentelemetry-api`), used when importable |
| Go | module `github.com/inorbithr/sdk/go/otel`: `inorbitotel.Pipeline()` registers both middlewares |
| Java | artifact `hr.inorbit:inorbit-sdk-otel` (`opentelemetry-api`) |
| C# | built in: `ActivitySource` and `Meter` named `InOrbit.Sdk` (no package; inert until a listener subscribes) |

The tracer and meter come from the global providers unless code passes
`tracer_provider` or `meter_provider`. `OTEL_SDK_DISABLED` is honoured by the
OpenTelemetry SDK itself. The SDK sends nothing anywhere itself: spans and metrics go to
the caller's pipeline (SR-16).

Spans, following the HTTP client semantic conventions (stable):

- `call_tracing`: one `INTERNAL` span per call, named for the operation
  (`radar.list_digests`), with `inorbit.operation`, `inorbit.request_id` and, on failure,
  `error.type`. Streams: the span covers opening to the end of the stream.
- `attempt_tracing`: one `CLIENT` span per attempt, a child of the call span. It is named
  `{method} {url.template}` (`GET /v1/radar/digests/{digest_id}`), or `{method}` when no
  template is known. Attributes: `http.request.method`, `server.address`, `server.port`,
  `url.full` (query values replaced by `REDACTED`), `url.template`,
  `http.response.status_code`, `error.type`, `http.request.resend_count` (from the
  second attempt on), `inorbit.server_request_id`, `inorbit.idempotency_replayed`.
- Propagation: W3C `traceparent` and `tracestate` from the attempt span. With tracing off
  or no integration installed, a `traceparent` the caller passes per call is sent
  unchanged (`design.md` section 4).

Metrics, when `metrics` is on:

| Name | Instrument | Unit | Attributes |
|---|---|---|---|
| `http.client.request.duration` | histogram, the semantic conventions' buckets | s | `http.request.method`, `server.address`, `server.port`, `http.response.status_code`, `error.type` |
| `inorbit.client.call.duration` | histogram | s | `inorbit.operation`, `error.type` |
| `inorbit.client.retries` | counter | `{retry}` | `inorbit.operation`, `inorbit.retry.reason` |
| `inorbit.client.token.exchanges` | counter | `{exchange}` | `inorbit.credential.source`, `error.type` |

No attribute holds an id, a path with identifiers, or anything from a body, so
cardinality stays bounded.

### 7.11 No circuit breaker

The SDK ships no circuit breaker. The retry budget (section 7.4) already turns sustained
failure into fast failure per client, the way AWS's retry quota and gRPC's retry
throttling do, and neither AWS nor Azure ships a breaker in its SDK core. Teams that run
a breaker already have one: Polly or `Microsoft.Extensions.Http.Resilience` (.NET),
resilience4j (Java), `failsafe-go`, cockatiel (TypeScript), `tower` (Rust). It belongs at
the per-call slot. [recipes.md](recipes.md) shows how.

### 7.12 Streams

| Middleware | Server-sent events | The `/v1/ws` socket |
|---|---|---|
| `request_id`, `user_agent` | yes, on the opening request | yes, on the upgrade |
| `idempotency_key` | not applicable (a `GET`) | not applicable |
| `call_tracing` | one span per stream | one span per stream |
| `deadline` | until the stream opens | until the upgrade answers |
| `retry` | opening only; an ended stream is not resumed (design.md section 7) | upgrade and reconnects, with the retry budget |
| `auth` | yes, and one fresh token after a 401 | yes, on every upgrade |
| `rate_limit` | yes, on the opening answer | yes, on the upgrade answer |
| `attempt_tracing` | the opening request | the upgrade |
| `logging` | opening, end and failure | upgrade, reconnects, socket-level errors |
| `hooks` | yes | where the runtime sees the upgrade (below) |
| user middleware | yes; it must not read the body | the upgrade only, where the runtime sees it |
| `timeout` | until the headers; then `stream_idle_timeout` | until the upgrade answers |

The socket upgrade goes through the pipeline in Rust, Go and Java. TypeScript's
`WebSocket`, Python's `websockets` and .NET's `ClientWebSocket` cannot pass a request
through a pipeline. There the built-ins set their headers directly, user middleware does
not see the upgrade, and `design.md` section 7 lists the exception. In every runtime the
upgrade sends `x-request-id` in the `iohr-` form; today TypeScript and C# send none and
Python sends another form.

### 7.13 The middleware contract

A middleware receives a `Request` and a `Next`, and returns a `Response` or an error of
the SDK's error family:

- `Request`: method, URL, headers (mutable), body as bytes (`None` when there is none;
  never a stream), and `info`.
- `info` (read only): `operation`, `idempotent` (whether `retry` may repeat it),
  `idempotency_key`, `request_id`, `attempt` (1-based; 0 in the per-call stage),
  `deadline`, `stream` (whether the answer is a stream), `profile`.
- `Response`: status, headers, and the body as bytes, or a body handle when `stream` is
  true, which a middleware must not read.
- A middleware may call `next` zero times (short-circuit), once, or more than once (a
  user retry). Each call of `next` from the per-call stage is a fresh attempt.
- A middleware must not log secrets or bodies. The built-ins hold to section 7.9; a
  user's middleware is the user's responsibility, and the docs say so.

## 8. Per-language shape

The concept is one; the idiom is each language's. Signatures are sketches for the
implementers; the doc comments in each runtime are the reference.

| Language | Middleware | Registering |
|---|---|---|
| Rust | `trait Middleware: Send + Sync + 'static { fn name(&self) -> &'static str; fn handle<'a>(&'a self, req: Request, next: Next<'a>) -> BoxFuture<'a, Result<Response, Error>>; }` with `next.run(req).await`. Own trait (no `tower` dependency); a `tower` adapter can follow behind a feature | `Client::builder().pipeline(\|p\| p.add_per_retry(Probe).remove("rate_limit")).load()?` |
| TypeScript | `interface Middleware { readonly name: string; handle(req: SdkRequest, next: (req: SdkRequest) => Promise<SdkResponse>): Promise<SdkResponse> }` | `Client.load({ pipeline: (p) => p.addPerRetry(probe).remove("rate_limit") })` |
| Python | `class Middleware(Protocol): name: str; def __call__(self, request: Request, call_next: Callable[[Request], Response]) -> Response`; `AsyncMiddleware` with `async def __call__` for `AsyncClient` | `Client.load(pipeline=lambda p: p.add_per_retry(Probe()).remove("rate_limit"))` |
| Go | `type Middleware struct { Name string; Wrap func(next http.RoundTripper) http.RoundTripper }`; `inorbit.CallInfoFrom(req.Context())` gives `info`. Existing `RoundTripper` wrappers (for example `otelhttp.NewTransport`) fit as is | `inorbit.Load(inorbit.WithPipeline(func(p *inorbit.Pipeline) { p.AddPerRetry(probe); p.Remove("rate_limit") }))` |
| Java | `interface Middleware { String name(); Response handle(Request request, Chain chain) throws InOrbitException; }` with `chain.proceed(request)` and `chain.info()` (OkHttp's interceptor shape) | `Client.builder().pipeline(p -> p.addPerRetry(probe).remove("rate_limit")).load()` |
| C# | `abstract class Middleware { public abstract string Name { get; } public abstract ValueTask<SdkResponse> SendAsync(SdkRequest request, MiddlewareNext next, CancellationToken cancellationToken); }`, plus `Middleware.FromHandler(name, DelegatingHandler)` so Polly and other `DelegatingHandler`s can sit at either slot | `Client.Load(new ClientOptions { Pipeline = p => p.AddPerRetry(new Probe()).Remove("rate_limit") })` |

Settings in code use each language's existing option style: builder methods (Rust,
Java), option object fields (TypeScript, C#), keyword arguments (Python), `With...`
functions (Go). Names are the catalogue's in the language's case: `total_timeout`,
`totalTimeout`, `WithTotalTimeout`, `TotalTimeout`.

Dependencies M6 adds, each with its reason (SR-20):

| Language | TOML | Logging | Tracing |
|---|---|---|---|
| Rust | `toml` | `tracing` (default feature) | `opentelemetry` (feature `otel`) |
| TypeScript | `smol-toml` (no dependencies of its own) | none | `@opentelemetry/api` (optional peer) |
| Python | `tomllib` (standard library, 3.11+) | `logging` (standard library) | `opentelemetry-api` (extra) |
| Go | `github.com/BurntSushi/toml` (no dependencies of its own) | `log/slog` (standard library) | submodule |
| Java | `jackson-dataformat-toml` (Jackson is already a dependency) | `System.Logger` (JDK) | separate artifact |
| C# | `Tomlyn` (no dependencies of its own) | `Microsoft.Extensions.Logging.Abstractions` | `System.Diagnostics` (in the runtime) |

## 9. Conformance

### 9.1 Cases against the replay server

New areas in `conformance/cases/`: `credentials`, `middleware`, `transport`, plus new
cases in `retries`. All are `pending` for the six languages until each passes them. The
schema additions (`conformance/case.schema.json`):

- `client`: `load: true` (build with `load`, not explicitly), `env` (the environment
  `load` sees, injected, never the process environment), `config_file` (TOML text the
  driver writes to a temporary file), `files` (name to content, written next to it;
  `{dir}` in any value is that directory), `profile`, `credential_sources`, `cli: true`
  (point `cli_path` at the replay binary's fake `iohr`), `pipeline` (probe middlewares to
  add or built-ins to remove), `log` and `log_headers` (capture records),
  `rate_limit`, `total_timeout_ms`, `retry_budget_capacity`, `tracing: true` (an
  in-memory exporter), `transport` (`https`, `mtls`, `proxy`: which replay listener and
  settings to use), `ca_bundle: false` (leave the replay CA out).
  In `env` and `config_file`, `{replay}` is the replay server's URL and `{dir}` the
  temporary directory.
- `action`: `options` (per-call `idempotency_key`, `traceparent`, `timeout_ms`) and
  `rewrite` (`after` N calls, files to rewrite: rotation).
- `request`: header values `"*"` (present, any value), `"$name"` (captured the first time,
  equal afterwards, as socket steps do), `"~<regex>"`; `headers_absent`; `via: proxy` or
  `direct`; `client_cert` (the subject the client presented).
- `expect`: `probes` (per probe: how many times it ran and a subset of the headers it saw
  each time), `logs` (`contains`: subsets of records; `excludes`: strings that appear in
  no record), `spans` (name, kind, attribute subsets, count), `rate_limit` (the snapshot
  on the result), `config` (a subset of `describe()`), `idempotency_key` (on the result).

Replay server features, needed before the first runtime can pass them:

1. Header matchers `*`, `$name` and `~regex`, and `headers_absent`.
2. A TLS listener with a CA generated at start (`ca.pem` in a temporary directory) and
   a leaf for `127.0.0.1` and `localhost`.
3. An mTLS listener requiring a client certificate signed by that CA, with a client
   certificate and key written next to it, and `client_cert` matching.
4. A CONNECT proxy listener that tunnels to the TLS listener and marks the requests it
   carried, so `via` can be matched.
5. `POST /_case` answers these locations (`https_url`, `mtls_url`, `proxy_url`,
   `ca_file`, `client_cert_file`, `client_key_file`) next to the case.
6. A fake `iohr`: the replay binary, run as `replay auth token --profile P --format json`,
   prints `{"access_token":"cli-<P>","expires_at":<now + 15 min>}`, and exits 1 with a
   message for a profile named `missing`.
7. The self-test learns the new matchers, so it sends values that satisfy them instead of
   the literal patterns.

### 9.2 Vectors

Resolution, `no_proxy` matching and header parsing are pure functions; they are tested
without a server, from `conformance/vectors/` (schema `conformance/vector.schema.json`):

| Directory | Each vector gives | The driver checks |
|---|---|---|
| `config/` | code options, environment, config file text, OS, typed profile | `describe()` subset, or the `ConfigError` problems |
| `config-path/` | OS and environment (`HOME`, `XDG_CONFIG_HOME`, `APPDATA`, `IOHR_CONFIG_DIR`, `INORBIT_CONFIG_FILE`) | the config file path chosen |
| `no-proxy/` | `no_proxy` value, URLs | proxied or not, per URL |
| `rate-limit/` | response headers | the snapshot |
| `durations/` | strings | the value or an error |

To run them, each runtime exposes resolution with injected inputs: `load` takes an
environment map, an OS name and a home directory as `load_options`
(`LoadOptions { env, os, home }`; `loadOptions`, `WithLoadOptions`, `LoadOptions`). That is public, because the same hook lets a user's
own tests resolve a configuration without touching the process environment. Each
language's driver runs every vector as a unit test next to its conformance driver.

## 10. Order of work (M6)

1. `cli/`: keep unknown keys and the `[sdk]` table when rewriting `config.toml`
   (`toml_edit`, which also keeps comments), add `iohr auth token` and
   `iohr sdk config`, and fix `IOHR_TOKEN` being ignored once a default profile exists.
   Done in iohr 0.1.0-alpha.8.
2. The replay server features of section 9.1; `tools/validate-cases.py` validates
   vectors.
3. The generator marks operations that take `Idempotency-Key`. `a-write-is-not-retried`
   moves to an operation without the header (`events.update_endpoint`), and every driver
   learns that operation.
4. Rust: settings, `load`, `describe`, the credential chain, the pipeline, then
   transport (proxy, trust, mTLS), then logging and the `otel` feature.
5. TypeScript, Python, Go, Java, C#, in that order, each lifting `pending` as it passes.
6. READMEs show `load` first. `design.md` section 2 points here.
