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

The clipboard fixtures run `sqlite3`, from the `sqlite` package, which the dev box has as 3.40.0. The install fixtures run `systemd-analyze`, which systemd 252 on the dev box ships. The scripts in `scripts/` and `bench/` also need Python 3. The dev box has 3.9.25 at `/usr/bin/python3`, and both `scripts/wl-globals.py` and `bench/summon.py` ran on it. `scripts/check.sh` also needs `shellcheck`, which the AL2023 repos do not have. The dev box has a release binary, 0.11.0, in `~/.local/bin`.

These are missing on the dev box:

- `wl-copy` and `wl-paste` (notes/07, measured). Tier 2 seeds and reads the clipboard through the test binary run again as a wl-clipboard-rs helper ([clipboard and paste](clipboard-and-paste.md#the-fixtures)).
- niri. Its build needs libinput, libseat, systemd, mesa-libgbm, pango, cairo, and libdisplay-info `-devel` packages, all in the AL2023 repos (notes/03). No note built it (notes/07), so niri inside Xvfb is unverified.

## Tier 2 cases for copy and close

The copy cases from [notes/07](../research/notes/07-paste-seam-workflow.md) are fixtures in [clipboard-and-paste.md](clipboard-and-paste.md#the-fixtures) since plan step 6: own writes, promotion, a copied answer, the filters, an image read back as the same PNG, a missing blob, file entries, and an empty clipboard after the daemon stops. A copy replies only after the capture thread sees its own marked offer, so a read right after `copied clip 1` gets the entry ([notes/11](../research/notes/11-clipboard-prototype.md), measured). Each seed waits for its row, because back-to-back seeds can race the watcher.

Two cases need the GPUI daemon under anvil, and [palette.md's](palette.md#the-fixtures) `summon` blocks hold them:

- **Close after copy.** After `lyncast toggle` and `lyncast activate clip:1`, a status query prints `closed`, the clipboard still holds the entry, and a second toggle opens one window. The case catches a nested close and a missing `QuitMode::Explicit`, which would quit and clear the selection.
- **Close after launch.** Activating an app row closes the window. The reviewed Launch arm never closed it.

## A fake niri socket stands in for niri IPC

From plan step 8, `crates/lyncast/tests/common/niri.rs` is the fake: a listener at `$RUNTIME/niri.sock` that reads each request line as niri does, logs it, and answers as a block's `$ fake-niri` line says ([launching](launching.md#the-fixtures)). Tier 1 blocks use it for launches, and Tier 2 blocks for a clip's Source and the palette's margin. A daemon line points `NIRI_SOCKET` at it explicitly, and the harness accepts no other value but an empty one.

The paragraphs below are the paste spec from notes/07, which no test runs yet. Paste comes after the MVP, with the niri virtual keyboard as its first backend ([decision log](../decisions.md#clipboard-and-paste)). niri IPC takes one JSON request per line on `$NIRI_SOCKET` (notes/03). The fake in notes/07 answers `FocusedWindow` with window 7 (foot), serves `Workspaces` and `Windows`, streams `WindowFocusChanged { id: Some(7) }`, and logs each request.

In the unrun spec, `lyncast activate clip:1 --action paste` prints `pasted clip 1 into window 7 with ctrl+shift+v`, and a sink client on anvil prints `ctrl+shift+v alpha`. If the stream sends `Some(9)`, the CLI prints `copied clip 1; not pasted: focus went to window 9`.

The reviews found three limits, read in source:

- A new `EventStream` subscriber first gets a state burst with `WindowsChanged` and no `WindowFocusChanged`. The fake must send that burst first, or the test passes while real niri times out.
- anvil never restores the keymap after the virtual keyboard, so each paste test needs a fresh anvil.
- anvil focuses a toplevel only on a pointer click, and an exclusive layer surface only when a key event arrives.

## The bench gate holds the budgets

The limits are in the [Budgets table](../decisions.md#budgets). `bench/summon.py` measures each one for the lyncast daemon on lavapipe under anvil and judges the run against them, and `scripts/check.sh` runs it on every commit. [notes/05](../research/notes/05-probe-summon-latency.md) first measured each one for `bench/probe`, as medians, and the limits came from these:

- **Warm summon.** Time to present, the client's buffer attach and commit, was 44.9 ms, range 44.1 to 48.9, over summons 2 to 20 in 3 processes, run twice.
- **PSS before the first summon.** 11.9 MiB across 3 processes.
- **Private anonymous memory after summons.** `Pss_Anon` was 57.3 MiB after 20 summons and after 70 cycles.
- **Growth.** Flat by about cycle 10, then -0.6 to +7 kB per cycle from cycle 21 to 70, across 6 processes. The budget asks for 100 cycles.
- **Idle CPU.** 0.0% over 10 s while hidden, which is below the 10 ms tick, in 3 runs.

### A run summons the daemon as lyncast toggle does

`summon.py` starts `lyncast daemon --trace` and reads its [PROBE lines](palette.md#--trace-prints-the-benchs-probe-lines). Each open and each close is one connection that carries `{"request":"toggle"}`, the request that `lyncast toggle` sends, and a summon counts from the write of its request. The bench checks each reply, waits for the summon's `window_closed` line after each close, and quits the daemon through `activate cmd:quit`, which must reply `ran cmd:quit` before the daemon exits 0. A wrong reply, a missing PROBE line, or another exit stops the run with exit status 1 before it judges anything. A `lyncast toggle` run for each summon would add the client's own start, about 2.6 ms on an earlier head (step 9's prework, measured), which the budget leaves out.

A run measures each process in these steps:

1. 2 s after `app_ready`, it reads the daemon's PSS, and then its CPU over 10 s while it is hidden.
2. It waits for a quiet box, as [a busy box](#a-busy-box-is-timed-again-then-fails-as-one) says.
3. It summons and closes the daemon 100 times. Each cycle sends the open, waits for `frame1_done` and 0.4 s more, sends the close, waits for `window_closed` and 0.4 s more, and reads `Pss_Anon` and the number of threads. It also reads the memory while the windows of summons 1 and 20 show.
4. Each of the first 20 summons is followed by a summon of `bench/probe`, which starts after step 2, runs beside the daemon, and quits after its 20th.
5. 1 s after cycle 100, it reads the memory again, then the CPU over 10 s.

Before the first process, a run summons each program twice and keeps only the times of those summons. That leaves the Mesa shader cache warm. With it cold, the first summon took 500.6 ms, and `Pss_Anon` was 72.0 MiB after 20 summons, where it was 60.0 MiB once the cache was warm (measured on 2026-10-21, 1 process of 22 cycles each).

### A run's daemons read and write none of the developer's history, ranking, or config

Each daemon gets `PATH` and `HOME` from the caller, with the variables of the GPU driver and its shader cache, `VK_*`, `MESA_*`, `LP_*`, and `XDG_CACHE_HOME`, the compositor's socket as an absolute `WAYLAND_DISPLAY`, `XDG_CURRENT_DESKTOP=niri`, and the [Vulkan directories](#both-programs-draw-with-the-callers-vulkan-drivers) that the caller's loader searches. Nothing else comes from the caller. `XDG_RUNTIME_DIR`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME` are directories of its own, which the run removes as it ends. `XDG_DATA_DIRS` holds 300 fixture apps, each with a 48 by 48 PNG icon, which `summon.py` writes the same way on every run. So a run reads and writes none of the developer's history, ranking, or config, and the empty query shows Clipboard History and the first 4 fixture apps with their icons ([ranking](ranking.md#the-empty-query-shows-suggestions-then-commands)). The probe gets the same variables and a runtime directory of its own. The Mesa shader cache stays where the caller keeps it, since a cold one fails both the latency and the memory budget, and an `XDG_CACHE_HOME` set around `scripts/test-env.sh` would also miss the anvil that `scripts/anvil.sh` caches there and build it again.

### Both programs draw with the caller's Vulkan drivers

The Vulkan loader looks for drivers in `vulkan/icd.d` and for implicit layers in `vulkan/implicit_layer.d` under `XDG_CONFIG_HOME`, each entry of `XDG_CONFIG_DIRS`, `/etc`, `XDG_DATA_HOME`, and each entry of `XDG_DATA_DIRS`, in that order, each variable with its default (vulkan-loader 1.3.296 with `VK_LOADER_DEBUG`, measured). The daemon's own directories hide both from that search. With `VK_DRIVER_FILES` unset, the daemon found no driver, so GPUI drew with GL, and with lavapipe's `VK_DRIVER_FILES` it loaded no `VK_LAYER_MESA_device_select`, which the probe loaded (measured on 2026-10-21). So `summon.py` lists those directories as the caller's variables name them and gives both programs the ones that exist: the driver directories as `VK_DRIVER_FILES`, unless the caller set it or `VK_ICD_FILENAMES`, and the layer directories as `VK_IMPLICIT_LAYER_PATH`, unless the caller set it. Each variable replaces the loader's own search.

Both programs print the log lines of GPUI's renderer, which name the adapter it picked, the daemon among its PROBE lines and the probe on its stdout. After the first timed summon of a process, the run reads both adapters and the `libVkLayer_*` libraries that each program maps. It stops with exit status 1 unless both programs drew with one adapter whose line ends in `(Vulkan)` and map the same layers, and it prints them as `== adapter of both programs:` and `== Vulkan layers of both programs:`.

### Each budget compares one statistic

| Budget | What the run compares with the limit |
|---|---|
| Warm summon | The median `frame1_presented` of summons 2 to 20, pooled over the processes. Summon 1 opens the first window of its process. |
| PSS before the first summon | The median over the processes of PSS 2 s after `app_ready`. |
| `Pss_Anon` after summons | For each process, the larger of `Pss_Anon` while summon 20 shows and after cycle 100, and then the median over the processes. |
| Growth | The largest least-squares slope of `Pss_Anon` over cycles 21 to 100 of any process, and no process with more threads at cycle 100 than at cycle 21. |
| Idle CPU | The largest CPU share of any process in its two 10 s windows while hidden, from the nanoseconds of the process's CPU clock, which counts the threads that have ended too. |

Ticks cannot judge idle CPU. One 10 ms tick in 10 s is exactly 0.1%, so a budget of under 0.1% would pass only on 0 ticks (notes/05). The run reads the process's CPU clock instead, the one that `clock_getcpuclockid(3)` gives for its pid, which counts the nanoseconds of every thread that the process has run, those that have ended too. A sum of `/proc/<pid>/task/<tid>/schedstat` over the threads would count only those still there at each read, and the daemon serves each connection and runs each task on a thread of its own that ends. With `cmd:reload-apps` activated every 2 s in each idle window, 4 times a window, the clock read 0.443% and failed the budget, while that sum read 0.021% and passed, and the ticks were 3 to 5 (deliberate breaks, 1 process of 22 cycles each, 2026-10-21). A run prints the ticks beside the clock.

PSS cannot judge the memory after summons. A shared library such as libLLVM counts by how many processes map it, and PSS after 20 summons moved from 92.8 to 106.7 MiB between back-to-back runs of the probe while `Pss_Anon` stayed within 1 MiB (step 9's prework, measured).

`Pss_Anon` climbs for about 60 cycles before it settles, so a slope over 100 cycles measures the settling too. In the head's runs of 2026-10-21, it rose 0.2 to 1.6 MiB from cycle 21 to cycle 100, partly in rises of up to 0.7 MiB within 3 cycles, so the slope over those cycles was 6.2 to 19.5 kB per cycle in 7 processes. An earlier build of the step, with the same daemon and a bench that gave it no Vulkan layer, gave 8.5 to 26.2 kB per cycle in 16 processes, and one process of 400 cycles of it grew 20.3 kB per cycle over cycles 21 to 100, 2.5 over cycles 101 to 400, and 1.3 over cycles 301 to 400, with 176 threads throughout. The limit of 40 kB per cycle sits above that settling, and a leak of 4 MB a summon, as look's was ([gpui-on-linux](../research/gpui-on-linux.md)), would give about 4,000.

### A busy box is timed again, then fails as one

lavapipe draws on every CPU, so other work on the box slows a summon. On the dev box, with 32 CPUs, the daemon took 45.2 to 45.8 ms process by process at load averages of 0.8 or less, and the probe beside it 44.0 to 44.4 ms. The probe alone took 43.3 to 43.5 ms at load averages under 1 and 45.5 to 46.1 ms at 2 to 8, and while a release build ran on every CPU, its median held at 45.7 ms while a tenth of its summons took 85 to 100 ms (each measured on 2026-10-21). With the box busy with other agents' work, at load averages of 23 to 55, the probe alone took 49 to 82 ms (step 9's brief). So before the timed summons, a run waits until fewer than a quarter of the CPUs were busy over a second, for 10 minutes at most, and it times the probe beside the daemon in the same cycles. A process whose summons took more than 50 ms while the probe's took more than 46.9 ms, 2 ms over notes/05's probe, timed a busy box, so the run measures it again, twice at most. A box that is still busy then fails the warm summon as `a busy box`. It never passes on one. With `LP_NUM_THREADS=1`, which slows lavapipe for both programs as a busy box does, the daemon took 98.0 ms and the probe 98.2 ms, the run measured the process three times, and the warm summon failed as `a busy box` (measured on 2026-10-21).

### scripts/bench.sh runs the bench

`scripts/bench.sh` builds the release `lyncast` and `bench/probe`, runs `sync` on both, starts a test environment, selects lavapipe through `VK_DRIVER_FILES` unless that is set, and runs `bench/summon.py` on them. Its arguments go to `summon.py`, for example `--procs 1 --cycles 30`. `summon.py` prints each measurement and each budget's verdict, exits 1 when a budget fails, and writes the raw data and the programs' stderr to `target/bench/`, or to `bench/` in `CARGO_TARGET_DIR`.

```sh
scripts/bench.sh          # 3 processes of 100 cycles, about 7 minutes
scripts/bench.sh --gate   # 1 process of 100 cycles, about 2.5 minutes, as scripts/check.sh runs it
scripts/bench.sh --real   # in a niri session, on its GPU, judging nothing
```

`python3 -I bench/summon.py --judge FILE` judges the raw data of a run again. The `judge` blocks below each judge a `run` block, the raw data of a run cut down to what the verdicts read, and `crates/lyncast/tests/bench.rs` checks that they print what the blocks say. A run that passes every budget exits 0:

```run quiet.json
{"args": {"warm": 3, "cycles": 6}, "runs": [
 {"summons": [{"frame1_presented": 99.6}, {"frame1_presented": 45.9}, {"frame1_presented": 45.1}],
  "probe_summons": [{"frame1_presented": 99.9}, {"frame1_presented": 44.4}, {"frame1_presented": 44.0}],
  "mem": {"hidden_before": {"Pss": 17612}, "shown_3": {"Pss_Anon": 59520}, "hidden_after_6": {"Pss_Anon": 61168}},
  "drift": [{"cycle": 1, "Pss_Anon": 45288, "threads": 176}, {"cycle": 2, "Pss_Anon": 50944, "threads": 176},
            {"cycle": 3, "Pss_Anon": 59408, "threads": 176}, {"cycle": 4, "Pss_Anon": 61100, "threads": 176},
            {"cycle": 5, "Pss_Anon": 61132, "threads": 176}, {"cycle": 6, "Pss_Anon": 61168, "threads": 176}],
  "idle_before": {"sched_pct": 0.0013}, "idle_after": {"sched_pct": 0.0012}}]}
```

```judge
$ summon.py --judge quiet.json
pass warm summon: 45.5 ms, probe 44.2 ms (50 ms or less)
pass PSS before the first summon: 17.2 MiB (18 MiB or less)
pass Pss_Anon after summons: 59.7 MiB (62 MiB or less)
pass growth over cycles 4 to 6: 34.0 kB per cycle at most, no new threads (40 kB per cycle or less)
pass idle CPU: 0.001% at most (under 0.1%)
summon.py: every budget passed
```

A run in which every budget fails prints each one and exits 1. Its two processes pool their summons for the warm summon and give the medians of the memory budgets, and the first one has the largest slope, a new thread, and the largest idle share:

```run slow.json
{"args": {"warm": 3, "cycles": 6}, "runs": [
 {"summons": [{"frame1_presented": 99.0}, {"frame1_presented": 52.0}, {"frame1_presented": 53.0}],
  "probe_summons": [{"frame1_presented": 98.0}, {"frame1_presented": 44.0}, {"frame1_presented": 44.4}],
  "mem": {"hidden_before": {"Pss": 19456}, "shown_3": {"Pss_Anon": 64000}, "hidden_after_6": {"Pss_Anon": 65000}},
  "drift": [{"cycle": 1, "Pss_Anon": 50000, "threads": 176}, {"cycle": 2, "Pss_Anon": 55000, "threads": 176},
            {"cycle": 3, "Pss_Anon": 58000, "threads": 176}, {"cycle": 4, "Pss_Anon": 60000, "threads": 176},
            {"cycle": 5, "Pss_Anon": 60050, "threads": 176}, {"cycle": 6, "Pss_Anon": 60100, "threads": 177}],
  "idle_before": {"sched_pct": 0.0013}, "idle_after": {"sched_pct": 0.12}},
 {"summons": [{"frame1_presented": 97.0}, {"frame1_presented": 51.0}, {"frame1_presented": 52.6}],
  "probe_summons": [{"frame1_presented": 96.0}, {"frame1_presented": 44.2}, {"frame1_presented": 44.6}],
  "mem": {"hidden_before": {"Pss": 19661}, "shown_3": {"Pss_Anon": 64512}, "hidden_after_6": {"Pss_Anon": 64000}},
  "drift": [{"cycle": 1, "Pss_Anon": 50000, "threads": 176}, {"cycle": 2, "Pss_Anon": 55000, "threads": 176},
            {"cycle": 3, "Pss_Anon": 58000, "threads": 176}, {"cycle": 4, "Pss_Anon": 60000, "threads": 176},
            {"cycle": 5, "Pss_Anon": 60030, "threads": 176}, {"cycle": 6, "Pss_Anon": 60060, "threads": 176}],
  "idle_before": {"sched_pct": 0.002}, "idle_after": {"sched_pct": 0.003}}]}
```

```judge
$ summon.py --judge slow.json
fail warm summon: 52.3 ms, probe 44.3 ms (50 ms or less)
fail PSS before the first summon: 19.1 MiB (18 MiB or less)
fail Pss_Anon after summons: 63.2 MiB (62 MiB or less)
fail growth over cycles 4 to 6: 50.0 kB per cycle at most, threads 176 to 177 (40 kB per cycle or less)
fail idle CPU: 0.120% at most (under 0.1%)
summon.py: 5 of 5 budgets failed
exit: 1
```

A process that gains a thread fails growth, though its slope passes:

```run thread.json
{"args": {"warm": 3, "cycles": 6}, "runs": [
 {"summons": [{"frame1_presented": 99.6}, {"frame1_presented": 45.9}, {"frame1_presented": 45.1}],
  "probe_summons": [{"frame1_presented": 99.9}, {"frame1_presented": 44.4}, {"frame1_presented": 44.0}],
  "mem": {"hidden_before": {"Pss": 17612}, "shown_3": {"Pss_Anon": 59520}, "hidden_after_6": {"Pss_Anon": 61168}},
  "drift": [{"cycle": 1, "Pss_Anon": 45288, "threads": 176}, {"cycle": 2, "Pss_Anon": 50944, "threads": 176},
            {"cycle": 3, "Pss_Anon": 59408, "threads": 176}, {"cycle": 4, "Pss_Anon": 61100, "threads": 176},
            {"cycle": 5, "Pss_Anon": 61132, "threads": 176}, {"cycle": 6, "Pss_Anon": 61168, "threads": 177}],
  "idle_before": {"sched_pct": 0.0013}, "idle_after": {"sched_pct": 0.0012}}]}
```

```judge
$ summon.py --judge thread.json
pass warm summon: 45.5 ms, probe 44.2 ms (50 ms or less)
pass PSS before the first summon: 17.2 MiB (18 MiB or less)
pass Pss_Anon after summons: 59.7 MiB (62 MiB or less)
fail growth over cycles 4 to 6: 34.0 kB per cycle at most, threads 176 to 177 (40 kB per cycle or less)
pass idle CPU: 0.001% at most (under 0.1%)
summon.py: 1 of 5 budgets failed
exit: 1
```

A process whose CPU share was over 0.1% only in the 10 s before its first summon fails idle CPU:

```run idle.json
{"args": {"warm": 3, "cycles": 6}, "runs": [
 {"summons": [{"frame1_presented": 99.6}, {"frame1_presented": 45.9}, {"frame1_presented": 45.1}],
  "probe_summons": [{"frame1_presented": 99.9}, {"frame1_presented": 44.4}, {"frame1_presented": 44.0}],
  "mem": {"hidden_before": {"Pss": 17612}, "shown_3": {"Pss_Anon": 59520}, "hidden_after_6": {"Pss_Anon": 61168}},
  "drift": [{"cycle": 1, "Pss_Anon": 45288, "threads": 176}, {"cycle": 2, "Pss_Anon": 50944, "threads": 176},
            {"cycle": 3, "Pss_Anon": 59408, "threads": 176}, {"cycle": 4, "Pss_Anon": 61100, "threads": 176},
            {"cycle": 5, "Pss_Anon": 61132, "threads": 176}, {"cycle": 6, "Pss_Anon": 61168, "threads": 176}],
  "idle_before": {"sched_pct": 0.12}, "idle_after": {"sched_pct": 0.0012}}]}
```

```judge
$ summon.py --judge idle.json
pass warm summon: 45.5 ms, probe 44.2 ms (50 ms or less)
pass PSS before the first summon: 17.2 MiB (18 MiB or less)
pass Pss_Anon after summons: 59.7 MiB (62 MiB or less)
pass growth over cycles 4 to 6: 34.0 kB per cycle at most, no new threads (40 kB per cycle or less)
fail idle CPU: 0.120% at most (under 0.1%)
summon.py: 1 of 5 budgets failed
exit: 1
```

A daemon that missed 50 ms while the probe beside it took more than 46.9 ms timed a busy box, which fails the warm summon as one:

```run busy.json
{"args": {"warm": 3, "cycles": 6}, "runs": [
 {"summons": [{"frame1_presented": 99.0}, {"frame1_presented": 52.1}, {"frame1_presented": 52.5}],
  "probe_summons": [{"frame1_presented": 99.0}, {"frame1_presented": 48.0}, {"frame1_presented": 48.2}],
  "mem": {"hidden_before": {"Pss": 17612}, "shown_3": {"Pss_Anon": 59520}, "hidden_after_6": {"Pss_Anon": 61168}},
  "drift": [{"cycle": 1, "Pss_Anon": 45288, "threads": 176}, {"cycle": 2, "Pss_Anon": 50944, "threads": 176},
            {"cycle": 3, "Pss_Anon": 59408, "threads": 176}, {"cycle": 4, "Pss_Anon": 61100, "threads": 176},
            {"cycle": 5, "Pss_Anon": 61132, "threads": 176}, {"cycle": 6, "Pss_Anon": 61168, "threads": 176}],
  "idle_before": {"sched_pct": 0.0013}, "idle_after": {"sched_pct": 0.0012}}]}
```

```judge
$ summon.py --judge busy.json
fail warm summon: 52.3 ms, probe 48.1 ms, a busy box (50 ms or less)
pass PSS before the first summon: 17.2 MiB (18 MiB or less)
pass Pss_Anon after summons: 59.7 MiB (62 MiB or less)
pass growth over cycles 4 to 6: 34.0 kB per cycle at most, no new threads (40 kB per cycle or less)
pass idle CPU: 0.001% at most (under 0.1%)
summon.py: 1 of 5 budgets failed
exit: 1
```

`bench/probe` is the notes/05 probe without its spawn-per-summon mode. It is its own cargo workspace, with the lockfile that notes/05 ran, so `cargo test` never builds GPUI. It draws an 800 by 500 window of 1,000 text rows in GPUI and nothing of lyncast, so it is the floor that GPUI and lavapipe set on the same box at the same time.

### The real run on Hongyi's niri machine

`scripts/bench.sh --real` runs the full bench in the niri session it starts in, on the machine's own GPU, and judges nothing, since real numbers are recorded and not gated. Its daemons also get `NIRI_SOCKET`, so each summon asks niri for its output as an installed daemon does, and `DBUS_SESSION_BUS_ADDRESS`, so GPUI reads the desktop portal. The rest is as under anvil, and the run's history, ranking, and config go in directories that it removes.

`--real` is a mode, so it must be the first argument, and the mode needs `NIRI_SOCKET` and `WAYLAND_DISPLAY`. `summon.py --real` needs `NIRI_SOCKET` too, which [the test environment](#tier-2-runs-anvil-inside-xvfb) unsets, so no run under anvil is recorded as real. Each refusal exits 2, as `crates/lyncast/tests/bench.rs` checks:

```cli
$ scripts/bench.sh --real 2>&1
bench.sh: --real runs inside niri, but NIRI_SOCKET or WAYLAND_DISPLAY is not set
exit: 2
$ NIRI_SOCKET=/run/niri.sock WAYLAND_DISPLAY=wayland-1 scripts/bench.sh --procs 1 --real 2>&1
bench.sh: --real must be the first argument
exit: 2
$ WAYLAND_DISPLAY=wayland-1 python3 -I bench/summon.py lyncast probe --real 2>&1
summon.py: --real runs inside niri, but NIRI_SOCKET is not set
exit: 2
```

1. Check out the commit to measure on the AMD machine, with Rust from `rustup`, Python 3, Mesa's radv, and the packages that GPUI's build asks for. Log in to niri.
2. In a terminal inside niri, run `unset VK_DRIVER_FILES VK_ICD_FILENAMES`, so both programs get the drivers that the Vulkan loader finds, radv among them.
3. Leave the machine idle, and touch neither the keyboard nor the mouse until the run ends. It opens 364 windows that take the keyboard, over about 7 minutes. Copy nothing either: the daemon's capture thread reads the clipboard while it runs.
4. Run `scripts/bench.sh --real 2>&1 | tee bench-real.log`. The `== adapter of both programs:` line must name the AMD GPU, not llvmpipe, and end in `(Vulkan)`.
5. Send back `bench-real.log`, the JSON that its `== raw data:` line names, and what this prints, with `vulkaninfo` from the Vulkan tools:

```sh
uname -r; niri --version; grep -m1 'model name' /proc/cpuinfo; nproc
vulkaninfo --summary | sed -n '/^Devices:/,$p'; niri msg outputs
```

A run that stops with an error writes no JSON. Send the newest `target/bench/summon-real-*.stderr.log` in its place.

The log and those lines go verbatim into `docs/research/notes/13-real-hardware-bench.md`. The decision log's Budgets section then gets one line with the warm summon and the two memory budgets of that run, linked to the note.

### Private memory is Pss_Anon because binary pages can count as dirty

The probe's binary sat on tmpfs in notes/05, so its pages, about 10 MiB, or 17 MiB after a summon, counted as Private_Dirty. A binary on disk does the same until the kernel writes its pages back. On xfs, a fresh copy of the probe showed 9.6 MiB of Private_Dirty right after the write and 0.9 MiB after `sync`, and its PSS stayed 9.6 MiB (measured). `scripts/bench.sh` therefore runs `sync` on the binary before it measures. notes/05 uses `Pss_Anon` as the real cost, and its probe's 57.3 MiB, from which the decision log's first limit came, is that column. PSS still counts the binary, which is 9.8 of the 11.9 MiB before the first summon.

### Lavapipe numbers catch regressions but are not absolute

Runs repeat closely, with a rerun median of 45.3 ms and a memory spread of 0.6 MiB or less, so a regression stands out (inferred). The absolute values describe lavapipe, which compiles shaders and draws on the CPU:

- 78% of stack samples in a warm summon were in GPUI's pipeline creation, mostly lavapipe's SPIR-V to NIR compile (sampled). A warm cache on radv or anv should beat 45 ms (inferred).
- A real driver replaces libLLVM, libgallium, and lavapipe's memfd memory (inferred).
- anvil sent frame-done 2.2 ms after present (measured). niri shows a frame at the next vblank, up to about 16.7 ms at 60 Hz (inferred).
- The 45 ms needs a warm Mesa shader cache. With the cache off, a warm summon took 424.5 ms (measured).

notes/05's probe showed no drift per cycle and near-zero idle CPU, and such structural results should carry over (notes/05). The daemon's `Pss_Anon` climbs for about 60 cycles on lavapipe before it settles, as [the growth statistic](#each-budget-compares-one-statistic) says. Real numbers come from Hongyi's niri machine.
