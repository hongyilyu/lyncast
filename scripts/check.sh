#!/usr/bin/env bash
# The local CI gate: formatting, lints, tests, a Tier 2 smoke test, and the bench gate.
set -euo pipefail
# An exported CDPATH would make cd print the directory into captured output.
unset CDPATH

cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo fmt --manifest-path bench/probe/Cargo.toml --check
cargo fmt --manifest-path bench/score-map/Cargo.toml --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --manifest-path bench/probe/Cargo.toml --target-dir target/bench-probe --locked -- -D warnings
cargo clippy --manifest-path bench/score-map/Cargo.toml --target-dir target/bench-score-map --locked -- -D warnings
cargo test --workspace --locked
shellcheck scripts/*.sh
scripts/test-env.sh run -- python3 -I scripts/wl-globals.py >/dev/null
scripts/bench.sh --gate
echo "check.sh: all checks passed"
