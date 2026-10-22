#!/usr/bin/env bash
# Regenerates the API contract: api/openapi.json from the Rust handlers, then the
# web app's TypeScript types (web/src/lib/api/schema.d.ts) from that document.
# Run it after changing any route or response type, and commit both files.
set -euo pipefail

cd "$(dirname "$0")/.."

# Write to a temp file first so a failed build never leaves a half-written document.
trap 'rm -f api/openapi.json.tmp' EXIT
cargo run -q -p waypoint-api --bin openapi > api/openapi.json.tmp
mv api/openapi.json.tmp api/openapi.json

if [ ! -d web/node_modules ]; then
	npm --prefix web install
fi

npm --prefix web run gen:api
