# 0012. Extensions: signed OCI artifacts, run as separate processes

Status: accepted, 2026-10-03. Supersedes ADR 0009 point 5 for the command line; adds
SR-25 to SR-28. Implements platform RFC 0028 (open) for `iohr`.

## Context

ADR 0009 point 5 held the command line to SR-16: it talks to the API host and the
sign-in host and nothing else. Platform RFC 0028 proposes extensions, separate programs
such as the agent of RFC 0029 that `iohr` installs, verifies, pins and runs, shipped as
signed OCI artifacts from InOrbit's registry or a company's mirror. Fetching them means
one more host. Running them means a second program that needs to call the API without
holding the person's credentials.

What a bank or a regulated company asks of such a mechanism:

- nothing changes on the machine unless a person runs a command (no self-update);
- what runs is what InOrbit's release workflow built, checkable without trusting the
  registry or the mirror in between;
- a security team can review what is installed, and a pipeline can install exactly the
  same set;
- an extension cannot read the keychain entry, the refresh token or the command line's
  memory.

## Decision

1. **The registry exception (SR-25).** `iohr` contacts the extension registry only in
   `iohr ext install`, `iohr ext upgrade` and `iohr ext sync`, never on any other
   command and never to look for updates. The registry is
   `ghcr.io/inorbithr/iohr-ext` unless `ext.registry` (or `IOHR_EXT_REGISTRY`) names a
   mirror. Those commands talk to that registry, the token service its challenge names
   and the storage host a blob download redirects to; `GET` only, HTTPS only (plain HTTP
   to this machine for tests, as SR-07). A private mirror's credential comes from
   `IOHR_EXT_REGISTRY_AUTH` and is sent to that registry and its token service only.
   There is still no telemetry. SR-16 is otherwise unchanged.
2. **The artifact** is the contract of RFC 0028: an OCI image index per version at
   `<registry>/<name>:<version>`, one manifest per platform, config media type
   `application/vnd.inorbit.iohr.extension.config.v1+json` (`name`, `version`,
   `entrypoint`, `scopes`, `description`), one layer
   `application/vnd.inorbit.iohr.extension.layer.v1.tar+gzip` holding the program.
   Signatures and provenance are Sigstore bundles (v0.3) attached as OCI 1.1 referrers
   of the index or of the platform manifest, found through the referrers API or the
   referrers tag schema.
3. **Verified before anything is used (SR-26).** Every manifest and blob is checked
   against its SHA-256 digest and declared size. Then the bundles: by default the
   artifact needs a signature and SLSA v1 provenance, both from a certificate Fulcio
   issued to `https://github.com/inorbithr/<repo>/.github/workflows/release.yml@refs/tags/*`
   for the issuer `https://token.actions.githubusercontent.com`, each a DSSE in-toto
   statement whose subject is the artifact's digest, each with a verified Rekor entry
   and certificate chain, checked offline against the Sigstore trusted root built into
   `iohr` (sigstore-verify, from the sigstore project). The provenance must name the
   same repository, that workflow and a tag. A company may add public keys
   (`ext.trusted_keys`) for a mirror that re-signs and for an internal channel; a
   signature by one of them over a statement naming the digest is then accepted, with
   its Rekor entry checked when the bundle has one. No flag skips any of this.
4. **Pinned.** `iohr-ext.lock` (TOML) records name, version, index digest and signer
   (the workflow identity without the tag, or `key:sha256:...`). The machine keeps one
   in its data directory; `--lock FILE` writes one a team commits, and `iohr ext sync`
   installs exactly that, refusing a different digest, version or signer. Nothing
   updates by itself: `iohr ext upgrade` is the only way to a newer version.
5. **Separate processes and the token channel (SR-27).** `iohr <name> ...` re-hashes the
   installed program, then runs it as a child process. Before it starts, `iohr` opens a
   Unix socket, mode 0600 in a new directory of mode 0700 (on Windows a named pipe with
   a random name that refuses remote clients), and passes `IOHR_EXT_API` and
   `IOHR_EXT_TOKEN_SOCKET`. The extension sends `POST /token {"scopes": [...]}` and gets
   `{"access_token", "expires_at"}`. Scopes outside the manifest are refused. The child
   never sees `IOHR_TOKEN` or `IOHR_TOKEN_<PROFILE>`; the refresh token never leaves
   `iohr`. The socket and its directory are removed when the child exits.
6. **Narrower tokens are not available yet.** The sign-in service cannot mint a token
   narrower than a person's session from that session, so the channel hands out the
   profile's current access token (15 minutes, refreshed by `iohr`) only when it holds
   every scope asked for, and refuses otherwise. When the platform can exchange a token
   for a narrower one (RFC 8693), the channel returns that instead; the extension's
   side does not change.
7. **Install shows the scopes** an extension may ask for, and `upgrade` names any new
   ones, the way a phone shows an app's permissions. Only InOrbit publishes extensions
   for now (RFC 0028).

## Consequences

- New runtime dependencies, each with a reason in `cli/README.md`: `sigstore-verify`
  (Sigstore bundle verification and the embedded trusted root), `flate2` (gzip, pure
  Rust backend), `semver` (choosing the newest release). The OCI client is written here
  over the existing `reqwest`: the crates available pulled a licence outside
  `cli/deny.toml` and about 30 more crates, for four read-only calls.
- The trusted root is the one `sigstore-verify` embeds. A Sigstore key rotation needs an
  `iohr` release; until then installs fail closed. A later decision may load updated
  roots through TUF from the configured registry.
- The release workflow that publishes an extension must push the index, attach a cosign
  signature and an `actions/attest-build-provenance` attestation as referrers, both from
  `release.yml` at a tag. An artifact without both is refused.
- On Windows the token channel is a named pipe; its default access list lets other local
  users open it for reading only, not send a request. The Unix channel is tested on
  Linux in pull requests; the Windows code is first built and tested by the `main`
  matrix (ADR 0009), and the extension tests that run a program are Unix-only.

## Sources

- Platform RFC 0028, "iohr extensions" (inorbit.hr/lab, opened 2026-10-03).
- OCI image spec v1.1 (manifest `subject`, `artifactType`) and distribution spec v1.1
  (referrers API and tag schema), github.com/opencontainers.
- Sigstore bundle format v0.3 and client verification, docs.sigstore.dev;
  sigstore/sigstore-rust `sigstore-verify` 0.14.
- SLSA v1.1 provenance, slsa.dev; GitHub artifact attestations, docs.github.com.
- RFC 8693, OAuth 2.0 Token Exchange.
