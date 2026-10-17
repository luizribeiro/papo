#!/usr/bin/env bash
set -euo pipefail

check=${1:-"$(dirname "$0")/check-no-merge-commits"}
check=$(cd "$(dirname "$check")" && pwd)/$(basename "$check")
repo=$(mktemp -d)
trap 'rm -rf "$repo"' EXIT

git -C "$repo" init --quiet --initial-branch=main
git -C "$repo" config user.name "Papo Tests"
git -C "$repo" config user.email "tests@papo.invalid"
git -C "$repo" config commit.gpgsign false
touch "$repo/file"
git -C "$repo" add file
git -C "$repo" commit --quiet -m base
git -C "$repo" switch --quiet -c feature
echo feature >"$repo/file"
git -C "$repo" commit --quiet -am feature

(cd "$repo" && "$BASH" "$check")

git -C "$repo" switch --quiet main
git -C "$repo" switch --quiet -c side
echo side >"$repo/side"
git -C "$repo" add side
git -C "$repo" commit --quiet -m side
git -C "$repo" switch --quiet feature
git -C "$repo" merge --quiet --no-ff side -m "merge side"

if output=$(cd "$repo" && "$BASH" "$check" 2>&1); then
  echo "merge history was accepted" >&2
  exit 1
fi
if [[ "$output" != *"merge side"* ]]; then
  echo "rejection did not identify the merge commit" >&2
  exit 1
fi
