#!/usr/bin/env bash
# The Homebrew formula for an iohr release (platform RFC 0021, ADR 0010).
#
#   tools/cli-formula.sh <version> <SHA256SUMS> > Formula/iohr.rb
#
# Prebuilt archives only, one per platform and architecture, each pinned by its SHA-256
# from the release's SHA256SUMS: nothing is compiled and nothing runs at install time
# beyond copying the binary and its completions.
set -euo pipefail
shopt -s inherit_errexit
version="${1:?usage: cli-formula.sh <version> <SHA256SUMS>}"
sums="${2:?usage: cli-formula.sh <version> <SHA256SUMS>}"
base="https://github.com/inorbithr/sdk/releases/download/iohr/v$version"

sha() {
  local file="iohr-$version-$1.tar.gz" sum
  sum=$(awk -v f="$file" '$2 == f || $2 == "*" f { print $1 }' "$sums")
  [ -n "$sum" ] || { echo "cli-formula: $file is not in $sums" >&2; exit 1; }
  echo "$sum"
}
block() {
  local sum
  sum=$(sha "$2")
  printf '    on_%s do\n      url "%s/iohr-%s-%s.tar.gz"\n      sha256 "%s"\n    end\n' \
    "$1" "$base" "$version" "$2" "$sum"
}

# Computed before the heredoc, so a missing checksum stops the script.
mac_arm=$(block arm aarch64-apple-darwin)
mac_intel=$(block intel x86_64-apple-darwin)
linux_arm=$(block arm aarch64-unknown-linux-musl)
linux_intel=$(block intel x86_64-unknown-linux-musl)

cat <<RUBY
# Written by inorbithr/sdk tools/cli-formula.sh for iohr $version; regenerated on every
# release, not edited by hand.
class Iohr < Formula
  desc "InOrbit command line: sign in, keep several accounts, call the API"
  homepage "https://docs.inorbit.hr"
  version "$version"
  license "Apache-2.0"

  on_macos do
$mac_arm
$mac_intel
  end

  on_linux do
$linux_arm
$linux_intel
  end

  def install
    bin.install "iohr"
    bash_completion.install "completions/iohr.bash" => "iohr"
    zsh_completion.install "completions/_iohr"
    fish_completion.install "completions/iohr.fish"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/iohr --version")
  end
end
RUBY
