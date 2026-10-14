# GPUI on Linux (research agent, 2026-10-14)

Raw agent report, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.

I couldn't invoke the `hongyi-mode` skill because it's user-only, so I went ahead without it. Line refs are to zed main at c403bc6 (2026-10-14), cloned in `/tmp/research-gpui/zed`. "Measured" means I ran it on this machine with Mesa 24.2.6 lavapipe, under Xvfb and weston-on-Xvfb, release-fast build. Logs are in `/tmp/research-gpui/measure/`.

## 1. Depending on GPUI
- **crates.io `gpui` is stuck at 0.2.2 (2025-10-22).** That version still uses blade and has no layer shell (no `LayerShell` anywhere in the downloaded crate). Main is still versioned 0.2.2 (`crates/gpui/Cargo.toml:3`). The README tells you to add `gpui_platform` (`README.md:8-13`), but `gpui_platform`, `gpui_linux` and `gpui_wgpu` aren't on crates.io. The CI publish PR #39832 was closed. gpui.rs says "GPUI is tied to Zed".
- **`gpui-pre` 0.3.9 (2026-10-12):** a weekly cron snapshot of zed main (zed@93d7456, 2026-10-10), published by longbridge (`gpui-kit/.github/workflows/release-gpui.yml:4-5`). It includes layer shell, the headless renderer and `request_windowing`. Use `gpui = { package = "gpui-pre", version = "=0.3.9" }` plus `gpui-pre-platform` (`gpui-kit/Cargo.toml:69-70`). Pin exactly: snapshots break the API (`:62-68`).
- **Other options:**
  - Git dep on zed, as zlaunch (an existing GPUI Wayland launcher) does (`zlaunch/Cargo.toml:11`). It drags in a ~536 MB repo.
  - `gpui-component`/`gpui-kit` 0.7.3 (longbridge, 17k stars), built on gpui-pre.
  - `gpui-ce` 0.2.2 (2026-08-28): a diverging fork with its own parley text stack and MSRV 1.95; its crates.io releases lag the repo.
  - Also seen on crates.io: `gpui-fast` 0.1.8 and `gpui-unofficial` 1.24.1-pre.
- **Toolchain:** edition 2024 (`Cargo.toml:275`), no `rust-version`, zed pins 1.98.1 (`rust-toolchain.toml`).
- **Build, measured:** clean build of 526 crates took 3m52s wall on 32 cores; incremental test builds took ~27s. Stripped hello_world is 26.5 MB.
- **System deps:** the binary only links libxcb, libxkbcommon(-x11), libXau and libxcb-xkb; Wayland, Vulkan and EGL are dlopened. You need xkbcommon/xcb/wayland/fontconfig headers to build, and Vulkan or EGL at runtime.

## 2. Renderer
- **Blade is gone.** The Linux renderer was rewritten on wgpu in #46758 (merged 2026-02-13). It uses wgpu 29 with `gles` and `vulkan` features (`gpui_wgpu/Cargo.toml:45`) and creates the instance with `VULKAN | GL` (`wgpu_context.rs:474`).
- **Vulkan isn't strictly required.** Measured: with Vulkan ICDs disabled, it fell back to GL through EGL (Mesa swrast) on Wayland and committed frames. On X11 the process stayed up with EGL loaded, but I couldn't confirm it drew anything.
- **lavapipe works (measured, X11 and Wayland).** CPU adapters rank last but are accepted (`wgpu_context.rs:616-623`); they're only rejected when recovering from device loss (`wgpu_renderer.rs:2230`).
- **Watch PR #63346 (open).** It would disable software ICDs at startup. It's meant to fail open, but it could affect lavapipe in CI (unverified).
- Zed's Linux docs (`docs/src/linux.md:143`) still say it renders with Vulkan, which is out of date.

## 3. Wayland
- **API:** `WindowKind::LayerShell(LayerShellOptions)` (`platform.rs:2725-2728`). Fields (`platform/layer_shell.rs:57-77`): `namespace`, `layer` (default Overlay), `anchor` bitflags, `exclusive_zone`, `exclusive_edge`, `margin`, `keyboard_interactivity` = `None | Exclusive | OnDemand` (OnDemand is the default).
- **History:** landed in #35610 (merged 2025-10-29, after the 0.2.2 release), then #60163 (exclusive zone/edge) and #50520 (`display_id`).
- **Unsupported compositors** get `LayerShellNotSupportedError` (measured on weston). Example: `examples/layer_shell.rs:87`.
- **Open problems:**
  - Keyboard interactivity can't be changed at runtime yet (#61420, tested on niri).
  - Sizing bugs: #64994, #61368, #48466. zlaunch works around them by requesting an 8K surface anchored to all edges (`zlaunch/src/app/window.rs:75-101`).
