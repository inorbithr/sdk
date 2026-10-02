#!/usr/bin/env bash
# reprepro's SignWith hook for the iohr APT repository (ADR 0010, cli/release/apt.sh).
# reprepro calls it with: $1 the unsigned Release, $2 the InRelease to write, $3 the
# Release.gpg to write (either may be empty). The passphrase goes to gpg on stdin with
# loopback pinentry; it is never an argument, a file or a log line.
set -euo pipefail
key="${APT_SIGNING_KEY_ID:?}!"
sign() {
  printf '%s' "${APT_SIGNING_PASSPHRASE:?}" |
    gpg --batch --yes --quiet --pinentry-mode loopback --passphrase-fd 0 \
      --local-user "$key" --digest-algo SHA512 "$@"
}
if [ -n "${2:-}" ]; then sign --clearsign --output "$2" "$1"; fi
if [ -n "${3:-}" ]; then sign --armor --detach-sign --output "$3" "$1"; fi
