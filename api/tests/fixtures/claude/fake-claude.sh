#!/bin/sh
# Stand-in for the `claude` CLI in tests. It records how it was called, then plays a mode.
# Usage: fake-claude.sh <mode> <record_dir> <claude args...>
#   <mode>.jsonl  print that fixture from this directory and exit 0
#   hang          sleep until killed
#   exit1         write to stderr and exit 1 without a result
#   garbage       print a line that is not JSON
#   flood         print 17 MiB with no newline (more than the runner reads)
set -eu

mode=$1
record=$2
shift 2
fixtures=$(cd "$(dirname "$0")" && pwd)

: >"$record/argv"
for arg in "$@"; do
	printf '%s\n' "$arg" >>"$record/argv"
done
pwd >"$record/cwd"
ls -A >"$record/ls"
if [ -n "${ANTHROPIC_API_KEY+x}" ] || [ -n "${ANTHROPIC_AUTH_TOKEN+x}" ]; then
	echo set >"$record/env_api_key"
else
	echo unset >"$record/env_api_key"
fi
# Reads to EOF, so the runner must close stdin for the fake to go on.
cat >"$record/stdin"

case $mode in
hang) exec sleep 60 ;;
exit1)
	echo "fake claude: something broke" >&2
	exit 1
	;;
garbage) echo "not json" ;;
flood) exec head -c 17825792 /dev/zero ;;
*) cat "$fixtures/$mode.jsonl" ;;
esac
