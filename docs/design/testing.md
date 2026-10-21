# Testing lyncast on a headless box

The dev box has no display, so every lyncast test drives the CLI and checks what it prints. The [decision log](../decisions.md#testing) wins any disagreement with this page.

## Tests assert literal output

Each test runs `lyncast` and compares stdout, stderr, and the exit code with literal strings. Every palette action also exists as a CLI command over the same socket protocol, so a CLI test takes the palette's path. `lyncast daemon --headless` runs the core, the clipboard watcher, and the socket without GPUI, and `lyncast daemon --record <file>` makes it record each request it would send to niri instead of sending it ([daemon](daemon.md#the-runner-starts-programs-and-tests-swap-in-a-recorder)).

MIME types are the exception. Their order differed between two probe runs, so tests check membership ([notes/07](../research/notes/07-paste-seam-workflow.md), measured). A failed action exits 1 with the reason on stderr (notes/07). A usage error exits 2, and a stale rate snapshot will exit 3 ([decision log](../decisions.md#calculator-engine)).

## Each tier proves a different part

| Tier | Runs under | Proves | Cannot prove |
|---|---|---|---|
| 1 | No display, `WAYLAND_DISPLAY` cleared | Logic that needs no compositor, such as promote rules and the no-compositor error | The clipboard or the window |
| 2 | Xvfb plus smithay `anvil` | The marker MIME, serving, promotion, daemon ownership of the selection, and, with GPUI, summon, close, and the palette's pixels | ext-data-control, niri IPC beyond the bytes a fake niri reads, niri focus, real GPU timing |
| 3 | GPUI `TestAppContext` | Actions, keymaps, focus, simulated keystrokes, seeded async work (`SEED`), layout | Layer shell, compositor focus, IME, the clipboard between processes, pixels, real presentation |

Tier 1 clears `WAYLAND_DISPLAY` because wayland-client connects whenever it is set ([notes/07](../research/notes/07-paste-seam-workflow.md), read in source). A run inside niri would otherwise copy into the real clipboard. The notes/07 review also unsets `WAYLAND_SOCKET` and uses a temp `XDG_RUNTIME_DIR`. The Tier 3 lists come from [notes/02](../research/notes/02-gpui-on-linux.md), where GPUI's own headless tests passed with no display (measured).

Tier 3 draws no pixels. lyncast builds each `TestAppContext` with no headless renderer, so a test window lays out and paints into a scene and an atlas, and its `draw` renders nothing (`gpui-pre` 0.3.9 `src/platform/test/window.rs:547-573`, read in source). The palette's pixels come from Tier 2, where a `summon` block takes a screenshot under anvil ([palette.md](palette.md#a-screenshot-under-anvil-shows-the-palette)).

## Tier 2 runs anvil inside Xvfb

Each test starts its own Xvfb and anvil, with its own `XDG_RUNTIME_DIR` ([notes/03](../research/notes/03-wayland-niri-integration.md), recommended).

`scripts/test-env.sh` starts one. On first use it builds anvil through `scripts/anvil.sh`, which pins the smithay rev and the lockfile of the tested build:

```sh
scripts/test-env.sh run -- cargo test   # run a command inside a new one, then stop it
scripts/test-env.sh                     # start one and print shell lines that point at it
scripts/test-env.sh stop DIR            # stop the one whose XDG_RUNTIME_DIR is DIR
```

Prefer `run`. It stops the environment when the command exits, and the command never runs if the start fails. For an interactive shell inside one, run `scripts/test-env.sh run -- bash`.

`start` prints three lines, after `scripts/wl-globals.py` completes a roundtrip with anvil and sees `wl_seat`, `zwlr_layer_shell_v1`, and `zwlr_data_control_manager_v1`:

```sh
export XDG_RUNTIME_DIR=/tmp/lyncast-env.XXXXXX
export WAYLAND_DISPLAY=wayland-1
unset WAYLAND_SOCKET DISPLAY NIRI_SOCKET DBUS_SESSION_BUS_ADDRESS
```

Xvfb keeps its screen in `$XDG_RUNTIME_DIR/Xvfb_screen0` (`-fbdir`), an XWD file that a Tier 2 test reads for a screenshot. The file holds 1280 by 800 pixels of 32 bits, about 4 MiB under `/tmp` for each environment.

Commands under `run` get the same environment. The `unset` line keeps a child off the caller's Wayland, X, niri, and D-Bus sessions. libwayland tries `WAYLAND_SOCKET` before `WAYLAND_DISPLAY`, a real `NIRI_SOCKET` would launch real apps, and GPUI reads the color scheme and cursor theme from the desktop portal on the session bus (read in source, gpui-pre 0.3.9). anvil gets no `WAYLAND_DISPLAY`, so it opens inside Xvfb even when the script runs inside niri.

A start fails within about 11 s if anvil never answers, and at once if Xvfb or anvil exits. Xvfb picks a free display with `-displayfd`, so several environments can run at once. Each one lives under `/tmp` whatever `TMPDIR` says, because the socket path must fit in 108 bytes. These known cases leave something behind (measured):

- SIGKILL to the script leaves the whole environment, because no trap can catch it. `stop DIR` still cleans it up.
- A signal in the instant between `mktemp` and recording the directory leaves the directory with an empty `pids` file, which `stop DIR` removes.
- Two copies of one signal that arrive within about a millisecond can skip the cleanup.
- SIGINT to `run` alone waits for the command to exit, as bash always does for SIGINT. Ctrl-C signals the whole process group, so it cleans up at once.
- SIGTERM or SIGHUP to `run` alone stops the environment and sends SIGTERM to the command. A command that survives SIGTERM, and any process the command started in the background, keep running without the compositor.

From step 6, each Tier 2 Rust test calls `start` and `stop` itself.

These facts come from notes/03 unless marked:

- Xvfb 21.1.13 is installed (tested).
- anvil at smithay HEAD 8a27023 built with `cargo build -p anvil --no-default-features --features egl,winit` in 32 s with no new packages. It runs as `DISPLAY=:97 anvil --winit` (tested).
- anvil offers `wl_seat` v9, layer-shell v5, zwlr-data-control v2, and xdg-activation, but no ext-data-control (tested).
- GPUI drew an 800x500 layer-shell window on the Overlay layer, but anvil never gave it keyboard focus ([notes/05](../research/notes/05-probe-summon-latency.md), measured).
- wgpu picked Vulkan lavapipe (notes/05, measured). Open zed PR #63346 would disable software ICDs at startup and could affect lavapipe in CI (notes/02, unverified).

### Headless weston cannot host GPUI

Weston 13.0.3 headless has no `wl_seat`, layer shell, data-control, or xdg-activation (notes/03, tested). gpui 0.2.2 panicked there on `seat.unwrap()` at `wayland/client.rs:503` ([notes/04](../research/notes/04-probe-gpui-0.2.2-headless.md), measured). `weston --backend=x11` inside Xvfb has a seat and drew a normal window (notes/04, measured), but GPUI returns `LayerShellNotSupportedError` on weston (notes/02, measured).

### Bare Xvfb needs an outside X event before GPUI presents

notes/03 recommends bare Xvfb for the later X11 path. With gpui 0.2.2 and no window manager, nothing is presented there (notes/04, measured). x11rb reads the MapNotify event into its in-process queue during `open_window`, so the socket never wakes GPUI's event loop. Any outside X event starts it, such as `xprop -set` or the probe's `PROBE_X11_KICK`, which changes a window property from a second X connection. No note re-checks the quirk on gpui-pre 0.3.9.

## Packages on Amazon Linux 2023

The notes record these needs:

- To build GPUI, headers for xkbcommon, xcb, wayland, and fontconfig, plus `libxkbcommon-x11-devel` for `-lxkbcommon-x11` (notes/02, notes/04).
- To run GPUI, Vulkan or EGL, which GPUI loads with dlopen (notes/02).
- To render without a GPU, Mesa 24.2.6 lavapipe and llvmpipe (notes/02, notes/05).

The probe installed these packages with `dnf` on 2026-10-14, read from its install log:

```sh
sudo dnf install mesa-vulkan-drivers vulkan-loader vulkan-tools weston xorg-x11-server-Xvfb \
	wayland-devel libxkbcommon-devel libxkbcommon-x11 libxkbcommon-x11-devel libxcb-devel \
	fontconfig-devel freetype-devel
```

`weston` is only needed to host GPUI under Xvfb for the old probe. Tier 2 uses anvil instead.

The clipboard fixtures run `sqlite3`, from the `sqlite` package, which the dev box has as 3.40.0. The scripts in `scripts/` and `bench/` also need Python 3. The dev box has 3.9.25 at `/usr/bin/python3`, and both `scripts/wl-globals.py` and `bench/summon.py` ran on it. `scripts/check.sh` also needs `shellcheck`, which the AL2023 repos do not have. The dev box has a release binary, 0.11.0, in `~/.local/bin`.

These are missing on the dev box:

- `wl-copy` and `wl-paste` (notes/07, measured). Tier 2 seeds and reads the clipboard through the test binary run again as a wl-clipboard-rs helper ([clipboard and paste](clipboard-and-paste.md#the-fixtures)).
- niri. Its build needs libinput, libseat, systemd, mesa-libgbm, pango, cairo, and libdisplay-info `-devel` packages, all in the AL2023 repos (notes/03). No note built it (notes/07), so niri inside Xvfb is unverified.

## Tier 2 cases for copy and close

The copy cases from [notes/07](../research/notes/07-paste-seam-workflow.md) are fixtures in [clipboard-and-paste.md](clipboard-and-paste.md#the-fixtures) since plan step 6: own writes, promotion, a copied answer, the filters, an image read back as the same PNG, a missing blob, file entries, and an empty clipboard after the daemon stops. A copy replies only after the capture thread sees its own marked offer, so a read right after `copied clip 1` gets the entry ([notes/11](../research/notes/11-clipboard-prototype.md), measured). Each seed waits for its row, because back-to-back seeds can race the watcher.

Two cases need the GPUI daemon under anvil, and [palette.md's](palette.md#the-fixtures) `summon` blocks hold them:

- **Close after copy.** After `lyncast toggle` and `lyncast activate clip:1`, a status query prints `closed`, the clipboard still holds the entry, and a second toggle opens one window. The case catches a nested close and a missing `QuitMode::Explicit`, which would quit and clear the selection.
- **Close after launch.** Activating an app row closes the window. The reviewed Launch arm never closed it.

## A fake niri socket stands in for niri IPC

From plan step 8, `crates/lyncast/tests/common/niri.rs` is the fake: a listener at `$RUNTIME/niri.sock` that reads each request line as niri does, logs it, and answers as a block's `$ fake-niri` line says ([launching](launching.md#the-fixtures)). Tier 1 blocks use it for launches. A daemon line points `NIRI_SOCKET` at it explicitly, and the harness accepts no other value but an empty one.

The paragraphs below are the paste spec from notes/07, which no test runs yet. Paste comes after the MVP, with the niri virtual keyboard as its first backend ([decision log](../decisions.md#clipboard-and-paste)). niri IPC takes one JSON request per line on `$NIRI_SOCKET` (notes/03). The fake in notes/07 answers `FocusedWindow` with window 7 (foot), serves `Workspaces` and `Windows`, streams `WindowFocusChanged { id: Some(7) }`, and logs each request.

In the unrun spec, `lyncast activate clip:1 --action paste` prints `pasted clip 1 into window 7 with ctrl+shift+v`, and a sink client on anvil prints `ctrl+shift+v alpha`. If the stream sends `Some(9)`, the CLI prints `copied clip 1; not pasted: focus went to window 9`.

The reviews found three limits, read in source:

- A new `EventStream` subscriber first gets a state burst with `WindowsChanged` and no `WindowFocusChanged`. The fake must send that burst first, or the test passes while real niri times out.
- anvil never restores the keymap after the virtual keyboard, so each paste test needs a fresh anvil.
- anvil focuses a toplevel only on a pointer click, and an exclusive layer surface only when a key event arrives.

## The bench gate holds the budgets

The limits are in the [Budgets table](../decisions.md#budgets). [notes/05](../research/notes/05-probe-summon-latency.md) measured each one on lavapipe under anvil, as medians:

- **Warm summon.** Time to present, the client's buffer attach and commit, was 44.9 ms, range 44.1 to 48.9, over summons 2 to 20 in 3 processes, run twice.
- **PSS before the first summon.** 11.9 MiB across 3 processes.
- **Private anonymous memory after summons.** `Pss_Anon` was 57.3 MiB after 20 summons and after 70 cycles.
- **Growth.** Flat by about cycle 10, then -0.6 to +7 kB per cycle from cycle 21 to 70, across 6 processes. The budget asks for 100 cycles.
- **Idle CPU.** 0.0% over 10 s while hidden, which is below the 10 ms tick, in 3 runs.

### Run the bench with scripts/bench.sh

`scripts/bench.sh` builds `bench/probe`, starts a test environment, selects lavapipe through `VK_DRIVER_FILES`, and runs `bench/summon.py`. Its arguments go to `summon.py`, for example `--procs 1 --cycles 30`. The default is 3 processes of 100 summon cycles, with latency from the first 20 summons, and takes about 6 minutes. `summon.py` prints each budget measurement and writes the raw data to `target/bench/`.

`bench/probe` is the notes/05 probe without its spawn-per-summon mode. It is its own cargo workspace, with the lockfile that notes/05 ran, so `cargo test` never builds GPUI. Until step 7 it stands in for the palette. Any program that reads the same socket lines and prints the same `PROBE` events can replace it, and the `summon.py` docstring lists both. The bench is not in `scripts/check.sh` yet, because no lyncast commit changes the probe.

### Private memory is Pss_Anon because binary pages can count as dirty

The probe's binary sat on tmpfs in notes/05, so its pages, about 10 MiB, or 17 MiB after a summon, counted as Private_Dirty. A binary on disk does the same until the kernel writes its pages back. On xfs, a fresh copy of the probe showed 9.6 MiB of Private_Dirty right after the write and 0.9 MiB after `sync`, and its PSS stayed 9.6 MiB (measured). `scripts/bench.sh` therefore runs `sync` on the binary before it measures. notes/05 uses `Pss_Anon` as the real cost, and the decision log's 57 MiB is that column. PSS still counts the binary, which is 9.8 of the 11.9 MiB before the first summon.

### Lavapipe numbers catch regressions but are not absolute

Runs repeat closely, with a rerun median of 45.3 ms and a memory spread of 0.6 MiB or less, so a regression stands out (inferred). The absolute values describe lavapipe, which compiles shaders and draws on the CPU:

- 78% of stack samples in a warm summon were in GPUI's pipeline creation, mostly lavapipe's SPIR-V to NIR compile (sampled). A warm cache on radv or anv should beat 45 ms (inferred).
- A real driver replaces libLLVM, libgallium, and lavapipe's memfd memory (inferred).
- anvil sent frame-done 2.2 ms after present (measured). niri shows a frame at the next vblank, up to about 16.7 ms at 60 Hz (inferred).
- The 45 ms needs a warm Mesa shader cache. With the cache off, a warm summon took 424.5 ms (measured).

Structural results, such as no drift per cycle and near-zero idle CPU, should carry over (notes/05). Real numbers come from Hongyi's niri machine.
