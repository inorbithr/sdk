#!/bin/sh
# Install iohr, the InOrbit command line, for this user (platform RFC 0021, ADR 0010).
#
#   curl -fsSL https://packages.inorbit.hr/install.sh | sh
#
# What it does, and nothing else:
#   1. picks the archive for this machine (Linux or macOS, x86-64 or arm64);
#   2. downloads it and the release's SHA256SUMS from the GitHub release over HTTPS;
#   3. refuses unless the archive's SHA-256 matches SHA256SUMS, and, when the GitHub CLI
#      is installed and signed in, unless `gh attestation verify` confirms this
#      repository's release workflow built it;
#   4. copies `iohr` into ~/.local/bin (no sudo). It changes no shell profile: if that
#      directory is not on your PATH, it says so.
#
# Settings, all optional:
#   IOHR_VERSION=0.1.0-alpha.1   a version (default: the newest iohr release)
#   IOHR_INSTALL_DIR=/some/dir   where to put the binary (default: ~/.local/bin)
set -eu

repo="inorbithr/sdk"
dir="${IOHR_INSTALL_DIR:-$HOME/.local/bin}"
version="${IOHR_VERSION:-}"

say() { printf 'iohr install: %s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

if have curl; then
  fetch() { curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"; }
elif have wget; then
  fetch() { wget --https-only -q "$1" -O "$2"; }
else
  die "needs curl or wget"
fi
if have sha256sum; then
  sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif have shasum; then
  sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
  die "needs sha256sum or shasum"
fi
have tar || die "needs tar"

case "$(uname -s)" in
  Linux) os="unknown-linux-musl" ;;
  Darwin) os="apple-darwin" ;;
  *) die "this installer is for Linux and macOS; on Windows use install.ps1 or winget" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  aarch64 | arm64) arch="aarch64" ;;
  *) die "no build for $(uname -m)" ;;
esac
# A shell under Rosetta reports x86_64 on Apple silicon; take the native build.
if [ "$os" = "apple-darwin" ] && [ "$arch" = "x86_64" ] &&
  [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
  arch="aarch64"
fi
target="$arch-$os"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -z "$version" ]; then
  say "finding the newest iohr release"
  fetch "https://api.github.com/repos/$repo/releases?per_page=100" "$tmp/releases.json"
  version=$(sed -n 's/.*"tag_name": *"iohr\/v\([^"]*\)".*/\1/p' "$tmp/releases.json" | head -n 1)
  [ -n "$version" ] || die "no iohr release found"
fi
case "$version" in
  *[!0-9A-Za-z.-]*) die "not a version: $version" ;;
esac

file="iohr-$version-$target.tar.gz"
base="https://github.com/$repo/releases/download/iohr/v$version"
say "installing iohr $version for $target into $dir"
say "downloading $base/$file"
fetch "$base/$file" "$tmp/$file"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"

want=$(awk -v f="$file" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
[ -n "$want" ] || die "$file is not in SHA256SUMS"
got=$(sha256 "$tmp/$file")
[ "$got" = "$want" ] || die "checksum mismatch for $file (expected $want, got $got); nothing was installed"
say "checksum ok ($got)"

if have gh && gh auth status >/dev/null 2>&1; then
  gh attestation verify "$tmp/$file" --repo "$repo" >/dev/null ||
    die "gh attestation verify failed for $file; nothing was installed"
  say "attestation ok (built by $repo's release workflow)"
else
  say "attestation not checked (needs the GitHub CLI, signed in); the checksum was"
fi

tar -xzf "$tmp/$file" -C "$tmp"
mkdir -p "$dir"
cp "$tmp/iohr-$version-$target/iohr" "$dir/iohr.new"
chmod 755 "$dir/iohr.new"
mv "$dir/iohr.new" "$dir/iohr"
say "installed $dir/iohr"
"$dir/iohr" --version >&2

case ":$PATH:" in
  *":$dir:"*) ;;
  *) say "$dir is not on your PATH; add it in your shell's profile, for example: export PATH=\"$dir:\$PATH\"" ;;
esac
say "next: iohr login"
