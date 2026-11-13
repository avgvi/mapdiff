#!/usr/bin/env bash
# Lays out the map of each pinned sample repository at every level, and the
# map of its last commit, and fails when a layout has overlapping boxes, a
# wire through a box it does not belong to, or differs between two runs.
#
# Needs the sample repositories (.github/scripts/fetch-repos.sh), the UI's packages
# (cd ui && npm ci) and a Node that runs TypeScript files directly (22.18
# or later).
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo build -q --release -p mapdiff
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

for repo in ripgrep zustand express httpx; do
  root=".cache/repos/$repo"
  if [ ! -d "$root" ]; then
    echo "skipping $repo: run .github/scripts/fetch-repos.sh to fetch it"
    continue
  fi
  for level in architecture package module code; do
    ./target/release/mapdiff map --json --fast --level "$level" --root "$root" >"$out/$repo-$level.json"
  done
  ./target/release/mapdiff map --json --fast --level module --commit HEAD --root "$root" >"$out/$repo-last-commit.json"
done

cd ui
node scripts/measure-layout.ts "$out"/*.json
