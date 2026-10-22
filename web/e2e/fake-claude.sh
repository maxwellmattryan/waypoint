#!/bin/sh
# Stands in for the `claude` CLI in the dashboard e2e test: plays the API test fixture's
# `slow` mode (sleeps 1s, then answers "pong"), recording its call in a throwaway directory.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec "$here/../../api/tests/fixtures/claude/fake-claude.sh" slow "$(mktemp -d)" "$@"
