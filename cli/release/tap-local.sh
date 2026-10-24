#!/usr/bin/env bash
# Propose an already published iohr release's Homebrew formula from this machine, for when
# the release workflow's `homebrew tap` job cannot run (GitHub Actions paused, the
# `release-cli` approval never given, a run cancelled). The same formula.sh and tap.sh the
# workflow uses; only where they run differs.
#
#   cli/release/tap-local.sh <version> [--dry-run]
#
# Needs the release's assets: SHA256SUMS comes from the GitHub release (`gh release
# download`), and a release without it (its `publish iohr` job never ran) stops here with
# that said, since the formula pins every archive by its checksum. GH_TOKEN, when unset,
# is `gh auth token`; it needs contents and pull-request write on inorbithr/homebrew-tap.
# `--dry-run` prints the formula and changes nothing.
set -euo pipefail
shopt -s inherit_errexit

version="${1:?usage: tap-local.sh <version> [--dry-run]}"
dry="${2:-}"
root=$(git rev-parse --show-toplevel)
tag="iohr/v$version"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT

if ! gh release download "$tag" --repo inorbithr/sdk --pattern SHA256SUMS --dir "$dir" 2>/dev/null; then
  echo "tap-local: release $tag has no SHA256SUMS. Its publish job (environment release-cli) has not" >&2
  echo "           run, so there are no archives to pin. Approve and run that job first." >&2
  exit 1
fi
if [ "$dry" = "--dry-run" ]; then
  bash "$root/cli/release/formula.sh" "$version" "$dir/SHA256SUMS"
  exit 0
fi
GH_TOKEN="${GH_TOKEN:-$(gh auth token)}" bash "$root/cli/release/tap.sh" "$version" "$dir/SHA256SUMS"
