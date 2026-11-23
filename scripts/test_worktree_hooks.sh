#!/usr/bin/env bash
set -euo pipefail

shell_hook=${1:?expected a dev shell hook script}
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_NOSYSTEM=1
export PRE_COMMIT_HOME="$fixture/pre-commit"

main="$fixture/main checkout"
linked="$fixture/linked checkout"
git init --quiet --initial-branch=main "$main"
git -C "$main" -c user.name="Papo Tests" -c user.email=tests@papo.invalid \
  commit --quiet --allow-empty -m base
expected=$(cd "$main/.git" && pwd -P)/hooks

check_shell_hook() {
  local checkout=$1 actual
  (cd "$checkout" && "$BASH" "$shell_hook")
  actual=$(git -C "$checkout" rev-parse --git-path hooks)
  if [[ "$actual" != "$expected" ]]; then
    printf '%s: expected hooks path %s, got %s\n' "$checkout" "$expected" "$actual" >&2
    exit 1
  fi
  test -x "$actual/pre-commit"
  test -x "$actual/pre-push"
}

check_shell_hook "$main"
git -C "$main" config --local core.hooksPath .git/hooks
check_shell_hook "$main"

git -C "$main" worktree add --quiet --detach "$linked"
git -C "$linked" config --local core.hooksPath .git/hooks
check_shell_hook "$linked"
git -C "$linked" config --local core.hooksPath .git/hooks
check_shell_hook "$linked"
