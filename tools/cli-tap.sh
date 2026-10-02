#!/usr/bin/env bash
# Propose the Homebrew formula for an iohr release to inorbithr/homebrew-tap as a pull
# request (platform RFC 0021, ADR 0010). Uses the GitHub API only, so nothing is pushed
# with a stored credential: GH_TOKEN is a short-lived GitHub App token limited to the tap.
#
#   GH_TOKEN=... tools/cli-tap.sh <version> <SHA256SUMS>
set -euo pipefail
shopt -s inherit_errexit

version="${1:?usage: cli-tap.sh <version> <SHA256SUMS>}"
sums="${2:?usage: cli-tap.sh <version> <SHA256SUMS>}"
: "${GH_TOKEN:?cli-tap: GH_TOKEN (a token for inorbithr/homebrew-tap) is not set}"
root=$(git rev-parse --show-toplevel)
tap="inorbithr/homebrew-tap"
branch="iohr-$version"
path="Formula/iohr.rb"

formula=$(bash "$root/tools/cli-formula.sh" "$version" "$sums")
base=$(gh api "repos/$tap/git/ref/heads/main" --jq .object.sha)
if ! gh api "repos/$tap/git/ref/heads/$branch" >/dev/null 2>&1; then
  gh api -X POST "repos/$tap/git/refs" -f ref="refs/heads/$branch" -f sha="$base" >/dev/null
fi
# The current file's blob, when there is one, so the API replaces it.
existing=$(gh api "repos/$tap/contents/$path?ref=$branch" --jq .sha 2>/dev/null || true)
args=(-X PUT "repos/$tap/contents/$path" -f branch="$branch"
  -f message="iohr $version" -f content="$(printf '%s\n' "$formula" | base64 | tr -d '\n')")
[ -n "$existing" ] && args+=(-f sha="$existing")
gh api "${args[@]}" >/dev/null

if [ -z "$(gh pr list --repo "$tap" --head "$branch" --json number --jq '.[].number')" ]; then
  gh pr create --repo "$tap" --head "$branch" --base main --title "iohr $version" \
    --body "The formula for [iohr $version](https://github.com/inorbithr/sdk/releases/tag/iohr/v$version), written by inorbithr/sdk's release workflow from the release's SHA256SUMS. CI installs and tests it on macOS and Linux."
fi
echo "cli-tap: proposed iohr $version to $tap"
