# Wayland and niri integration (research agent, 2026-10-14)

Raw agent report, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.

I couldn't invoke `hongyi-mode`: the Skill tool refused it (disable-model-invocation), so Hongyi has to run `/hongyi-mode` themselves. I did the research without it. All scratch probes and builds are in `/tmp/research-wl/bench/`.

## 1. niri (latest release v26.04, 2026-04-25; HEAD 4263c9a, 2026-10-13; repo moved to github.com/niri-wm/niri)
- **Protocols (all present, checked in source):**
  - In `src/niri.rs`: wlr-layer-shell :2525, wlr-data-control :2555, ext-data-control-v1 :2560 (both also cover primary selection), xdg-activation :2606.
  - Layer-shell and data-control are hidden from sandboxed clients by the `client_is_unrestricted` filter.
  - zwlr-foreign-toplevel and ext-foreign-toplevel-list are both in `src/protocols/foreign_toplevel.rs:7-12,79`.
- **Activation:**
  - Tokens expire after 10 s (`src/handlers/mod.rs:86`). A token without a serial only marks the window urgent (:748-756).
  - New windows with no token still get focus through the "Smart" rule, unless a fullscreen window is pending or `strict-new-window-focus-policy` is set (`src/layout/workspace.rs:652`).
- **Hotkey:** `binds { Mod+Space repeat=false hotkey-overlay-title="Launcher" { spawn "mylauncher" "toggle"; } }` (wiki `Configuration:-Key-Bindings.md:66-70,160,196-235`).
  - Every `spawn` gets a fresh `XDG_ACTIVATION_TOKEN`/`DESKTOP_STARTUP_ID` (`src/input/mod.rs:794-801`, `src/utils/spawning.rs:152-155`).
  - Use the overlay layer. The top layer is hidden behind fullscreen windows (wiki `Layer‐Shell-Components.md:3`).
- **IPC:** `$NIRI_SOCKET`, one JSON request per line (wiki `IPC.md`).
  - Requests include Outputs, Windows, FocusedOutput, FocusedWindow, Action and EventStream (`niri-ipc/src/lib.rs:68-121`).
  - Each `Window` has id, app_id, title, pid, workspace_id and focus_timestamp (:1341-1372). A window switcher is `Windows` sorted by focus_timestamp, then `Action FocusWindow{id}`.
  - `Action Spawn` sent over IPC runs the same code as a key bind (`src/ipc/server.rs:382-392`): it mints a token, double-forks and puts the app in a systemd scope (spawning.rs:239-256).
  - The niri-ipc crate (26.4.0) is **GPL-3.0-or-later**.

## 2. Clipboard history
- **Protocol:** bind ext-data-control (fall back to zwlr), get a device for the seat, and read the selection/offer events plus `receive(mime, fd)`. No keyboard focus is needed. Only connected clients get events, and the data disappears when the source app exits, so **a resident process is required**.
- **Crates:**
  - **wl-clipboard-rs 0.9.4** (2026-09-28, MIT/Apache) added a blocking `watch::Watcher` and prefers ext over wlr (CHANGELOG). Run it on its own thread and put timeouts on pipe reads.
  - wayland-clipboard-listener 0.6.1 is GPL-3.0 with few users.
  - smithay-client-toolkit 0.21.1 has no data-control module.
  - clipboard-rs 0.3.5 polls on Wayland.
- **Prior art:**
  - **cliphist** v0.7.0: `wl-paste --watch cliphist store` into bbolt. Defaults are 750 items and 5 MB max per item; it skips entries when `CLIPBOARD_STATE=sensitive` (cliphist.go:64,83).
  - **clipse**: a `wl-paste --watch` subprocess on Wayland, XFixes via cgo on X11, history in a JSON file plus image temp files.
  - **vicinae** v0.29.1: a separate ext-only `data-control-server` helper. Stores SQLite (WAL, trigram search) with payload files and MD5 dedupe. It drops `vicinae/concealed` and flags `x-kde-passwordManagerHint`.
- **MIME:** handle `text/plain;charset=utf-8`, `image/png`, `text/uri-list` and `x-special/gnome-copied-files`. Skip any offer that includes `x-kde-passwordManagerHint` (wl-paste only checks that it is present, wl-paste.c:103-109).
- **Storage:** rusqlite 0.40.2 (bundled SQLite, FTS5) for metadata and text, with blobs stored as files named by hash. redb 4.4.0 (pure Rust, no full-text search) works if search stays in memory.
- **X11:** XFixes `SelectSelectionInput` on CLIPBOARD. **x11rb 0.14.0** (`xfixes` feature) works on Xvfb (tested). arboard 3.6.1 has no change notification. Whether Xdcv supports XFixes is **unverified**.

## 3. App launcher
- **freedesktop-desktop-entry 0.8.3** (2026-09-12, MPL-2.0) covers XDG dir scanning, locale, NoDisplay, OnlyShowIn/NotShowIn, Terminal and actions.
  - **Its Exec parser splits on plain whitespace.** I tested `sh -c "echo a b"` and got `["sh","-c","\"echo","a","b\"",…]`.
  - A Brave flatpak entry keeps literal `'run'` quotes.
  - You need your own Exec tokenizer: double quotes, escapes, and %f/%F/%u/%U, %i (`--icon X`), %c, %k.
