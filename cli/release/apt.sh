#!/usr/bin/env bash
# The iohr APT repository (platform RFC 0021, ADR 0010).
#
#   cli/release/apt.sh             build dist/apt/ from the .debs in dist/cli/, signed
#   cli/release/apt.sh --publish   build, then upload it to the packages bucket with the
#                                public key (iohr.gpg) and the installers
#
# The repository is static: dists/stable/main for amd64 and arm64, built from scratch by
# reprepro each release with the newest iohr (apt installs the newest anyway; older
# package files stay in the bucket's pool/ for anyone who pinned one). Signed with the
# repository's signing subkey (APT_SIGNING_KEY, APT_SIGNING_KEY_ID, APT_SIGNING_PASSPHRASE)
# in a throwaway GNUPGHOME, and checked against cli/install/iohr.gpg before upload.
# Publishing needs R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_ENDPOINT (Cloudflare R2's
# S3 API) and uploads to the bucket behind https://packages.inorbit.hr.
set -euo pipefail

publish=false
[ "${1:-}" = "--publish" ] && publish=true
root=$(git rev-parse --show-toplevel)
bucket="${IOHR_PACKAGES_BUCKET:-inorbit-packages}"
for tool in reprepro gpg gpgv; do
  command -v "$tool" >/dev/null || { echo "cli-apt: needs $tool" >&2; exit 1; }
done
: "${APT_SIGNING_KEY:?cli-apt: APT_SIGNING_KEY is not set}"
: "${APT_SIGNING_KEY_ID:?cli-apt: APT_SIGNING_KEY_ID is not set}"
: "${APT_SIGNING_PASSPHRASE:?cli-apt: APT_SIGNING_PASSPHRASE is not set}"

GNUPGHOME=$(mktemp -d)
export GNUPGHOME
chmod 700 "$GNUPGHOME"
repo="$root/dist/apt"
cleanup() { gpgconf --kill gpg-agent 2>/dev/null || true; rm -rf "$GNUPGHOME"; }
trap cleanup EXIT

# The secret subkey (its primary key is a stub: certifying stays offline).
printf '%s\n' "$APT_SIGNING_KEY" | gpg --batch --quiet --import

rm -rf "$repo"
mkdir -p "$repo/conf"
cat >"$repo/conf/distributions" <<CONF
Origin: InOrbit
Label: InOrbit packages
Suite: stable
Codename: stable
Architectures: amd64 arm64
Components: main
Description: InOrbit command line (iohr)
SignWith: ! $root/cli/release/apt-sign.sh
CONF

shopt -s nullglob
debs=("$root"/dist/cli/*.deb)
[ ${#debs[@]} -gt 0 ] || { echo "cli-apt: no .deb in dist/cli/" >&2; exit 1; }
for deb in "${debs[@]}"; do
  reprepro -b "$repo" --export=never includedeb stable "$deb"
done
reprepro -b "$repo" export stable

# The signature must check against the key users are told to fetch.
gpgv --keyring "$root/cli/install/iohr.gpg" "$repo/dists/stable/InRelease"
gpgv --keyring "$root/cli/install/iohr.gpg" "$repo/dists/stable/Release.gpg" "$repo/dists/stable/Release"
rm -rf "$repo/conf" "$repo/db"
cp "$root/cli/install/iohr.gpg" "$root/cli/install/install.sh" "$root/cli/install/install.ps1" "$root/dist/"
find "$repo" -type f | sort

$publish || exit 0

: "${R2_ACCESS_KEY_ID:?cli-apt: R2_ACCESS_KEY_ID is not set}"
: "${R2_SECRET_ACCESS_KEY:?cli-apt: R2_SECRET_ACCESS_KEY is not set}"
: "${R2_ENDPOINT:?cli-apt: R2_ENDPOINT is not set}"
command -v aws >/dev/null || { echo "cli-apt: needs the AWS CLI (S3 API)" >&2; exit 1; }
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION=auto AWS_EC2_METADATA_DISABLED=true
# R2 answers the CLI's default extra checksums inconsistently; ask for them only when needed.
export AWS_REQUEST_CHECKSUM_CALCULATION=when_required AWS_RESPONSE_CHECKSUM_VALIDATION=when_required
s3() { aws --endpoint-url "$R2_ENDPOINT" s3 "$@"; }

# Package files first, then the indexes that name them, InRelease last: a client never
# sees an index pointing at a file that is not there yet.
s3 cp --recursive "$repo/pool" "s3://$bucket/apt/pool" --cache-control "public, max-age=31536000, immutable"
s3 cp --recursive "$repo/dists" "s3://$bucket/apt/dists" --exclude "*InRelease" --cache-control "public, max-age=60"
s3 cp "$repo/dists/stable/InRelease" "s3://$bucket/apt/dists/stable/InRelease" --cache-control "public, max-age=60"
for f in iohr.gpg install.sh install.ps1; do
  s3 cp "$root/dist/$f" "s3://$bucket/$f" --cache-control "public, max-age=300"
done
echo "cli-apt: published to https://packages.inorbit.hr/apt"
