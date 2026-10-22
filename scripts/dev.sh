#!/usr/bin/env bash
# Starts Waypoint for local development: Postgres (Docker Compose), the API and the
# web dev server. Ctrl-C stops the API and web server; Postgres keeps running.
# Written for macOS's stock bash 3.2 (no `wait -n`).
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -f .env ]; then
	echo "Missing .env — run: cp .env.example .env" >&2
	exit 1
fi

set -a
# shellcheck disable=SC1091
. ./.env
set +a

if [ ! -d web/node_modules ]; then
	npm --prefix web install
fi

docker compose up -d --wait db

# Job control puts each background child in its own process group, so cleanup can
# kill `cargo run`'s server and Vite's children along with them.
set -m

api_pid=""
web_pid=""
cleaned_up=0

cleanup() {
	if [ "$cleaned_up" -eq 1 ]; then
		return
	fi
	cleaned_up=1
	trap - INT TERM EXIT
	for pid in "$api_pid" "$web_pid"; do
		if [ -n "$pid" ]; then
			kill -TERM -- "-$pid" 2>/dev/null || true
		fi
	done
	for pid in "$api_pid" "$web_pid"; do
		if [ -n "$pid" ]; then
			wait "$pid" 2>/dev/null || true
		fi
	done
	echo "Stopped the API and web server. Postgres is still running (stop it with: docker compose down)."
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Builds the waypoint-mcp relay too, so agent calls find it beside the API binary.
cargo build -p waypoint-api --bins
cargo run -p waypoint-api &
api_pid=$!

npm --prefix web run dev &
web_pid=$!

while kill -0 "$api_pid" 2>/dev/null && kill -0 "$web_pid" 2>/dev/null; do
	sleep 1
done
