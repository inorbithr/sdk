# Security requirements

What every SDK must do before 1.0, in all four languages. Each requirement is binding for
code review: a pull request that breaks one is not merged, and a requirement changes only
through an ADR. The client options these create are in [design.md](../design.md)
section 11.

Each entry gives the requirement, why, how it is verified, and the framework items it
serves where the mapping is real:

- **SSDF**: NIST SP 800-218 v1.1 practice ids.
- **CRA**: Regulation (EU) 2024/2847, Annex I. `I.2(x)` is Part I point 2(x), the
  essential product requirements; `II(n)` is Part II point n, vulnerability handling.
- **OSPS**: OpenSSF OSPS Baseline control ids.

Verification is one of: a unit test in each language, a conformance case run by all four
(`conformance/cases/`), or review against this page.

## Transport

**SR-01. TLS 1.2 minimum.** Connections use TLS 1.2 or 1.3; TLS 1.3 is preferred where
the runtime supports it. Older versions are never negotiated.
- Why: older TLS versions have known weaknesses; customers in finance and health audit
  for this.
- Verify: unit test against a TLS 1.1-only server fails the handshake.
- Refs: SSDF PW.9; CRA I.2(e), I.2(f).

**SR-02. Certificate verification is always on.** Server certificates and host names
are verified against the trust store. The public API has no option that turns
verification off. A test-only escape, if a language needs one, lives behind a
non-default feature or build tag and is named `dangerous_*`.
- Why: a verification switch is the most common way client libraries end up insecure
  in production.
- Verify: unit test against a self-signed server fails; review of the public API.
- Refs: SSDF PW.9; CRA I.2(b), I.2(e).

**SR-03. Custom CA bundle.** A caller can add CA certificates (PEM) to verify the API
host, for corporate proxies that re-sign TLS. The system trust store stays the default
(Rust: `rustls-platform-verifier`; Python: `truststore`).
- Verify: unit test with a private CA.
- Refs: SSDF PW.9.

**SR-04. Client certificates (mTLS).** A caller can present a client certificate and
key (PEM; PKCS#12 where the language makes it simple). Where the language's TLS stack
allows it, a signer hook lets the key stay in an HSM or OS key store.
- Verify: unit test against a server that requires a client certificate.
- Refs: SSDF PW.9.

**SR-05. Proxies.** The SDK honours `HTTPS_PROXY` and `NO_PROXY` (case as the language
convention expects) and accepts an explicit proxy option, including proxies that need
authentication. Proxy credentials are treated as secrets (SR-10).
- Verify: unit test through a local CONNECT proxy.

**SR-06. Optional certificate pinning.** A caller can pin the API's public key (SPKI
SHA-256) with at least one backup pin. Pinning is off by default: it breaks on key
rotation and in networks that inspect TLS, which banks often run.
- Verify: unit test with a matching and a mismatching pin.
- Refs: SSDF PW.9.

**SR-07. HTTPS-only base URLs.** `base_url` and `token_url` must be `https://`. Plain
`http://` is refused at construction, except for loopback addresses, which tests and the
conformance replay server use.
- Verify: unit test; conformance runs on loopback.
- Refs: CRA I.2(b), I.2(e).

**SR-08. FIPS-capable modes.** Each SDK can run with FIPS-validated cryptography
supplied by its runtime, and its documentation says how. The SDK does no cryptography of
its own.
- Go: `GOFIPS140` at build time or `GODEBUG=fips140=on`, using the Go Cryptographic
  Module.
- Rust: a `fips` feature that enables rustls's FIPS mode through aws-lc-rs and asserts
  it at client construction.
- TypeScript: Node started with the OpenSSL 3 FIPS provider (`--enable-fips`).
- Python: the system OpenSSL with its FIPS provider enabled.

Documentation says "FIPS-capable", never "FIPS-validated": the validation belongs to the
cryptographic module the customer runs, not to the SDK.
- Verify: review; a CI job builds the Go and Rust variants.
- Refs: SSDF PW.9.

**SR-09. Region selection with no silent fallback.** A `region` option (`eu`, `us`,
when the API offers them) or an explicit `base_url` selects where requests go. The SDK
never retries or fails over to another region on its own.
- Why: data residency is a contractual promise for banks and hospitals; a silent
  fallback breaks it.
- Verify: conformance case: a 503 from the selected region is retried there or
  returned, never sent elsewhere.
- Refs: customers' contractual data residency (DORA Art. 30(2)(b), GDPR Chapter V).

## Secrets and data

