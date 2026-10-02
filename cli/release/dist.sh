#!/usr/bin/env bash
# Build `iohr` for one target and package it as a release archive (ADR 0010).
#
#   cli/release/dist.sh <target>        e.g. x86_64-unknown-linux-musl
#
# Writes to dist/cli/:
#   iohr-<version>-<target>.tar.gz     Linux and macOS (.zip on Windows): the binary,
#                                      LICENSE, NOTICE, README.md, completions/
#   iohr_<version>_<arch>.deb          Linux targets only (cargo-deb)
#   iohr-<version>-<target>.msi        Windows targets only (WiX 5, cli/packaging/iohr.wxs)
#
# Linux targets link statically against musl and build in a pinned Alpine image, so one
# binary runs on every distribution; the runner's architecture must match the target's.
# macOS and Windows build natively. Run through `mise run cli:dist <target>`.
#
# The MSI uses WiX 5.0.2, the last WiX release under its original MS-RL licence; WiX 6
# and later add a maintenance-fee EULA (ADR 0010).
set -euo pipefail

target="${1:?usage: cli/release/dist.sh <rust target triple>}"
root=$(git rev-parse --show-toplevel)
out="$root/dist/cli"
mkdir -p "$out"
cd "$root/cli"

# The image the Linux binaries are built in: Rust as pinned in mise.toml, on Alpine (musl).
alpine="rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f"

version=$(cargo pkgid -p iohr | sed 's/.*[#@]//')
name="iohr-$version-$target"
# Archives carry the commit's time, not the build's, so the same binary always packs to
# the same archive.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}"

echo "cli-dist: building iohr $version for $target"
case "$target" in
  *-linux-musl)
    docker run --rm \
      -v "$root:/src" -w /src/cli \
      -e SOURCE_DATE_EPOCH \
      "$alpine" sh -euc "
        apk add --no-cache -q musl-dev cmake make perl gcc g++ >/dev/null
        cargo build --locked --release -p iohr --target '$target'
        chown -R $(id -u):$(id -g) target"
    exe="iohr"
    ;;
  *)
    rustup target add "$target" >/dev/null
    cargo build --locked --release -p iohr --target "$target"
    exe="iohr"
    case "$target" in *-windows-*) exe="iohr.exe" ;; esac
    ;;
esac
bin="target/$target/release/$exe"

# Completions, from the binary just built (the runner's architecture is the target's).
completions="target/completions"
mkdir -p "$completions"
"$bin" completion bash >"$completions/iohr.bash"
"$bin" completion zsh >"$completions/_iohr"
"$bin" completion fish >"$completions/iohr.fish"
"$bin" completion powershell >"$completions/_iohr.ps1"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name/completions"
cp "$bin" "$stage/$name/"
cp "$root/LICENSE" "$root/NOTICE" "$stage/$name/"
cp README.md "$stage/$name/README.md"
cp "$completions"/* "$stage/$name/completions/"

case "$target" in
  *-windows-*)
    # PowerShell's archiver is on every Windows runner; Git Bash's tar cannot write zip.
    (cd "$stage" && powershell -NoProfile -Command "Compress-Archive -Path '$name' -DestinationPath '$name.zip'")
    mv "$stage/$name.zip" "$out/"
    # The installer: WiX 5 as a .NET tool in a throwaway tool path, never a global install.
    wix="$stage/wix-tool"
    dotnet tool install --tool-path "$wix" wix --version 5.0.2 >/dev/null
    case "$target" in x86_64-*) arch=x64 ;; aarch64-*) arch=arm64 ;; esac
    (cd "$stage" && "$wix/wix" build -arch "$arch" \
      -d "IOHR_VERSION=${version%%-*}" -d "IOHR_STAGE=$name" \
      -o "$name.msi" "$root/cli/packaging/iohr.wxs")
    mv "$stage/$name.msi" "$out/"
    ;;
  *-apple-*)
    (cd "$stage" && tar -czf "$out/$name.tar.gz" "$name")
    ;;
  *)
    (cd "$stage" && tar --sort=name --owner=0 --group=0 --numeric-owner \
      --mtime="@$SOURCE_DATE_EPOCH" -cf - "$name" | gzip -n >"$out/$name.tar.gz")
    # The Debian package: the static binary, completions, licence (crates/iohr/Cargo.toml
    # [package.metadata.deb]). No build here; cargo-deb packages what was built above.
    cargo deb -p iohr --target "$target" --no-build --no-strip --output "$out/"
    ;;
esac

ls -l "$out"
