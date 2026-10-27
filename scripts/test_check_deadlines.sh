#!/usr/bin/env bash
set -euo pipefail

check=${1:-scripts/check-deadlines}
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/crates/papo-core/src" "$fixture/crates/papo/src"

printf '%s\n' 'fn timer(now: Instant, duration: Duration) { let _ = crate::deadline(now, duration); }' \
  > "$fixture/crates/papo-core/src/lib.rs"
"$check" "$fixture"

printf '%s\n' 'fn timer(now: Instant) { let _ = now + Duration::from_secs(1); }' \
  > "$fixture/crates/papo-core/src/lib.rs"
if output=$("$check" "$fixture" 2>&1) \
  || [[ "$output" != *'raw monotonic deadline arithmetic is forbidden'* ]]; then
  echo 'check-deadlines accepted raw Instant addition' >&2
  exit 1
fi