**SR-10. Secrets are typed and redacted.** Key secrets, access tokens, proxy passwords
and private keys are held in a `Secret` type. Printing, formatting, serialising or
logging it shows `<redacted>`: Go `String`/`GoString`/`Format`, Rust `Debug`/`Display`,
TypeScript `toString`/`toJSON`/`util.inspect`, Python `__repr__`/`__str__`. The key id
(`ak_...`) is not secret and may be shown.
- Verify: a unit test per language for every way to print; conformance case
  `secret-never-in-message`.
- Refs: SSDF PW.5; CRA I.2(e); OSPS BR-07.

**SR-11. Secrets are wiped where the language allows.** Rust zeroes secret memory on
drop (`zeroize`, `secrecy`). Go, TypeScript and Python cannot guarantee it; their docs
say so and keep secrets in as few copies as possible.
- Verify: review; Rust unit test on the type.
- Refs: CRA I.2(e).

**SR-12. Tokens live in memory only.** The token cache is in process memory. The SDK
never writes a token or a secret to disk, environment variables or shared storage. This
binds the libraries; the command line keeps credentials under SR-24 (ADR 0009).
- Verify: review.
- Refs: CRA I.2(e).

**SR-13. No bodies in logs.** Logging is off by default. When the caller turns it on,
the SDK logs method, path template, status, attempt, duration and request id. It never
logs request or response bodies, query values, or the `Authorization` header.
- Verify: unit test capturing debug logs during a call that carries a body.
- Refs: SSDF PW.5; CRA I.2(e), I.2(g).

**SR-14. Redaction hook.** A caller can register a function that sees every log record
and error message before it leaves the SDK, and can mask fields their own policy marks
as sensitive.
- Verify: unit test.
- Refs: CRA I.2(e), I.2(g).

**SR-15. No personal data in URLs or errors.** SDK methods never place values that could
be personal data, health data (PHI) or card numbers in URL paths or query strings, except
opaque identifiers the API defines (`org_id`). Error messages built by the SDK include
the code, status and request id, and never echo request values.
- Why: URLs end up in proxy logs, access logs and browser history; messages end up in
  bug trackers.
- Verify: review of every operation; conformance cases check messages.
- Refs: CRA I.2(g); GDPR Art. 25 (data protection by design).

**SR-16. No telemetry.** The SDK sends nothing anywhere except the requests the caller
makes to the API and the token endpoint: no usage pings, no crash reports, no update
checks. The command line has one exception, the extension registry, under SR-25;
`iohr sdk add` contacts nothing itself and runs the project's package manager (SR-31).
- Verify: review; conformance runs with only the replay server reachable.
- Refs: CRA I.2(g).

