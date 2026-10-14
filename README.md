# lyncast

lyncast is a small, fast launcher for the [niri](https://github.com/niri-wm/niri) Wayland compositor, written in Rust with GPUI. It is modeled on TinyCast and Raycast.

**Status:** design done, code not started. See the [MVP build plan](docs/plan.md).

## What the MVP does

- Launches apps from their `.desktop` entries, with fuzzy search and frecency ranking.
- Keeps a clipboard history of text, images, and files, and copies an entry back with Enter.
- Answers arithmetic typed into the search box (`+`, `-`, `*`, `/`, square root) in a card above the results.
- Runs built-in commands, including lock, suspend, log out, reboot, and power off.

The palette opens in about 45 ms from a resident daemon (measured on a software renderer). The memory budget is in the [decision log](docs/decisions.md#budgets).

## Docs

Start with the [docs index](docs/README.md). The [decision log](docs/decisions.md) records every decision and its evidence.

## Build

lyncast pins Rust 1.98.1 in `rust-toolchain.toml`.

```sh
cargo build
```

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
