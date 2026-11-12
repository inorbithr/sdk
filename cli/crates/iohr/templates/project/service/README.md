# @@NAME@@

@@DESCRIPTION@@

An HTTP service in @@LANGUAGE_LABEL@@ with what a service needs on its first day: probes, JSON
logs that carry a trace id, one span per request, request metrics and a clean shutdown. It
uses the standard library alone, so there is nothing to install.

| Route | Answers |
| --- | --- |
| `GET /healthz` | 200 while the process is up (liveness) |
| `GET /readyz` | 200 once it serves, 503 while it starts and while it shuts down (readiness) |
| `GET /metrics` | request counts by route and status, Prometheus text |
| `GET /v1/hello?name=NAME` | `{"message":"Hello, NAME"}`; your first route, replace it |

## What is here

@@LAYOUT@@
- `.env.example`: the variables the service reads, names only.
- `IO.md`, `AGENTS.md`: how to work in this folder, for people and agents.
- `.github/`: CI (test, secret scan, SBOM), the signed and attested release, dependency updates, CODEOWNERS.
- `SECURITY.md`: how to report a vulnerability.
- `inorbit.toml`, `.inorbit/space`: the InOrbit project record and the link to its space.

## Run

```sh
@@RUN@@                  # listens on HOST:PORT (default 127.0.0.1:8080)
curl -i localhost:8080/healthz
```

Send a `traceparent` header and the same trace id comes back on the response and in the
request's log line and span. The log never holds a header, a query string or an environment
value: it holds the route that matched (`unmatched` when none did), the status, the duration
and the trace id.

## Test

```sh
@@README_TEST@@
```

The test boots the service on a free port, calls `/healthz` and `/readyz`, checks that a request
produced one span with its route, and checks that the log holds the trace id and none of the
environment's values.

## Release

Push a tag `v*`. `.github/workflows/release.yml` builds, writes a CycloneDX SBOM, signs every
file with cosign (keyless) and attaches a build-provenance attestation. Its steps and how to
verify a download are in the workflow's header.

## Licence

@@LICENSE@@
