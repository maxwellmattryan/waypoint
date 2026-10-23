#!/bin/sh
# Stand-in for the `claude` CLI in tests. It records how it was called, then plays a mode.
# Usage: fake-claude.sh <mode> <record_dir> <claude args...>
#   <mode>.jsonl  print that fixture from this directory and exit 0
#   slow          sleep 1s, then play success.jsonl
#   hang          sleep until killed
#   hang_tree     start a background `sleep` (its pid goes to <record_dir>/grandchild),
#                 then sleep until killed
#   exit1         write to stderr and exit 1 without a result
#   garbage       print a line that is not JSON
#   flood         print 17 MiB with no newline (more than the runner reads)
#   mcp           with `--mcp-config <path>`: copy the config to <record_dir>/mcp_config,
#                 write the socket dir's mode to <record_dir>/mcp_dir_mode, run the
#                 config's relay command with mcp_requests.jsonl on stdin (its stdout goes
#                 to <record_dir>/mcp), then play success.jsonl. Without it (a tool's own
#                 model call during that relay), record into <record_dir>/inner and play
#                 success.jsonl.
#   mcp_<name>    like mcp, relaying mcp_<name>_requests.jsonl instead
set -eu

mode=$1
record=$2
shift 2
fixtures=$(cd "$(dirname "$0")" && pwd)

mcp_config=""
prev=""
for arg in "$@"; do
	if [ "$prev" = "--mcp-config" ]; then
		mcp_config=$arg
	fi
	prev=$arg
done
case $mode in
mcp | mcp_*) requests=${mode}_requests.jsonl ;;
*) requests="" ;;
esac
if [ -n "$requests" ] && [ -z "$mcp_config" ]; then
	record="$record/inner"
	mkdir -p "$record"
fi

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
slow)
	sleep 1
	cat "$fixtures/success.jsonl"
	;;
hang) exec sleep 60 ;;
hang_tree)
	sleep 60 &
	echo $! >"$record/grandchild"
	exec sleep 60
	;;
exit1)
	echo "fake claude: something broke" >&2
	exit 1
	;;
garbage) echo "not json" ;;
flood) exec head -c 17825792 /dev/zero ;;
mcp | mcp_*)
	if [ -n "$mcp_config" ]; then
		cp "$mcp_config" "$record/mcp_config"
		cmd=$(sed -n 's/.*"command":"\([^"]*\)".*/\1/p' "$mcp_config")
		sock=$(sed -n 's/.*"args":\["--socket","\([^"]*\)"\].*/\1/p' "$mcp_config")
		sock_dir=$(dirname "$sock")
		stat -c %a "$sock_dir" >"$record/mcp_dir_mode" 2>/dev/null ||
			stat -f %Lp "$sock_dir" >"$record/mcp_dir_mode"
		"$cmd" --socket "$sock" <"$fixtures/$requests" >"$record/mcp"
	fi
	cat "$fixtures/success.jsonl"
	;;
*) cat "$fixtures/$mode.jsonl" ;;
esac
