#!/usr/bin/env bash
# `mise run cli:preview-install`: build iohr from this checkout and install it as
# `iohr-preview`, signed the way the released iohr is (an Apple Development or Developer ID
# certificate, identifier hr.inorbit.iohr), so the macOS Keychain trusts every rebuild
# alike instead of refusing or asking again after each one (an ad-hoc build is a new code
# identity every time).
#
# The signing is the one `iohr upgrade` does (cli/crates/iohr/src/commands/upgrade/sign.rs),
# reached through `iohr sign-binary`: the identity lookup, the codesign call and the
# identifier live there and nowhere else. This script builds, hands the copy over to it,
# checks the result and swaps it in.
#
#   IOHR_PREVIEW_DIR  where iohr-preview goes (default ~/.local/bin)
#   IOHR_PREVIEW_BIN  an iohr already built, installed instead of building (a test hook)
#   IOHR_SIGN_IDENTITY  an identity for `codesign --sign`; unset is the first Developer ID
#                       Application, else the first Apple Development one in the keychain
set -euo pipefail

identifier=hr.inorbit.iohr
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
dir=${IOHR_PREVIEW_DIR:-$HOME/.local/bin}
dest=$dir/iohr-preview

[ "$(uname -s)" = Darwin ] || {
  echo "cli:preview-install: the Keychain trust it fixes is macOS's; nothing to do here" >&2
  exit 1
}

if [ -n "${IOHR_PREVIEW_BIN:-}" ]; then
  built=$IOHR_PREVIEW_BIN
else
  cargo build --release --locked --manifest-path "$root/cli/Cargo.toml" -p iohr
  built=${CARGO_TARGET_DIR:-$root/cli/target}/release/iohr
fi
[ -x "$built" ] || { echo "cli:preview-install: no binary at $built" >&2; exit 1; }

mkdir -p "$dir"
# Sign a copy beside the destination: a failed signing leaves the installed one as it is.
staged=$(mktemp "$dir/.iohr-preview.XXXXXX")
trap 'rm -f "$staged"' EXIT
cp "$built" "$staged"
chmod 755 "$staged"
"$built" sign-binary "$staged"

# What the Keychain will see: the identifier and a team, not an ad-hoc hash.
details=$(codesign -dvv "$staged" 2>&1) || {
  echo "cli:preview-install: codesign cannot read the signature of the signed copy $staged: $details" >&2
  exit 1
}
printf '%s\n' "$details" | grep -qx "Identifier=$identifier" || {
  echo "cli:preview-install: the signed copy's identifier is not $identifier" >&2
  exit 1
}
printf '%s\n' "$details" | grep -q '^Authority=' || {
  echo "cli:preview-install: the signed copy has no certificate (still ad hoc)" >&2
  exit 1
}
team=$(printf '%s\n' "$details" | sed -n 's/^TeamIdentifier=//p') || {
  echo "cli:preview-install: cannot read the team identifier from the signed copy" >&2
  exit 1
}
{ [ -n "$team" ] && [ "$team" != "not set" ]; } || {
  echo "cli:preview-install: the signed copy has no team identifier" >&2
  exit 1
}
codesign --verify --strict "$staged"

# Never cp over a running or installed binary (macOS kills the process that next runs it):
# rename the new file over it, which replaces the directory entry in one step.
mv -f "$staged" "$dest"
trap - EXIT

codesign -dv "$dest" 2>&1 | grep -E '^(Identifier|TeamIdentifier)='
echo "cli:preview-install: $dest ($("$dest" --version))"
