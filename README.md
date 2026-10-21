# lyncast

lyncast is a small, fast launcher for the [niri](https://github.com/niri-wm/niri) Wayland compositor, written in Rust with GPUI. It is modeled on TinyCast and Raycast.

**Status:** steps 1 to 8 of the [MVP build plan](docs/plan.md), the scaffold, the core shapes, the calculator engine, the app index and ranking, the daemon and the CLI, the clipboard, the palette, and launching through niri, are done. Step 9's bench gate runs in `scripts/check.sh`, and step 10 ships the systemd unit and the niri file that [Install](#install) sets up. Both steps still need their run on real hardware. `lyncast daemon --headless` runs the daemon, and `lyncast query`, `lyncast toggle`, `lyncast status`, `lyncast clip list`, and `lyncast activate` ask it, so they fail when no daemon is running. `lyncast daemon` without `--headless` opens a palette window on each summon, with the root list, the Actions menu, and the clipboard screen.

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

## Install

lyncast needs niri 26.04 or later, started as a systemd session through `niri-session`, which is how a display manager's niri session starts it. Inside the session, `systemctl --user is-active niri.service` prints `active`.

The steps add a line to `~/.config/niri/config.kdl`. While that file does not exist, niri reads `/etc/niri/config.kdl`, so the steps copy that file there first. If neither file exists, the copy fails and the steps leave `config.kdl` alone. Start niri once, which writes its default config to `~/.config/niri/config.kdl`, and run the steps again. From a checkout of this repository:

```sh
cargo build --release --locked
install -Dm755 target/release/lyncast ~/.local/bin/lyncast
install -Dm644 dist/systemd/lyncast.service ~/.config/systemd/user/lyncast.service
install -Dm644 dist/niri/lyncast.kdl ~/.config/niri/lyncast.kdl
test -e ~/.config/niri/config.kdl || cp /etc/niri/config.kdl ~/.config/niri/config.kdl
test -e ~/.config/niri/config.kdl && { grep -qxF 'include optional=true "lyncast.kdl"' ~/.config/niri/config.kdl || printf '\ninclude optional=true "lyncast.kdl"\n' >> ~/.config/niri/config.kdl; }
niri validate
systemctl --user daemon-reload
systemctl --user enable lyncast.service
```

`niri validate` must log `config is valid`. If it prints an error instead, do not log out, because niri would start with its default config, which holds none of your settings and no lyncast bind. Fix the error, or remove the `include` line, and run `niri validate` again. Then log out and back in. niri starts the daemon, and `Mod+Space` opens the palette. To start the daemon in the running session instead, run `systemctl --user start lyncast.service`. Then `systemctl --user status lyncast.service` shows `active (running)`, and `~/.local/bin/lyncast status` prints `closed`.

The steps install three files:

- `~/.local/bin/lyncast` is the binary. The unit and the bind name this path, so neither depends on `PATH`.
- `~/.config/systemd/user/lyncast.service` runs `lyncast daemon` while the niri session runs. `niri.service` wants it, so it starts with niri and with no other desktop. It restarts the daemon when the process crashes, on a panic of its main thread or a signal such as SIGSEGV or SIGKILL, so the clipboard history keeps recording. It does not restart the daemon after Quit lyncast, after another daemon took the socket, or after the compositor went.
- `~/.config/niri/lyncast.kdl` binds `Mod+Space` to `lyncast toggle` and blurs the wallpaper behind the palette. The `include` line pulls it in, and `optional=true` lets niri start without it.

A later bind replaces an earlier one with the same keys, so with the `include` line at the end of `config.kdl`, a `Mod+Space` bind of your own, often `switch-layout`, stops working. To keep yours, move the `include` line to the top of `config.kdl`, and give lyncast another key inside your own `binds` block:

```kdl
    Mod+Slash repeat=false hotkey-overlay-title="Open lyncast" { spawn "~/.local/bin/lyncast" "toggle"; }
```

Do not paste `lyncast.kdl` into `config.kdl`, and do not add a second `binds` block there. Two `binds` blocks in one file are an error, and niri then starts with its default config, which has no lyncast bind. `niri validate` reports such errors.

To blur the windows under the palette instead of the wallpaper, uncomment `xray false` in `lyncast.kdl`. niri marks that kind of blur experimental, and it disappears while the palette opens and closes.

To upgrade, pull, run the install steps again, which add the `include` line only once, and run `systemctl --user restart lyncast.service`. The steps put the shipped `lyncast.kdl` back, so make an `xray false` edit again after each upgrade.

To remove lyncast, run `systemctl --user disable --now lyncast.service`, delete the three files, and remove the `include` line from `config.kdl`.

### Without systemd

If niri does not run as `niri.service`, for example when `niri --session` starts from a TTY, the unit never starts. Start the daemon from niri instead, with this line in `config.kdl`:

```kdl
spawn-at-startup "~/.local/bin/lyncast" "daemon"
```

Use the unit or this line, not both, since a second daemon exits with `lyncast: a daemon is already running`. niri does not restart a daemon that it started this way after a crash, and it discards the daemon's output.

### When something is off

- If `Mod+Space` does nothing, the daemon is probably not running. niri discards a bind's output, so `lyncast toggle` fails there without a message. Run `systemctl --user status lyncast.service` and `journalctl --user -u lyncast.service`.
- If the palette misses apps, such as Flatpak apps, or names them in another language than you expect, the daemon's environment differs from your shell's. The daemon reads its environment from the systemd user manager, and `niri-session` copies the login environment there. Compare `systemctl --user show-environment` with `env`. To give the daemon the variables it lacks, run `systemctl --user edit lyncast.service`, add a `[Service]` line and an `Environment=` line for each variable, such as `Environment=LANG=de_DE.UTF-8`, and run `systemctl --user restart lyncast.service`. A file under `~/.config/environment.d/` does not help with a variable that the login environment sets too, such as `LANG` or `XDG_DATA_DIRS`, because the login's value wins.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