- **Transparency and blur:** `Transparent` works. `Blurred` only speaks `org_kde_kwin_blur` (`wayland/client.rs:77`). niri's blur (#3483, v26.04) uses ext-background-effect, and zed's PRs for that (#53746, #59842) are still open. On niri, set `layer-rule { background-effect { blur true } }` instead (niri `Window-Effects.md`).
- **Decorations:** client or server side via xdg-decoration (`platform.rs:2553`). Doesn't matter for layer surfaces.
- **xdg-activation** is implemented (`wayland/window.rs:1859-1874`), and a token can be passed via `GraphicalEnvironment.activation_token`.
- **niri issues:**
  - The NVIDIA freeze was fixed by #50640 (#44814, #50574).
  - The transparency border was a niri config issue (#50447).
  - Still open: IME preedit #65302, AMD artifacts #63358.

## 4. Clipboard
- `ClipboardEntry` = `String | Image | ExternalPaths` (`platform.rs:3010-3017`).
- **Reading:** Wayland reads text or image (`wayland/clipboard.rs:193-209`).
- **Writing is text only on both backends:** Wayland `send` (`:177-181`) and X11 (`x11/client.rs:1839-1851`).
- Primary selection is supported.
- **No change-observer API.** For a clipboard history you need a separate wlr/ext-data-control client. zlaunch runs one on its own thread (`zlaunch/src/clipboard/monitor.rs:13-16`).

## 5. Lists and inputs
- `uniform_list(id, count, |range, …|)` (`elements/uniform_list.rs:22`) measures the first row and renders only the visible range, so 10k items is fine. `list()` + `ListState` handles variable row heights (`elements/list.rs:24,314`).
- **No reusable TextInput in core.** `examples/input.rs` is 784 lines of copy-paste with IME marked ranges, and has an open bug (#64389).
- gpui-component provides `Input`/`InputState` (`gpui-kit/crates/base/src/input/mod.rs:100`) and a virtualized `List` (`component/src/list/list.rs:554`).

## 6. Testing
- **External crates can use it.** The `test-support` feature (`gpui/Cargo.toml:21`) exposes `TestAppContext`, `VisualTestContext`, `TestApp`, `HeadlessAppContext` and `#[gpui::test]` (`app.rs:32-41`). gpui-kit already does this from outside zed (`crates/kit/Cargo.toml:27`, `shell/tests/integration/`).
- `VisualTestAppContext` (real rendering) is macOS-only (`app.rs:40`).
- **Measured, with no DISPLAY or WAYLAND_DISPLAY:**
  - The `testing` example passed 7/7.
  - The `gpui_wgpu` headless_renderer tests passed 4/4 on lavapipe, including pixel readback (#64718). `current_headless_renderer()` (`gpui_platform.rs:110-128`) supports screenshots with real text shaping.
- **Real-platform headless mode:** `WindowingRequest::Headless` and `App::request_windowing` (`app.rs:238,1248`; #64969). Windows lay out and take input but draw nothing.
- **Can test headlessly:** actions, keymaps, focus, simulated keystrokes, async work with a seeded scheduler (`SEED`), layout, offscreen pixels.
- **Can't test headlessly:** layer-shell protocol, compositor focus, IME, clipboard between processes, real presentation.
- **Caveat:** GPUI panics if the compositor has no `wl_seat` (measured on headless weston; `platform.rs:366`). Layer-shell end-to-end tests would need a headless wlroots compositor such as sway (unverified).

## 7. Footprint
- **Measured, inflated by lavapipe:**
  - hello_world RSS: 146 MB on X11, 140 MB on Wayland, 155 MB on the GL fallback. uniform_list: 128 MB.
  - Idle CPU: 0–0.4%.
  - First buffer commit ~140–160 ms after the first Wayland request.
- **What's in it:** libLLVM 51 MB (lavapipe's JIT), heap 28–45 MB, shmem ~22 MB, binary 17 MB, gallium 8–11 MB.
- **Threads:** 173 here. GPUI spawns one worker per CPU (`dispatcher.rs:34`); the count drops to 49 with `LP_NUM_THREADS=1`.
- **Reported:**
  - #63327 (niri, Intel): unused lavapipe+LLVM (~25 MiB) and EGL/gallium (~9 MiB) stay mapped even when a hardware GPU is in use. PR #63346 would save ~35 MiB.
  - zlaunch daemon: ~270–300 MB, rising to 1.1 GB after loading emoji (zlaunch#28).
- The glyph atlas is per window (`wgpu_renderer.rs:338`); PR #60791 (atlas memory retention) is open.
- I found no reported hardware-GPU numbers for a minimal app (unverified).

## 8. Linux pain points
- **Hide/show doesn't exist on Linux.** `App::hide` and `activate` are no-ops (`gpui_linux/src/linux/platform.rs:701-707`). `WindowOptions.show` is only read on Windows (`gpui_windows/src/window.rs:578`). You have to `remove_window` and reopen, which is what zlaunch does (`window.rs:141-145`). Reopening reuses the GPU device (`wgpu_renderer.rs:329-336`) but builds a new glyph atlas.
- **Fractional scaling:** supported via wp_fractional_scale (`wayland/window.rs:853,1208`), but #65032 (blurry at fractional scale) is open.
- **IME:** open issues #65302, #64850, plus fcitx5 on X11 (#58192, #60578).
- **Fonts:** cosmic-text + swash. Subpixel AA needs dual-source blending (`wgpu_context.rs:426-432`). See also #57777 (atlas blur) and colour emoji trouble in zlaunch#23.
- **Multi-monitor:** `displays()` is empty at startup, which breaks choosing a layer-shell output (#46378 open; fix PR #61578 open).
- **Idle CPU:** near zero (measured).

## Recommendation
Depend on `gpui-pre` pinned to `=0.3.9`, with `gpui-pre-platform` features `wayland,x11`. It's the same code as a zed git rev, but published on crates.io, reproducible, and matching gpui-component. Add gpui-component only if you want its IME-ready `Input`. Avoid crates.io `gpui` 0.2.2 (blade, no layer shell) and gpui-ce (diverging, lagging releases).

**Biggest risk:** GPUI on Linux can't hide a window, so every summon creates a new layer surface and glyph atlas. On top of that, the Mesa/driver stack probably sets a memory floor of ~100 MB or more (inferred from the lavapipe runs and #63327). Benchmark a keep-alive daemon that closes and reopens the window on real niri hardware before committing. Weekly snapshot API churn is the second risk.
