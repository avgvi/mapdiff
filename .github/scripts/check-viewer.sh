#!/usr/bin/env bash
# Starts the reviewer on a sample repository and walks through the map
# viewer in headless Chrome: levels, search, selection, trace, panning.
#
# Needs Chrome or Chromium, the sample repositories (.github/scripts/fetch-repos.sh)
# and a built page (cd ui && npm ci && npm run build).
#
#   .github/scripts/check-viewer.sh [repo]      # default: ripgrep, the largest
set -euo pipefail
cd "$(dirname "$0")/../.."

repo="${1:-ripgrep}"
root=".cache/repos/$repo"
if [ ! -d "$root" ]; then
  echo "no $root: run .github/scripts/fetch-repos.sh" >&2
  exit 2
fi
cargo build -q --release -p mapdiff
port=$((7900 + RANDOM % 90))
./target/release/mapdiff serve --repo "$root" --port "$port" --no-open >/dev/null 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do
  curl -fs "http://127.0.0.1:$port/api/meta" >/dev/null 2>&1 && break
  sleep 0.1
done
node ui/scripts/check-viewer.mjs "http://127.0.0.1:$port"
