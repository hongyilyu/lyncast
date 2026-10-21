#!/usr/bin/env bash
# Builds the release lyncast and bench/probe, and runs bench/summon.py on them.
#
#   bench.sh [ARG...]          the full bench in a private anvil on lavapipe, 3 processes of 100 cycles
#   bench.sh --gate [ARG...]   the bench that check.sh runs, 1 process of 100 cycles
#   bench.sh --real [ARG...]   in the niri session it runs in, on its GPU, judging nothing
#
# Each ARG goes to summon.py, for example `--procs 1 --cycles 30`. Unless VK_DRIVER_FILES says
# otherwise, the anvil runs select lavapipe.
set -euo pipefail
# An exported CDPATH would make cd print the directory into captured output.
unset CDPATH

cd "$(dirname "$0")/.."
mode=${1:-}
case $mode in
--gate | --real) shift ;;
esac
for arg in "$@"; do
	if [[ $arg == --real ]]; then
		echo "bench.sh: --real must be the first argument" >&2
		exit 2
	fi
done
if [[ $mode == --real && (-z ${NIRI_SOCKET:-} || -z ${WAYLAND_DISPLAY:-}) ]]; then
	echo "bench.sh: --real runs inside niri, but NIRI_SOCKET or WAYLAND_DISPLAY is not set" >&2
	exit 2
fi
target=${CARGO_TARGET_DIR:-target}
cargo build --release --locked -p lyncast
cargo build --release --locked --manifest-path bench/probe/Cargo.toml --target-dir "$target/bench-probe"
programs=("$target/release/lyncast" "$target/bench-probe/release/summon-probe")
# A binary still in the page cache counts as Private_Dirty until writeback.
sync "${programs[@]}"
summon=(python3 -I bench/summon.py "${programs[@]}" --out "$target/bench")
case $mode in
--real) exec "${summon[@]}" --real "$@" ;;
--gate) summon+=(--procs 1) ;;
esac
export VK_DRIVER_FILES=${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}
exec scripts/test-env.sh run -- "${summon[@]}" "$@"
