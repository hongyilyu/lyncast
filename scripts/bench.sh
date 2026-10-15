#!/usr/bin/env bash
# Builds bench/probe and runs bench/summon.py against it in a private anvil, on
# lavapipe unless VK_DRIVER_FILES says otherwise. Arguments go to summon.py,
# for example `scripts/bench.sh --procs 1 --cycles 30`.
set -euo pipefail
# An exported CDPATH would make cd print the directory into captured output.
unset CDPATH

cd "$(dirname "$0")/.."
bin=target/bench-probe/release/summon-probe
cargo build --release --locked --manifest-path bench/probe/Cargo.toml --target-dir target/bench-probe
# A binary still in the page cache counts as Private_Dirty until writeback.
sync "$bin"
export VK_DRIVER_FILES=${VK_DRIVER_FILES:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}
exec scripts/test-env.sh run -- python3 -I bench/summon.py "$bin" "$@"
