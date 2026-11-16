#!/usr/bin/env bash
set -euo pipefail

check=$(realpath "${1:-scripts/check-example-api}")
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT

stable_main='use papo::{
    Client,
    Event,
};
use papo::sts::{self, StsPolicy as Policy};

async fn op(client: &Client, nick: &str) -> Result<(), papo::Error> {
    let reply_to__nick = nick;
    client.op("#papo", reply_to__nick).await
}'

expect_accepted() {
  if ! output=$("$check" "$fixture" 2>&1); then
    echo "check-example-api rejected $1: $output" >&2
    exit 1
  fi
}

expect_rejected() {
  local line=$1 reason=$2 file=${3:-ping-bot/src/main.rs}
  local source="$fixture/examples/$file"
  local original
  original=$(cat "$source")
  printf '%s\n%s\n' "$original" "$line" > "$source"
  if output=$("$check" "$fixture" 2>&1); then
    echo "check-example-api accepted: $line" >&2
    exit 1
  fi
  if [[ "$output" != *"$reason"* ]]; then
    echo "check-example-api did not say '$reason' for: $line" >&2
    exit 1
  fi
  printf '%s\n' "$original" > "$source"
}

mkdir -p "$fixture/examples/ping-bot/src" "$fixture/examples/ping-bot/tests"
printf '%s\n' "$stable_main" > "$fixture/examples/ping-bot/src/main.rs"
printf '%s\n' 'let casemapping = papo::Casemapping::default();' > "$fixture/examples/ping-bot/tests/output.rs"
expect_accepted 'public API and a test file'

expect_rejected 'let state = papo::__testing::exercise_session_mapping;' \
  "doc-hidden __-prefixed"
expect_rejected 'use papo::{Client, __is_known_cap};' \
  "doc-hidden __-prefixed"
expect_rejected 'let channel = state.__channel_local("#papo");' \
  "doc-hidden __-prefixed"
expect_rejected 'let origin = Origin::__new(7);' \
  "doc-hidden __-prefixed"
expect_rejected 'let nick = papo::Casemapping::default().nick("luiz");' \
  "Casemapping::default()"
expect_rejected 'let fold = Casemapping::default;' \
  "Casemapping::default()"

chmod 000 "$fixture/examples/ping-bot/src/main.rs"
if output=$("$check" "$fixture" 2>&1) || [[ "$output" != *'failed'* ]]; then
  echo 'check-example-api passed an example it could not read' >&2
  exit 1
fi
chmod 644 "$fixture/examples/ping-bot/src/main.rs"

if output=$("$check" "$fixture/missing" 2>&1); then
  echo 'check-example-api passed a tree it could not search' >&2
  exit 1
fi
