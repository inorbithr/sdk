#!/usr/bin/env bash
# winget manifests for an iohr release (platform RFC 0021, ADR 0010), from the .msi files
# and SHA256SUMS in dist/cli/, written to dist/winget/ in winget-pkgs' layout and checked
# against winget's published JSON schemas. Submitting them to microsoft/winget-pkgs is a
# separate, credentialed step.
#
#   tools/cli-winget.sh <version>
set -euo pipefail
shopt -s inherit_errexit

version="${1:?usage: cli-winget.sh <version>}"
root=$(git rev-parse --show-toplevel)
cli="$root/dist/cli"
id="InOrbit.iohr"
schema_version="1.10.0"
dir="$root/dist/winget/manifests/i/InOrbit/iohr/$version"
base="https://github.com/inorbithr/sdk/releases/download/iohr/v$version"
command -v msiinfo >/dev/null || { echo "cli-winget: needs msiinfo (apt-get install msitools)" >&2; exit 1; }

installer() {
  local arch="$1" target="$2" file sum code
  file="iohr-$version-$target.msi"
  [ -f "$cli/$file" ] || { echo "cli-winget: $file is not in dist/cli/" >&2; exit 1; }
  sum=$(awk -v f="$file" '$2 == f || $2 == "*" f { print toupper($1) }' "$cli/SHA256SUMS")
  [ -n "$sum" ] || { echo "cli-winget: $file is not in SHA256SUMS" >&2; exit 1; }
  code=$(msiinfo export "$cli/$file" Property | awk -F'\t' '$1 == "ProductCode" { print $2 }' | tr -d '\r')
  [ -n "$code" ] || { echo "cli-winget: no ProductCode in $file" >&2; exit 1; }
  printf '  - Architecture: %s\n    InstallerUrl: %s/%s\n    InstallerSha256: %s\n    ProductCode: "%s"\n' \
    "$arch" "$base" "$file" "$sum" "$code"
}
x64=$(installer x64 x86_64-pc-windows-msvc)
arm64=$(installer arm64 aarch64-pc-windows-msvc)
released=$(git log -1 --format=%cs)

mkdir -p "$dir"
header="# yaml-language-server: \$schema=https://aka.ms/winget-manifest"
cat >"$dir/$id.yaml" <<YAML
$header.version.$schema_version.schema.json
PackageIdentifier: $id
PackageVersion: $version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: $schema_version
YAML
cat >"$dir/$id.installer.yaml" <<YAML
$header.installer.$schema_version.schema.json
PackageIdentifier: $id
PackageVersion: $version
InstallerType: msi
Scope: machine
UpgradeBehavior: install
Commands:
  - iohr
ReleaseDate: $released
Installers:
$x64
$arm64
ManifestType: installer
ManifestVersion: $schema_version
YAML
cat >"$dir/$id.locale.en-US.yaml" <<YAML
$header.defaultLocale.$schema_version.schema.json
PackageIdentifier: $id
PackageVersion: $version
PackageLocale: en-US
Publisher: InOrbit d.o.o.
PublisherUrl: https://inorbit.hr
PublisherSupportUrl: https://github.com/inorbithr/sdk/issues
PackageName: iohr
PackageUrl: https://docs.inorbit.hr
License: Apache-2.0
LicenseUrl: https://github.com/inorbithr/sdk/blob/main/LICENSE
ShortDescription: The InOrbit command line. Sign in, keep several accounts, call the API.
Moniker: iohr
Tags:
  - cli
  - api
  - inorbit
ReleaseNotesUrl: https://github.com/inorbithr/sdk/releases/tag/iohr/v$version
ManifestType: defaultLocale
ManifestVersion: $schema_version
YAML

# winget's own schemas, from the winget-cli repository.
schemas="https://raw.githubusercontent.com/microsoft/winget-cli/master/schemas/JSON/manifests/v$schema_version"
uvx --quiet check-jsonschema==0.38.2 --schemafile "$schemas/manifest.version.$schema_version.json" "$dir/$id.yaml"
uvx --quiet check-jsonschema==0.38.2 --schemafile "$schemas/manifest.installer.$schema_version.json" "$dir/$id.installer.yaml"
uvx --quiet check-jsonschema==0.38.2 --schemafile "$schemas/manifest.defaultLocale.$schema_version.json" "$dir/$id.locale.en-US.yaml"
find "$dir" -type f | sort
