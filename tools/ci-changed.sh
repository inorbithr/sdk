#!/usr/bin/env bash
# `mise run ci:changed`: what CI would run for the changes since origin/main (committed,
# staged, unstaged and untracked), with CI's path rules (the `changes` job in
# .github/workflows/ci.yml). `--dry-run` prints the plan without running it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

dry=0
[ "${1:-}" = "--dry-run" ] && dry=1
base=$(git merge-base HEAD "${CI_CHANGED_BASE:-origin/main}")
mapfile -t files < <({ git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u)

touched() { printf '%s\n' "${files[@]}" | grep -Eq "$1"; }

# A change here regenerates or re-checks every language (ADR 0011).
shared='^(spec/|conformance/|mise\.toml$|\.github/workflows/ci\.yml$|cli/crates/iohr-(openapi|codegen)/)'
declare -A dir=([go]=go [rust]=rust [ts]=typescript [py]=python [java]=java [csharp]=csharp [swift]=swift)
langs=()
for lang in go rust ts py java csharp swift; do
  extra=''
  [ "$lang" = swift ] && extra='|^Package\.swift$'
  if touched "$shared" || touched "^(${dir[$lang]}|examples/${dir[$lang]})/$extra"; then
    langs+=("$lang")
  fi
done

tasks=(repo:check)
touched '^spec/' && tasks+=(spec:lint)
touched '^conformance/' && tasks+=(conformance:validate conformance:server:check)
for lang in "${langs[@]}"; do
  tasks+=("$lang:gen" "gen-diff:${dir[$lang]}" "$lang:check" "conformance:$lang")
done
touched '^(cli/|rust/|spec/|mise\.toml$|\.github/workflows/ci\.yml$)' && tasks+=(cli:check)
touched '^examples/' && tasks+=(examples:check)

echo "ci:changed: ${#files[@]} changed file(s) since $(git rev-parse --short "$base")"
echo "ci:changed: ${tasks[*]}"
[ "$dry" = 1 ] && exit 0

# Build the command line once for every <lang>:gen instead of once per task.
if [ "${#langs[@]}" -gt 0 ] && [ -z "${IOHR_BIN:-}" ]; then
  cargo build -q --manifest-path cli/Cargo.toml -p iohr
  target=$(cargo metadata -q --format-version 1 --no-deps --manifest-path cli/Cargo.toml |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
  IOHR_BIN="$target/debug/iohr"
  export IOHR_BIN
fi

for task in "${tasks[@]}"; do
  case "$task" in
    gen-diff:*)
      d=${task#gen-diff:}
      git diff --exit-code -- "$d" || {
        echo "ci:changed: regenerating $d changed committed files; commit the result" >&2
        exit 1
      }
      ;;
    *) mise run "$task" ;;
  esac
done
echo "ci:changed: all ${#tasks[@]} step(s) passed"
