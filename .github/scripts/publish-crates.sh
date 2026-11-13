#!/usr/bin/env bash
# Publishes the crates to crates.io, each after the ones it depends on.
# A crate whose version is already there is skipped, so the script can be
# run again after a failure.
#
# Needs CARGO_REGISTRY_TOKEN, and the built page copied to
# crates/mapdiff-server/page (cp -R ui/dist crates/mapdiff-server/page).
set -euo pipefail
cd "$(dirname "$0")/../.."

version="$(cargo metadata --no-deps --format-version 1 |
  python3 -I -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "mapdiff"))')"

for crate in mapdiff-core mapdiff-extract mapdiff-gitdiff mapdiff-graph mapdiff-server mapdiff; do
  if curl -fs -A mapdiff-release "https://crates.io/api/v1/crates/$crate/$version" >/dev/null; then
    echo "skipping $crate $version: already published"
    continue
  fi
  # crates.io limits how many new crates one account may create in a row.
  # Wait out the limit once before giving up.
  cargo publish -p "$crate" --allow-dirty || {
    echo "retrying $crate in 11 minutes"
    sleep 660
    cargo publish -p "$crate" --allow-dirty
  }
done