**SR-24. Command-line credentials.** The `iohr` command line keeps secrets only in the
operating system's credential store (macOS Keychain, Windows Credential Manager, the
Secret Service on Linux), one entry per profile and account, so two accounts never
share an entry. The config file never holds a secret. A plain file store exists only
behind `--insecure-storage`, mode 0600 in a 0700 directory. A token from `IOHR_TOKEN`
stays in memory. A token is never accepted as a command-line argument. A connection's
secret (`iohr connections add` and `reconnect`) is typed without echo, or read from a
file or stdin; it stays in wiped memory until the one request that carries it, and is
never stored, printed or accepted as an argument. `iohr auth token` prints a profile's access token on
standard output, for a program that runs it (the SDKs' `cli` source); the refresh token
never leaves the credential store, and the command writes nothing but the store's own
rotation.
- Why: a developer machine holds credentials for several accounts; shell history, the
  process list and dotfile backups are where tokens leak.
- Verify: unit tests for entry keys and file modes; a test that refuses a token in
  `argv`; a test that runs every command with `--verbose` and finds no token or
  connection secret in the output; a test that a connection secret travels only in the
  body of the call that makes the connection; a test that `iohr auth token` prints the
  access token and never the refresh token; the credential store tested on each operating system in CI.
- Refs: CRA I.2(e); clig.dev "Arguments and flags".

**SR-29. No secrets in the config file.** The SDK refuses a key secret, a token, a
private-key password or a proxy password found in the config file it shares with the
command line, and names the alternatives (a secret file, the environment, `iohr login`).
Secrets reach the SDK from code, the environment, a file holding only the secret, or the
command line's credential store (ADR 0015).
- Why: a config file is copied by dotfile backups, support bundles and configuration
  management; a secret in it outlives every rotation.
- Verify: vectors `a-secret-in-the-file-is-refused`, `a-proxy-password-in-the-file-is-refused`.
- Refs: CRA I.2(e).

**SR-30. Logging by allowlist.** With logging on, header values are logged only from an
allowlist, other headers by name with `REDACTED`, and `authorization`,
`proxy-authorization`, `cookie` and `set-cookie` never, whatever the configuration.
Bodies and query values are never logged (SR-13). `describe()` and every error redact
secrets and URL user-info.
- Verify: conformance cases `logs-carry-metadata-never-secrets-or-bodies`,
  `allowlisted-headers-are-logged-others-redacted`; vector `secrets-are-redacted-in-describe`.
- Refs: SSDF PW.5; CRA I.2(e), I.2(g).

## Command-line extensions

Added by [ADR 0012](../adr/0012-extensions.md). They bind the `iohr` command line only.

**SR-25. The extension registry, and only for extension commands.** `iohr` contacts
the extension registry (`ghcr.io/inorbithr/iohr-ext`, or the mirror `ext.registry`
names) only in `iohr ext install`, `upgrade` and `sync`: that registry, the token
service its challenge names and the storage host a blob download redirects to, with
`GET`, over HTTPS (plain HTTP to this machine only). Never on another command, never to
look for updates, no telemetry. A mirror's credential comes from
`IOHR_EXT_REGISTRY_AUTH` and goes to that registry and its token service only.
- Why: banks and regulated companies do not allow software that changes or reports on
  its own; a security team needs to know every host the tool reaches.
- Verify: review; integration tests run against a registry on loopback with nothing
  else reachable; a test that `--verbose` never shows the registry credential.
- Refs: CRA I.2(g); SSDF PW.4.

**SR-26. Extensions are verified before they are used.** Every manifest and blob is
checked against its SHA-256 digest and declared size. An artifact is installed only
with a signature and SLSA provenance from InOrbit's release workflow (Sigstore keyless:
Fulcio certificate chain, SCT, Rekor entry, identity
`https://github.com/inorbithr/<repo>/.github/workflows/release.yml@refs/tags/*`, issuer
`https://token.actions.githubusercontent.com`, provenance naming the same repository),
or with a signature by a public key in `ext.trusted_keys`. Every installed program is
re-hashed before it runs. No option skips a check.
- Why: the registry and any mirror in between are not trusted; the release workflow is.
- Verify: unit tests against a real Sigstore bundle (accepted for its own signer,
  refused for another identity, a tampered signature or another digest); integration
  tests for a tampered blob, an unsigned artifact, a stranger's key and a program
  changed on disk.
- Refs: SSDF PS.2, PW.4; CRA I.2(a), I.2(f); SLSA Build L3 (consumer side).

**SR-27. Extensions never hold the person's credentials.** An extension runs as its
own process. It gets an access token only through a channel private to the user (a
Unix socket of mode 0600 in a directory of mode 0700, or a named pipe that refuses
remote clients), only for scopes its manifest declares, and only while it runs. The
refresh token, the credential store and `IOHR_TOKEN*` never reach it.
- Why: an extension is a different program with its own vulnerabilities; it must not
  be able to keep or widen access.
- Verify: unit tests for the channel (undeclared scope refused, scope the credential
  lacks refused); an integration test that runs an extension and checks its
  environment, the socket's modes, the scoping and that the socket is gone afterwards.
- Refs: CRA I.2(d), I.2(e); least privilege.

**SR-28. Extensions are pinned and change only on request.** What is installed is
recorded in `iohr-ext.lock` (name, version, digest, signer). `iohr ext sync` installs
exactly a lock's entries and refuses another digest, version or signer. Only
`iohr ext upgrade` moves to a newer version.
- Why: a team and its pipelines must run the same reviewed code; a security team must
  be able to read what that is.
- Verify: integration tests for install with `--lock`, sync and a signer mismatch.
- Refs: SSDF PS.3; CRA I.2(c).

**SR-32. Privileges are shown and confirmed before install.** An extension's manifest
may name the Linux capabilities its system service holds (`privileges`, platform RFC
0061): at most 16, each one the kernel defines, none twice; absent means none. `iohr`
grants none of them, since it runs as the person; the service gets them from its own
package or unit. `iohr ext install` shows each privilege in plain words and installs a
privileged extension only after the person types `yes` at a terminal or passes `--yes`;
without a terminal and without `--yes` it refuses and installs nothing. `iohr ext
upgrade` names privileges the new release adds and asks again under the same rules.
`iohr-ext.lock` records the confirmed privileges (format version 2 once an entry has
any, so an `iohr` from before this requirement refuses the file instead of ignoring
them); `iohr ext sync` refuses an artifact that declares a privilege its lock entry
does not record, and treats the privileges a lock records as confirmed by whoever
committed it. `iohr <name>` and `iohr ext verify` refuse an installed version whose
privileges the machine's lock does not record. `iohr ext list` and `verify` show them,
and `--json` includes them.
- Why: an extension that controls a privileged service (eBPF capture holds `CAP_BPF`,
  `CAP_PERFMON`, `CAP_NET_ADMIN`) changes what the machine exposes; a person and a
  security team must see that before it is installed, and a pipeline must not take on a
  new privilege silently.
