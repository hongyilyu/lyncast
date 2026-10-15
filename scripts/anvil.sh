#!/usr/bin/env bash
# Prints the path of smithay's anvil compositor, building it on first use.
# The rev and the lockfile are both pinned, because smithay does not commit
# its Cargo.lock.
set -euo pipefail
# An exported CDPATH would make cd print the directory into captured output.
unset CDPATH

# From the repo root, rust-toolchain.toml picks the compiler for the build.
cd "$(dirname "$0")/.."
rev=8a270232f1bfc416c781273c98a3f52a812a0bba
lock=scripts/anvil.Cargo.lock
# The key hashes this script and the lockfile, so a new rev, lockfile, or build
# flag gets a new binary.
key=$(cat scripts/anvil.sh "$lock" | sha256sum | cut -c1-16)
cache=${XDG_CACHE_HOME:-$HOME/.cache}/lyncast/anvil-$key
bin=$cache/anvil

if [[ ! -x $bin ]]; then
	mkdir -p "$cache"
	exec 9>"$cache/build.lock"
	flock 9
	if [[ ! -x $bin ]]; then
		echo "anvil.sh: building anvil at smithay $rev" >&2
		src=$(mktemp -d)
		trap 'rm -rf "$src"' EXIT
		git -C "$src" init -q
		git -C "$src" fetch -q --depth 1 https://github.com/Smithay/smithay "$rev"
		git -C "$src" checkout -q FETCH_HEAD
		cp "$lock" "$src/Cargo.lock"
		cargo build --release --locked --manifest-path "$src/Cargo.toml" \
			-p anvil --no-default-features --features egl,winit --target-dir "$src/target" >&2
		install -m 755 "$src/target/release/anvil" "$bin.tmp"
		mv "$bin.tmp" "$bin"
	fi
fi
echo "$bin"