- **Icons:** freedesktop-icons 0.4.0 parses every index.theme on first use. After that, each lookup probes files with `exists()` per directory and extension, and it doesn't use icon-theme.cache (`theme/mod.rs:126-147`). Resolve icons lazily for visible rows on a background thread and cache the results. I didn't measure the cost.
- **Launching:**
  - On niri, use IPC `Action Spawn`, which gives you the token and the systemd scope for free.
  - Elsewhere, use `systemd-run --user --scope` or double-fork plus setsid. `app2unit` (GPL-3.0) and `uwsm app` wrap this.
  - GPUI only creates activation tokens inside `open_uri`/`reveal_path`, from a mouse-press serial (`gpui_linux/src/linux/wayland/client.rs` around 1395-1425). There is no public API for launches triggered from the keyboard.
- **Frecency:**
  - zoxide multiplies the score by 4 within an hour, by 2 within a day, and divides by 2 within a week and by 4 after that.
  - vicinae adds 6 × (5·ln(1+0.1·count) + exponential recency with a 30-day half-life, capped) to the fuzzy score (`fuzzy-searchable.hpp:33-41`).

## 4. Fuzzy matching (measured here, AMD EPYC 9R14, one thread, paths from /usr)
| items | frizbee 0.13.0 | nucleo-matcher 0.3.1 |
|---|---|---|
| 1k | 5–8 µs | 20–30 µs |
| 10k | 34–73 µs | 250–440 µs |
| 100k | 0.4–1.2 ms | 2.9–5.2 ms |

The nucleo numbers include converting each string to UTF-32, which nucleo's high-level crate caches. Upstream, frizbee is about 4× faster than nucleo on 1.4M paths (BENCHMARKS.md). nucleo's last release was in 2024. skim and fuzzy-matcher are about 6× slower than nucleo; sublime_fuzzy was last released in 2020.

**Recommendation: frizbee** (MIT, no dependencies, tolerates typos, `match_list_indices` gives highlight positions). Re-score the whole list on each keystroke in the background. At 100k items you don't need nucleo's incremental machinery.

## 5. Calculator
- **fend-core 1.5.8** (MIT, **no dependencies**) has units and `evaluate_preview_with_interrupt`, which was written for live previews (lib.rs:600-630). Currency rates come from a handler you supply. Measured: 8 µs init, 18–175 µs per query, about 2 s clean build.
- **numbat 1.24.0** (MIT/Apache) has richer units and pulls exchange rates from the ECB. Measured: 41 ms to load its standard library, about 0.5 ms per query, 9 s build, 79 crates.
- **Avoid:** evalexpr 13.1.0 is **AGPL-3.0-only**, and meval was last released in 2018.
- **Minor:** kalk 3.2.3 has only basic units, cas-rs has a tiny user base, and rink-core (MPL-2.0) needs data files.

**Recommendation: fend-core.** On each keystroke, run the preview with a ~20 ms interrupt, show the result as the top row if there is one, and copy it on Enter.

## 6. Headless testing on AL2023
- **Weston 13.0.3 headless (tested):** has no layer-shell, no data-control, no xdg-activation, and not even `wl_seat`. It can't test the clipboard at all.
- **Xvfb 21.1.13 (installed, tested):** XFixes 5.0 change events work.
- **smithay anvil (HEAD 8a27023, tested):** `cargo build -p anvil --no-default-features --features egl,winit` took 32 s and needed no new packages.
  - Running `DISPLAY=:97 anvil --winit` inside Xvfb works on software rendering (llvmpipe). It offers `wl_seat` v9, layer-shell v5, zwlr-data-control v2 and xdg-activation.
  - The wl-clipboard-rs Watcher picked up a copy end to end and flagged a `--sensitive` copy as a password.
  - It has no ext-data-control.
- **niri:** `src/backend/headless.rs` says it is for tests, and `main.rs` hardcodes `headless=false`. Open PR #3953 adds `NIRI_BACKEND=headless`.
  - niri falls back to a nested window when `DISPLAY` is set (niri.rs:772-787), so niri inside Xvfb should behave like anvil. This is **unverified**.
  - Building it needs libinput, libseat, systemd, mesa-libgbm, pango, cairo and libdisplay-info `-devel` packages, all in the AL2023 repos. Build with `--no-default-features` to skip pipewire.
- **sway/wlroots:** not in the AL2023 repos. A source build needs newer meson (the repo has 0.63.3) and newer wayland-protocols (the repo has 1.36). Exact minimum versions are unverified.

**Recommended setup:** Xvfb plus anvil for each test, with its own `XDG_RUNTIME_DIR`. Add niri-in-Xvfb as a second tier to cover ext-data-control and niri IPC, and use bare Xvfb for the X11 path. GPUI renders through wgpu (GL and Vulkan), and the software drivers llvmpipe and lavapipe are installed, so the UI may render there too (unverified).

## Decisions this forces on the architecture
1. A **resident daemon** for the clipboard watcher, app index and frecency, plus a thin `toggle` CLI that talks to it over a Unix socket (the vicinae pattern). Keep the watcher off the GPU.
2. **GPUI from a pinned zed git revision.** The crates.io release 0.2.2 (2025-10-22) predates layer-shell (zed PR #35610, merged 2025-10-29).
3. On niri, use the **overlay layer with exclusive keyboard focus**, and launch apps through IPC `Spawn` to get focus tokens and systemd scopes.
4. **Separate backends** behind one interface: Wayland data-control vs X11 XFixes for the clipboard, and niri IPC vs `systemd-run`/double-fork for launching.
5. Write your own **Exec tokenizer**.
6. **Licences:** hand-write serde structs for niri IPC instead of linking the GPL `niri-ipc` crate, and don't use evalexpr (AGPL).
7. The watcher needs the **password-hint filter, item size caps and read timeouts** from day one.
8. **X11/DCV has no niri binds**, so the hotkey needs `XGrabKey` or a desktop-environment keybinding. Unverified on Xdcv.
