#!/usr/bin/env bash
# Check that every hand-made fixture is valid in its own language, so a parse
# failure in the extractor is never the fixture's fault.
#
# Needs: cargo, python3, and the UI's node_modules (run `npm ci` in ui/ first).

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

bin="ui/node_modules/.bin"
if [ ! -x "$bin/tsc" ] || [ ! -x "$bin/oxlint" ]; then
  echo "error: run 'npm ci' in ui/ first (tsc and oxlint are needed)" >&2
  exit 1
fi

echo "rust: compile and format"
cargo check --quiet --manifest-path fixtures/rust/Cargo.toml
cargo fmt --check --manifest-path fixtures/rust/Cargo.toml

echo "python: parse"
python3 -I - fixtures/python/pkg/*.py <<'EOF'
import ast
import sys

for path in sys.argv[1:]:
    with open(path, encoding="utf-8") as handle:
        ast.parse(handle.read(), path)
EOF

echo "typescript: type-check"
"$bin/tsc" -p fixtures/typescript

echo "javascript and typescript: parse"
# All lint rules are off, so anything oxlint prints is a syntax error.
# With every rule off, the only thing left to fail on is a file that does
# not parse.
if ! output="$("$bin/oxlint" -A all fixtures/typescript fixtures/javascript 2>&1)" ||
  printf '%s' "$output" | grep -Eq "and [1-9][0-9]* errors?"; then
  printf '%s\n' "$output" >&2
  exit 1
fi

echo "fixtures ok"
