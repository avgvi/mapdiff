#!/usr/bin/env bash
# Fetch the pinned sample repositories listed in fixtures/repos.lock into
# .cache/repos/<name>, each checked out at its pinned commit.
#
# A repo already at its pin is left alone.
#
# Usage: .github/scripts/fetch-repos.sh [name ...]
#   With no names, fetches every repo in the lock file.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
lock="$root/fixtures/repos.lock"
cache="${MAPDIFF_REPO_CACHE:-$root/.cache/repos}"
# History depth. The diff tests walk recent commits, so keep some.
depth="${MAPDIFF_REPO_DEPTH:-60}"

mkdir -p "$cache"
status=0

while read -r name language url commit; do
  case "$name" in '' | '#'*) continue ;; esac

  if [ "$#" -gt 0 ]; then
    wanted=no
    for arg in "$@"; do
      [ "$arg" = "$name" ] && wanted=yes
    done
    [ "$wanted" = yes ] || continue
  fi

  dir="$cache/$name"
  if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$commit" ]; then
    echo "ok       $name ($language) at ${commit:0:12}"
    continue
  fi

  echo "fetching $name ($language) at ${commit:0:12}"
  mkdir -p "$dir"
  git -C "$dir" init -q
  # Named, so that a pull request can be fetched into the sample later.
  git -C "$dir" remote get-url origin >/dev/null 2>&1 || git -C "$dir" remote add origin "$url"
  if ! git -C "$dir" fetch -q --depth "$depth" "$url" "$commit"; then
    echo "error: could not fetch $commit from $url" >&2
    status=1
    continue
  fi
  git -C "$dir" -c advice.detachedHead=false checkout -q --force FETCH_HEAD

  actual="$(git -C "$dir" rev-parse HEAD)"
  if [ "$actual" != "$commit" ]; then
    echo "error: $name is at $actual, expected $commit" >&2
    status=1
  fi
done <"$lock"

exit "$status"
