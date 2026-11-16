#!/usr/bin/env bash
set -euo pipefail

check=${1:-scripts/check-banned-words}
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
retired="author""ship"

expect_rejected() {
  if output=$("$check" "$fixture" 2>&1) \
    || [[ "$output" != *'is forbidden'* ]]; then
    echo "check-banned-words accepted $1" >&2
    exit 1
  fi
}

mkdir -p "$fixture/crates/papo-proto/tests/data" "$fixture/.github"
printf '%s\n' 'pub enum Party { Me, Other, Unknown }' > "$fixture/lib.rs"
printf '%s\n' "\"Work\" shall mean the work of $retired" > "$fixture/LICENSE-APACHE"
printf '%s\n' "original works of $retired" > "$fixture/crates/papo-proto/tests/data/LICENSE"
"$check" "$fixture"

printf '%s\n' "if message.${retired^^} == Party::Me {}" > "$fixture/lib.rs"
expect_rejected 'the retired name in source'

printf '%s\n' 'pub enum Party { Me, Other, Unknown }' > "$fixture/lib.rs"
printf '%s\n' "run: echo $retired" > "$fixture/.github/ci.yml"
expect_rejected 'the retired name in a hidden directory'

if output=$("$check" "$fixture/missing" 2>&1); then
  echo 'check-banned-words passed a tree it could not search' >&2
  exit 1
fi
