#!/usr/bin/env bash
set -euo pipefail

check=${1:-"$(dirname "$0")/check-cfg"}
check=$(cd "$(dirname "$check")" && pwd)/$(basename "$check")
fixture=$(cd "$(dirname "$0")/fixtures/cfg" && pwd)
repo=$(mktemp -d)
trap 'rm -rf "$repo"' EXIT
cp -R "$fixture/." "$repo"

"$BASH" "$check" "$repo"

printf '%s\n' '#[cfg(feature = "extra")]' 'pub fn extra() {}' >>"$repo/crates/sample/src/lib.rs"
if output=$("$BASH" "$check" "$repo" 2>&1) || [[ "$output" != *"cfg count rose from 1 to 2"* ]]; then
  echo 'cfg budget increase was not rejected' >&2
  exit 1
fi

printf '%s\n' 'sample 5' >"$repo/scripts/cfg-budget.txt"
for name in one two three; do
  printf '%s\n' '#[cfg(feature = "extra")]' "pub fn $name() {}" >>"$repo/crates/sample/src/lib.rs"
done
if output=$("$BASH" "$check" "$repo" 2>&1) || [[ "$output" != *'repeats #[cfg(feature = "extra")] 4 times'* ]]; then
  echo 'repeated cfg predicate was not rejected' >&2
  exit 1
fi

printf '%s\n' '#[doc(cfg(feature = "fixture"))]' >>"$repo/crates/sample/src/lib.rs"
if output=$("$BASH" "$check" "$repo" 2>&1) || [[ "$output" != *'manual doc(cfg) attributes are forbidden'* ]]; then
  echo 'manual doc cfg was not rejected' >&2
  exit 1
fi

mkdir -p "$repo/crates/papo/src"
printf '%s\n' '#[cfg(target_os = "wasi")]' >"$repo/crates/papo/src/lib.rs"
if output=$("$BASH" "$check" "$repo" 2>&1) || [[ "$output" != *'raw WASI target predicates are restricted'* ]]; then
  echo 'raw WASI target predicate was not rejected' >&2
  exit 1
fi
