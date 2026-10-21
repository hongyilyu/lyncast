# lyncast

lyncast is a small, fast launcher for the [niri](https://github.com/niri-wm/niri) Wayland compositor, written in Rust with GPUI. It is modeled on TinyCast and Raycast.

**Status:** steps 1 to 8 of the [MVP build plan](docs/plan.md), the scaffold, the core shapes, the calculator engine, the app index and ranking, the daemon and the CLI, the clipboard, the palette, and launching through niri, are done. Step 9's bench gate runs in `scripts/check.sh`, and its run on real hardware is still to come. `lyncast daemon --headless` runs the daemon, and `lyncast query`, `lyncast toggle`, `lyncast status`, `lyncast clip list`, and `lyncast activate` ask it, so they fail when no daemon is running. `lyncast daemon` without `--headless` opens a palette window on each summon, with the root list, the Actions menu, and the clipboard screen.

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
scripts/check.sh          # fmt, clippy, tests, shellcheck, a Tier 2 smoke test, and the bench gate
scripts/bench.sh          # the summon bench and its budgets, about 7 minutes
scripts/bench.sh --gate   # 1 process of 100 cycles, about 2.5 minutes, as scripts/check.sh runs it
```

The scripts need Xvfb, Mesa, and Python 3. [Testing](docs/design/testing.md) lists the packages.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
