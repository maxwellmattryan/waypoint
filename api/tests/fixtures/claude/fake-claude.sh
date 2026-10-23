#!/bin/sh
# Stand-in for the `claude` CLI in tests. It records how it was called, then plays a mode.
# Usage: fake-claude.sh <mode> <record_dir> <claude args...>
#   <mode>.jsonl  print that fixture from this directory and exit 0
#   slow          sleep 1s, then play success.jsonl
#   hang          sleep until killed
#   hang_tree     start a background `sleep` (its pid goes to <record_dir>/grandchild),
#                 then sleep until killed
#   usage_limit_once  the first call creates <record_dir>/usage_limit_hit and plays
#                 usage_limit.jsonl; later calls play success.jsonl
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
#   classify_by_email  answer per email: play the fixture of the first line of
#                 classify_by_email.tsv (`<needle>TAB<file>.jsonl[TAB<seconds>]`, `#` lines
#                 are comments) whose needle `grep -F` finds in this call's stdin, after
#                 sleeping <seconds> when given, else classify_email_other.jsonl.
#                 Concurrent calls each read their own stdin.
#   route_<name>  like classify_by_email, but reads <name>.tsv, where a needle of `*`
#                 matches any stdin; with no line matching it fails like exit1.
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
# Reads to EOF, so the runner must close stdin for the fake to go on. This call's copy
# stays its own; `stdin` is the latest call's.
own_stdin="$record/stdin.$$"
cat >"$own_stdin"
cp "$own_stdin" "$record/stdin"
case $mode in
classify_by_email | route_*) ;;
*) rm -f "$own_stdin" ;;
esac

case $mode in
route_*)
	answer=""
	delay=""
	while IFS="	" read -r needle file wait; do
		case $needle in '' | '#'*) continue ;; esac
		if [ "$needle" = "*" ] || grep -qF -- "$needle" "$own_stdin"; then
			answer=$file
			delay=$wait
			break
		fi
	done <"$fixtures/${mode#route_}.tsv"
	rm -f "$own_stdin"
	if [ -z "$answer" ]; then
		echo "fake claude: no line of ${mode#route_}.tsv matches" >&2
		exit 1
	fi
	if [ -n "$delay" ]; then
		sleep "$delay"
	fi
	cat "$fixtures/$answer"
	;;
classify_by_email)
	answer=classify_email_other.jsonl
	delay=""
	while IFS="	" read -r needle file wait; do
		case $needle in '' | '#'*) continue ;; esac
		if grep -qF -- "$needle" "$own_stdin"; then
			answer=$file
			delay=$wait
			break
		fi
	done <"$fixtures/classify_by_email.tsv"
	if [ -n "$delay" ]; then
		sleep "$delay"
	fi
	rm -f "$own_stdin"
	cat "$fixtures/$answer"
	;;
slow)
	sleep 1
	cat "$fixtures/success.jsonl"
	;;
hang) exec sleep 60 ;;
usage_limit_once)
	if [ -e "$record/usage_limit_hit" ]; then
		cat "$fixtures/success.jsonl"
	else
		: >"$record/usage_limit_hit"
		cat "$fixtures/usage_limit.jsonl"
	fi
	;;
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
