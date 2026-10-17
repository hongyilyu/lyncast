# lyncast

lyncast is a small, fast launcher for the [niri](https://github.com/niri-wm/niri) Wayland compositor, written in Rust with GPUI. It is modeled on TinyCast and Raycast.

**Status:** steps 1 to 4 of the [MVP build plan](docs/plan.md), the scaffold, the core shapes, the calculator engine, and the app index and ranking, are done. `lyncast query` prints the ranked rows. The palette is not written yet.

## What the MVP does

- Launches apps from their `.desktop` entries, with fuzzy search and frecency ranking.
- Keeps a clipboard history of text, images, and files, and copies an entry back with Enter.
- Answers arithmetic typed into the search box (`+`, `-`, `*`, `/`) in a card above the results.
- Runs built-in commands, including lock, suspend, log out, reboot, and power off.

The palette opens in about 45 ms from a resident daemon (measured on a software renderer). The memory budget is in the [decision log](docs/decisions.md#budgets).

## Docs

Start with the [docs index](docs/README.md). The [decision log](docs/decisions.md) records every decision and its evidence.

## Build

lyncast pins Rust 1.98.1 in `rust-toolchain.toml`.

```sh
cargo build
scripts/check.sh   # fmt, clippy, tests, shellcheck, and a Tier 2 smoke test
scripts/bench.sh   # the summon bench, about 6 minutes
```

The scripts need Xvfb, Mesa, and Python 3. [Testing](docs/design/testing.md) lists the packages.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
