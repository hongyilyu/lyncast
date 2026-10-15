#!/usr/bin/env bash
# The local CI gate: formatting, lints, tests, and a Tier 2 smoke test.
set -euo pipefail
# An exported CDPATH would make cd print the directory into captured output.
unset CDPATH

cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo fmt --manifest-path bench/probe/Cargo.toml --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --manifest-path bench/probe/Cargo.toml --target-dir target/bench-probe --locked -- -D warnings
cargo test --workspace --locked
shellcheck scripts/*.sh
scripts/test-env.sh run -- python3 -I scripts/wl-globals.py >/dev/null
echo "check.sh: all checks passed"
