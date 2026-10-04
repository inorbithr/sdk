# Recipes

Draft, 2026-10-04. These recipes show how to configure the SDK for common environments.
Each uses only the settings of [config.md](config.md). They describe milestone M6, which
is not built yet: the code lines are sketches until each runtime ships `load`. At that
point they move into `examples/` and are compiled in CI, as `style.md` requires. In
every recipe the program is the same one line, so only the environment or the file
changes:

```rust
let client = inorbithr::Client::load()?;          // Rust
```
```ts
const client = Client.load();                     // TypeScript
```
```python
client = Client.load()                            # Python
```
```go
client, err := inorbit.Load()                     // Go
```

`iohr sdk config` (or `client.config().describe()`) prints what a client will use and where
each value came from, with secrets redacted. Use it first when a recipe does not behave.

## Local development after `iohr login`

```sh
iohr login                 # browser sign-in; writes the profile and keeps the session in the keychain
```

Nothing else is needed. `load` finds no `INORBIT_*` credentials, reads `config.toml`,
takes the default profile `iohr login` set, and gets tokens from `iohr auth token`. To
use another signed-in profile for one run:

```sh
INORBIT_PROFILE=acme python report.py
```

## CI: GitHub Actions with a repository secret

Use an API token (RFC 0016, the shortest lifetime that covers the job's schedule) or a
key with only the scopes the job needs:

```yaml
jobs:
  report:
    runs-on: ubuntu-latest
    permissions: { contents: read }
    env:
      INORBIT_KEY_ID: ${{ vars.INORBIT_KEY_ID }}
      INORBIT_KEY_SECRET: ${{ secrets.INORBIT_KEY_SECRET }}
      INORBIT_SCOPES: identity:read radar:read
      INORBIT_CREDENTIAL_SOURCES: env          # never fall back to anything else on a runner
    steps:
      - uses: actions/checkout@<pinned sha>
      - run: python report.py
```

For a generated surface with profiles, the job sets each profile's variables,
`INORBIT_<PROFILE>_KEY_ID` and so on, and the command line's
`IOHR_TOKEN_<PROFILE>` for `iohr sdk check`.

**GitHub Actions OIDC** (no stored secret) waits for the platform to accept workload
identity (roadmap M6, platform follow-up 1). When it does, the job will grant
`id-token: write` and set `INORBIT_FEDERATED_KEY_ID`, and the `workload` source will
exchange the runner's token. Nothing in the program changes.

## Kubernetes with a mounted Secret, rotated without restart

```yaml
env:
  - { name: INORBIT_KEY_ID, value: ak_7f3c }
  - { name: INORBIT_KEY_SECRET_FILE, value: /var/run/secrets/inorbit/key-secret }
  - { name: INORBIT_SCOPES, value: "identity:read usage:read" }
  - { name: INORBIT_CREDENTIAL_SOURCES, value: env }
  - { name: INORBIT_CONFIG_FILE, value: "off" }
volumeMounts:
  - { name: inorbit, mountPath: /var/run/secrets/inorbit, readOnly: true }   # no subPath: it stops updates
volumes:
  - name: inorbit
    secret: { secretName: inorbit-key }
```

The secret file is read before every token exchange. After a key rotation (new secret
written to the Secret; the kubelet updates the file within a minute or so), the next
refresh uses it. Keep the old secret valid until one token lifetime (15 minutes) after
the file changed.

A **projected service account token** (`serviceAccountToken` volume, audience
`iohr-api`) will plug in through `INORBIT_WEB_IDENTITY_TOKEN_FILE` once the platform
exchanges it (platform follow-up 1).

## Behind a corporate proxy with a TLS-inspecting CA

```sh
export HTTPS_PROXY=http://proxy.corp.example:3128
export NO_PROXY=localhost,.corp.example,10.0.0.0/8
export INORBIT_CA_BUNDLE=/etc/pki/corp/root.pem    # added to the system roots, not replacing them
```

Or for every profile on a workstation, in `config.toml`:

```toml
[sdk]
proxy = "http://proxy.corp.example:3128"
no_proxy = "localhost,.corp.example"
ca_bundle = "/etc/pki/corp/root.pem"
```

A proxy that needs a password takes it in the URL from the environment only
(`INORBIT_PROXY=http://user:pass@proxy:3128`), never from the file. Streams and the
socket use the same proxy. `no_proxy` behaves the same in every language (`config.md`
section 6.2). In particular, `.corp.example` and `corp.example` mean the same thing.

## mTLS to a private gateway

A company that sends API traffic through its own egress gateway, which requires a
client certificate:

```toml
[profiles.prod]
base_url = "https://inorbit-gw.corp.example"
token_url = "https://inorbit-gw.corp.example/oauth2/token"
client_cert = "/etc/inorbit/tls/client.crt"
client_key = "/etc/inorbit/tls/client.key"
ca_bundle = "/etc/inorbit/tls/gateway-ca.pem"
system_trust = false            # trust only the gateway's CA
```

A key password, if any, comes from `INORBIT_CLIENT_KEY_PASSWORD`. In Rust and Go,
cert-manager rotation of the two files is picked up without a restart; in the other
languages, create a new client after a rotation (`config.md` section 6.4).

## Several accounts in one process

With a generated surface, each profile is a type, and each reads only its own
credentials:

```sh
INORBIT_ACME_KEY_ID=...   INORBIT_ACME_KEY_SECRET_FILE=/run/secrets/acme   INORBIT_ACME_SCOPES="radar:read"
INORBIT_GLOBEX_TOKEN_FILE=/run/secrets/globex-token
INORBIT_TIMEOUT=10s       # process settings apply to both unless INORBIT_ACME_TIMEOUT overrides
```

```rust
let acme: Client<Acme> = Client::load()?;
let globex: Client<Globex> = Client::load()?;
```

With the published surface, pass the profile in code: `Client.load({ profile: "acme" })`
reads `[profiles.acme]` and the `iohr` login for it.

## A custom endpoint or an air-gapped network

```sh
INORBIT_BASE_URL=https://inorbit.internal.example
INORBIT_TOKEN_URL=https://auth.inorbit.internal.example/oauth2/token
INORBIT_CA_BUNDLE=/etc/pki/internal-root.pem
INORBIT_PROXY=off               # ignore any proxy the host's environment sets
```

The SDK contacts nothing but these two hosts (SR-16). It does not fail over to another
host or region (SR-09).

## Serverless: cold start, no config file

```sh
INORBIT_CONFIG_FILE=off
INORBIT_CREDENTIAL_SOURCES=env
INORBIT_KEY_ID=ak_...
INORBIT_KEY_SECRET=...          # from the platform's secret store, injected as an environment variable
INORBIT_SCOPES=identity:read
INORBIT_TOTAL_TIMEOUT=20s       # inside the function's own limit
```

Build the client once, outside the handler, so warm invocations reuse its connection
pool and cached token. `load` reads no file and runs no process with these settings; the
first call pays one token exchange.

## An existing OpenTelemetry setup

Install the integration (Rust feature `otel`, `@opentelemetry/api`, `inorbithr[otel]`,
`github.com/inorbithr/sdk/go/otel`, `inorbit-sdk-otel`; nothing for .NET). The SDK uses
the global tracer and meter providers your application already set up. Each call becomes
an `INTERNAL` span named for the operation, with one `CLIENT` span per attempt, and
`traceparent` is sent from the attempt span. The InOrbit gateway continues the trace on
its side. To keep your own spans and switch the SDK's off, set `INORBIT_TRACING=false`;
a `traceparent` you pass per call is still sent.

In Go, register the middlewares explicitly:

```go
client, err := inorbit.Load(inorbitotel.Pipeline())
```

## Strict logging for an audited environment

```sh
INORBIT_LOG=info                # one record per call: operation, status, attempts, duration, request ids
INORBIT_LOG_HEADERS=false       # the default; headers never appear
```

Bodies, query values, credentials and cookies never reach a log at any level. To mask
something your own policy marks as sensitive, register `redact` in code. It sees every
record last and can change or drop it. Quote `request_id` and `server_request_id` in
support requests; both are on every result and error.

## A circuit breaker your platform team already uses

Add it at the per-call slot so it sees calls, not attempts:

```csharp
var client = Client.Load(new ClientOptions {
    Pipeline = p => p.AddPerCall(Middleware.FromHandler("breaker", new ResilienceHandler(myPipeline))),
});
```

The same position takes resilience4j in Java, `failsafe-go`, cockatiel in TypeScript, or
a `tower` layer through an adapter in Rust.

## Tests

**Against the conformance replay server**, the way this repository tests itself:

```sh
mise run conformance:server     # prints http://127.0.0.1:PORT
INORBIT_BASE_URL=http://127.0.0.1:PORT INORBIT_TOKEN_URL=http://127.0.0.1:PORT/oauth2/token \
INORBIT_CONFIG_FILE=off INORBIT_KEY_ID=ak_test INORBIT_KEY_SECRET=s3cr3t INORBIT_SCOPES=identity:read \
  pytest
```

`http://` is allowed because the address is loopback (SR-07).

**Without a network**, supply a fake transport: an `http_client` (`httpx.MockTransport`,
an `http.RoundTripper`, a `fetch` function), or a per-retry middleware that answers
without calling `next`. Resolve configuration in a test without touching the process
environment by passing the environment to `load`:

```python
client = Client.load(config_file="off", load_options=LoadOptions(env={"INORBIT_TOKEN": "t"}))
```
