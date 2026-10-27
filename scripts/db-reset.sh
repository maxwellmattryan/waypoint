#!/usr/bin/env bash
# Drops and recreates the dev database ($POSTGRES_DB in the Compose `db` service), deleting
# all its data. Use it when the API reports a migration was "previously applied but has been
# modified" or is "missing", usually after switching branches or worktrees. Migrations apply
# again on the next ./scripts/dev.sh. Pass --yes to skip the typed confirmation.
# Written for macOS's stock bash 3.2.
set -euo pipefail

usage="Usage: scripts/db-reset.sh [--yes]"
yes=0
for arg in "$@"; do
	case "$arg" in
	--yes) yes=1 ;;
	*)
		echo "$usage" >&2
		exit 2
		;;
	esac
done

cd "$(dirname "$0")/.."

if [ ! -f .env ]; then
	echo "Missing .env — run: cp .env.example .env" >&2
	exit 1
fi

set -a
# shellcheck disable=SC1091
. ./.env
set +a

db="${POSTGRES_DB:-}"
if ! printf '%s' "$db" | grep -Eq '^[A-Za-z0-9_]+$'; then
	echo "Refusing: POSTGRES_DB must match ^[A-Za-z0-9_]+\$ (got '$db')." >&2
	exit 1
fi
case "$db" in
postgres | template0 | template1)
	echo "Refusing: POSTGRES_DB is the system database '$db'." >&2
	exit 1
	;;
esac
if [ -z "${POSTGRES_USER:-}" ]; then
	echo "Refusing: POSTGRES_USER is not set in .env." >&2
	exit 1
fi

if [ "$yes" -ne 1 ]; then
	if [ ! -t 0 ]; then
		echo "Refusing: stdin is not a terminal. Pass --yes to reset $db without a prompt." >&2
		exit 1
	fi
	echo "This drops the database '$db' in the Compose db service and deletes all its data."
	printf 'Type the database name to confirm: '
	answer=""
	read -r answer || true
	if [ "$answer" != "$db" ]; then
		echo "Aborted: '$answer' is not '$db'. Nothing was changed." >&2
		exit 1
	fi
fi

docker compose up -d --wait db

docker compose exec -T db psql -U "$POSTGRES_USER" -d postgres -v ON_ERROR_STOP=1 \
	-c "DROP DATABASE IF EXISTS \"$db\" WITH (FORCE)" \
	-c "CREATE DATABASE \"$db\""

echo "Reset $db. Migrations apply on the next ./scripts/dev.sh."
