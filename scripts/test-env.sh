#!/usr/bin/env bash
# Runs a private Wayland compositor for Tier 2 tests: Xvfb plus smithay anvil
# with the winit backend, in a fresh XDG_RUNTIME_DIR (docs/design/testing.md).
#
#   test-env.sh [start]         start one and print shell lines that point at it
#   test-env.sh stop DIR        stop the one whose XDG_RUNTIME_DIR is DIR
#   test-env.sh run -- CMD...   run CMD inside a new one, then stop it
#
# start prints nothing until a client has seen the globals in $required.
set -euo pipefail
# An exported CDPATH would make cd print the directory into captured output.
unset CDPATH

scripts=$(cd "$(dirname "$0")" && pwd)
required=(wl_seat zwlr_layer_shell_v1 zwlr_data_control_manager_v1)
# Variables that would let a child reach the caller's session.
leaks=(WAYLAND_SOCKET DISPLAY NIRI_SOCKET DBUS_SESSION_BUS_ADDRESS)

# The environment this process is starting or running. The EXIT trap stops it,
# and bash also runs that trap when HUP, INT, or TERM ends the script. A trap
# on those signals would make bash wait for the running command instead.
env_dir=
trap 'trap "" HUP INT TERM; cleanup' EXIT

usage() {
	sed -n 's/^#   //p' "$0" >&2
	exit 2
}

# Runs a command until it succeeds, for at most about 10 s.
retry() {
	local end=$((SECONDS + 10))
	until "$@"; do
		((SECONDS < end)) || return 1
		sleep 0.05
	done
}

# True while pid is alive and was started by the environment in dir.
owned() {
	local pid=$1 dir=$2
	grep -qxzF "XDG_RUNTIME_DIR=$dir" "/proc/$pid/environ" 2>/dev/null
}

signal() {
	local sig=$1 dir=$2 pid
	for pid in "${@:3}"; do
		if owned "$pid" "$dir"; then kill "-$sig" "$pid" 2>/dev/null || true; fi
	done
}

exited() {
	local dir=$1 pid
	for pid in "${@:2}"; do
		if owned "$pid" "$dir"; then return 1; fi
	done
}

stop() {
	local dir pids
	[[ -e $1 ]] || return 0
	# start records the canonical path, so DIR/, ./DIR, and symlinks all match.
	dir=$(cd -- "$1" 2>/dev/null && pwd -P) || true
	if [[ ${dir##*/} != lyncast-env.* || ! -f $dir/pids ]]; then
		echo "test-env.sh: $1 is not a test environment" >&2
		return 1
	fi
	mapfile -t pids <"$dir/pids"
	signal TERM "$dir" "${pids[@]}"
	if ! retry exited "$dir" "${pids[@]}"; then
		signal KILL "$dir" "${pids[@]}"
		retry exited "$dir" "${pids[@]}" || { echo "test-env.sh: processes in $dir/pids did not exit" >&2; return 1; }
	fi
	rm -rf "$dir"
}

# Also kills this shell's own children, in case a signal arrived between
# starting one and recording its pid.
cleanup() {
	[[ -n $env_dir ]] || return 0
	# shellcheck disable=SC2046 # one word per pid
	kill $(jobs -p) 2>/dev/null || true
	stop "$env_dir"
}

fail() {
	local log
	echo "test-env.sh: $1" >&2
	for log in "$env_dir"/*.log; do
		echo "--- $log" >&2
		tail -n 20 "$log" >&2
	done
	exit 1
}

# Succeeds once Xvfb has written its display number.
displayed() {
	kill -0 "$1" 2>/dev/null || fail "Xvfb exited"
	[[ -s $env_dir/display ]]
}

# Succeeds once anvil answers a roundtrip on wayland-1, the first name it
# tries in an empty directory.
serving() {
	kill -0 "$1" 2>/dev/null || fail "anvil exited"
	WAYLAND_DISPLAY=wayland-1 python3 -I "$scripts/wl-globals.py" \
		>"$env_dir/globals" 2>"$env_dir/wl-globals.log"
}

# Starts an environment and points this shell at it. It must run in the main
# shell, because it sets env_dir for the EXIT trap and exports the variables
# that run and the printed lines use.
start() {
	local anvil dir pid g
	anvil=$("$scripts/anvil.sh")
	# A short fixed base keeps the socket path under the 108-byte sun_path limit.
	dir=$(mktemp -d /tmp/lyncast-env.XXXXXX)
	: >"$dir/pids"
	env_dir=$(cd -- "$dir" && pwd -P)
	# Every child gets XDG_RUNTIME_DIR=$env_dir, which is how owned() knows it.
	export XDG_RUNTIME_DIR=$env_dir
	unset WAYLAND_DISPLAY "${leaks[@]}"
	# -fbdir keeps the screen in $env_dir/Xvfb_screen0, an XWD file that screenshots read.
	Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp -fbdir "$env_dir" \
		3>"$env_dir/display" >"$env_dir/xvfb.log" 2>&1 </dev/null &
	pid=$!
	echo "$pid" >>"$env_dir/pids"
	retry displayed "$pid" || fail "Xvfb did not report a display"
	# With no WAYLAND_DISPLAY, winit opens anvil in Xvfb instead of nesting it
	# in the caller's session.
	DISPLAY=":$(<"$env_dir/display")" "$anvil" --winit >"$env_dir/anvil.log" 2>&1 </dev/null &
	pid=$!
	echo "$pid" >>"$env_dir/pids"
	retry serving "$pid" || fail "anvil did not answer a Wayland roundtrip"
	for g in "${required[@]}"; do
		grep -q "^$g " "$env_dir/globals" || fail "anvil does not offer $g"
	done
	export WAYLAND_DISPLAY=wayland-1
}

case ${1:-start} in
start)
	start
	printf 'export XDG_RUNTIME_DIR=%s\nexport WAYLAND_DISPLAY=%s\nunset %s\n' \
		"$XDG_RUNTIME_DIR" "$WAYLAND_DISPLAY" "${leaks[*]}"
	env_dir= # The caller owns it now.
	;;
stop) [[ $# -eq 2 ]] || usage; stop "$2" ;;
run) [[ ${2:-} == -- && $# -ge 3 ]] || usage; shift 2; start; "$@" ;;
*) usage ;;
esac