- Verify: unit tests for the manifest rules, the lock format and the confirmation
  (typed `yes`, any other answer, `--yes`, no terminal); integration tests for the
  refusal without `--yes`, the JSON output of install, list and verify, an upgrade that
  adds a privilege, a sync whose artifact declares more than its lock, and a run whose
  privileges were never confirmed.
- Refs: CRA I.2(d), I.2(j); least privilege.

**SR-31. `iohr sdk add` runs the project's package manager and nothing else.** It
chooses the manager from the project's own files (a lock file, `packageManager`, an
active virtualenv), prints the exact command, and runs that one program with an argument
vector in the project directory: no shell, never sudo, never pip outside an active
virtualenv. The program is looked up only in absolute `PATH` entries, so a file of that
name in the project is never run. `iohr` contacts no host for it; the package manager
reaches its own registry and writes what it always writes. Package names are fixed in
the binary and a version is checked to be one (digits, letters, `.`, `-`, `+`). With
`--dry-run` nothing runs.
- Why: installing a dependency is the person's decision on their machine; a command line
  that adds a shell, elevated rights or a global install would turn that into a way to
  run something else.
- Verify: unit tests over temporary directories for every manager, the virtualenv rule
  and refused versions; an integration test that runs a stand-in program and checks the
  arguments it received and the exit code passed on.
- Refs: CRA I.2(d); least privilege.

## Reliability and traceability

**SR-17. Request ids both ways.** Every request carries a client-generated request id
header; every result and every error exposes the client id and, when the API returns
one, the server's request id.
- Why: customers correlate SDK calls with their own audit logs and quote the id in
  support requests.
- Verify: unit test; conformance case.
- Refs: CRA I.2(l).

**SR-18. Idempotency before retrying writes.** A non-idempotent request (`POST`,
`PATCH`) is retried automatically only when its operation takes `Idempotency-Key` (the
platform does since RFC 0033) and the request carries one: the caller's, or one the SDK
generates once per logical call and reuses on every attempt. Any other `POST` or `PATCH`
is never retried automatically (ADR 0015, `config.md` section 7.5).
- Verify: conformance cases `a-write-is-not-retried`,
  `a-write-with-an-idempotency-key-is-retried`.
- Refs: CRA I.2(f).

**SR-19. Bounded time everywhere.** Every operation has a connect timeout and a total
deadline; the defaults are finite (design.md section 2). Streams have a connect timeout
and an idle timeout: no data and no keep-alive for 45 s ends the stream with a timeout
error. No configuration can set an infinite wait; the caller can only choose a longer
finite one.
- Verify: unit test against a server that never answers.
- Refs: CRA I.2(h), I.2(i).

## Supply chain

**SR-20. Minimal dependencies.** Runtime dependencies are limited to the lists in each
language's `AGENTS.md`. Each added dependency needs a reason in the pull request.
Lockfiles are committed for CI and examples.
- Verify: review; dependency review on every pull request.
- Refs: SSDF PW.4; CRA I.2(j); OSPS DO-06.

**SR-21. Known-vulnerability gate.** A release cannot ship with a known High or Critical
vulnerability in its dependencies unless a VEX statement explains why the SDK is not
affected. The check runs in CI (`govulncheck`, `cargo audit`, `pip-audit`, `npm audit`,
OSV-Scanner).
- Verify: CI.
- Refs: SSDF RV.1; CRA I.2(a), II(2); OSPS VM-05.

**SR-22. Reproducible builds.** Builds use the flags that make output depend only on
source and lockfile: Go `-trimpath`, Rust `--locked`, Python `SOURCE_DATE_EPOCH` for
sdist and wheel, npm packed from a clean checkout.
- Verify: CI rebuilds and compares digests (planned).
- Refs: SSDF PW.6, PS.2.

**SR-23. Every release is attested.** Each published artifact has build provenance and
a CycloneDX SBOM attached and attested, so users can check what they installed
([verifying-releases.md](verifying-releases.md)).
- Verify: release workflow; the verification guide is tested on each release.
- Refs: SSDF PS.2, PS.3; CRA II(1), II(7); SLSA Build L3 (target).
