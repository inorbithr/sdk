# Verifying extensions

What `iohr ext install` checks before an extension is used, how to run the same checks
by hand, and how a company serves extensions from its own mirror. The decision is
[ADR 0012](../adr/0012-extensions.md); the requirements are SR-25 to SR-28 and SR-32 in
[requirements.md](requirements.md).

## What is published

Each extension version is an OCI image index at
`ghcr.io/inorbithr/iohr-ext/<name>:<version>`, with one image manifest per platform
(`linux/amd64`, `linux/arm64`, `darwin/arm64`, `darwin/amd64`, `windows/amd64`,
`windows/arm64`). Each manifest has:

- a config blob, media type `application/vnd.inorbit.iohr.extension.config.v1+json`:
  `name`, `version`, `entrypoint`, `scopes`, `description` and, for an extension that
  controls a system service, `privileges`: the Linux capabilities that service holds,
  such as `["CAP_BPF", "CAP_PERFMON", "CAP_NET_ADMIN"]`;
- one layer, `application/vnd.inorbit.iohr.extension.layer.v1.tar+gzip`, holding the
  program at `entrypoint`.

Attached to the index as OCI 1.1 referrers, each a Sigstore bundle
(`application/vnd.dev.sigstore.bundle.v0.3+json`):

- a signature by the release workflow (cosign, keyless);
- SLSA v1 build provenance from the same workflow run.

## What `iohr ext install` checks

In this order, and nothing is written to disk until all of them pass:

1. The index and this platform's manifest match their SHA-256 digests (and the digest
   in the lock, for `iohr ext sync`).
2. The bundles. A bundle counts only if it is a DSSE in-toto statement whose subject is
   the digest of the index or of this platform's manifest, and:
   - its certificate chains to Sigstore's Fulcio root, carries a valid SCT, was valid
     when the entry was logged, names the identity
     `https://github.com/inorbithr/<repo>/.github/workflows/release.yml@refs/tags/<tag>`
     and the issuer `https://token.actions.githubusercontent.com`;
   - its Rekor entry has a valid inclusion proof and checkpoint and matches the bundle;
   - the signature over the statement verifies with the certificate's key.

   Both a signature and an SLSA provenance must pass, from the same repository, and the
   provenance must name that repository, `.github/workflows/release.yml` and a tag.
   The Sigstore trusted root is built into `iohr`; nothing is fetched to check it.
   Alternatively, a signature by a key in `ext.trusted_keys` passes (see below).
3. The config: the name and version are the ones asked for, the entrypoint is a
   relative path without `..`, every scope is `resource:action`, every privilege is a
   capability the kernel defines (at most 16, none twice).
4. The layer matches its digest and size; only the entrypoint is taken from it, and
   only as a regular file.

For `iohr ext install PUBLISHER/NAME`, the catalogue on the API host names the version,
its index digest and its signer first, and the index is fetched by that digest. After
checks 1 to 4 the artifact must match the listing: the same version, exactly the signer
the catalogue names (which must be the listing's signing identity, a release workflow at
any tag when the identity ends in `@refs/tags/`), and the same scopes and privileges.
A mismatch fails the install. The catalogue adds a condition; it never lets through
anything checks 1 to 4 refuse.

Then, for an extension that declares privileges, the confirmation (SR-32): each
privilege is shown in plain words, and the extension is installed only after you type
`yes` or pass `--yes`. `iohr` never grants a privilege; the service's own package or
unit does. The lock records what you confirmed, and `iohr ext sync` refuses an artifact
that declares more.

`iohr ext list` shows what is installed with its digest, privileges and signer.
`iohr ext verify` repeats checks 2 and the program's hash offline, from the bundles kept
at install. Every run of an extension re-hashes the program first.

## Checking by hand

With [crane](https://github.com/google/go-containerregistry), [cosign](https://docs.sigstore.dev/)
3 and the [GitHub CLI](https://cli.github.com/):

```sh
ref=ghcr.io/inorbithr/iohr-ext/agent
digest=$(crane digest "$ref:0.1.0")                 # the index digest iohr-ext.lock pins
crane manifest "$ref@$digest"                       # the platforms

cosign verify "$ref@$digest" \
  --certificate-identity-regexp '^https://github\.com/inorbithr/[^/]+/\.github/workflows/release\.yml@refs/tags/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com

gh attestation verify "oci://$ref@$digest" --owner inorbithr \
  --predicate-type https://slsa.dev/provenance/v1
```

Compare `digest` with the `digest` field of your `iohr-ext.lock`.

## From a company's mirror

Point every machine at the mirror once (or set `IOHR_EXT_REGISTRY` in a pipeline):

```sh
iohr config set ext.registry registry.acme.hr/inorbit/iohr-ext
```

A mirror that copies artifacts with their referrers (`oras cp -r`, or a proxy cache
that keeps referrers) needs nothing else: the release workflow's signature and
provenance travel with the artifact and are checked as above.

A mirror that re-signs what it admits, or an internal channel, adds its public key:

```sh
iohr config set ext.trusted_keys mirror.pub          # one or more PEM public keys
iohr config get ext.trusted_keys                     # prints their fingerprints
```

Its signature must be a Sigstore bundle (v0.3) holding a DSSE in-toto statement whose
subject is the index or manifest digest, attached as a referrer; `cosign sign --key`
in cosign 3 writes this format. When the bundle carries a Rekor entry it is checked;
a private channel without a transparency log is checked by the signature alone. The
lock then records the signer as `key:sha256:<fingerprint>`.

For a mirror that needs a login, `IOHR_EXT_REGISTRY_AUTH=user:password` is sent to that
registry and the token service it names, and nowhere else. It is never a flag.

## What is not checked

- Whether the release workflow's code is what you expect: the provenance names the
  repository, workflow and tag, which you can read.
- Revocation: Sigstore has no certificate revocation; a compromised release is answered
  with a new release and an advisory (`SECURITY.md`).
- The SBOM and OpenVEX attached to a release (RFC 0028) are published for your scanners;
  `iohr` does not read them.
