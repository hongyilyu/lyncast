# GPUI on Linux runs a launcher but cannot hide a window

This page explains what GPUI on Linux can do for lyncast as of 2026-10-14. It draws on a survey, [notes/02](notes/02-gpui-on-linux.md), and two probes, [notes/04](notes/04-probe-gpui-0.2.2-headless.md) and [notes/05](notes/05-probe-summon-latency.md). The [decision log](../decisions.md) is the source of truth.

All numbers come from a headless dev box that renders with Mesa lavapipe, a software Vulkan driver. Measured means a probe ran it, and inferred or unverified means nobody did. notes/02 cites zed main at c403bc6, which is newer than the `gpui-pre` 0.3.9 snapshot.

## lyncast pins `gpui-pre` to `=0.3.9`

crates.io `gpui` is stuck at 0.2.2, which renders with blade and has no layer shell. Zed's platform crates are not on crates.io, and the publish PR #39832 was closed. longbridge publishes `gpui-pre` as a weekly cron snapshot of zed main. Version 0.3.9 (2026-10-12) is zed@93d7456 and has layer shell, the headless renderer, and `request_windowing`. Snapshots break the API, so lyncast pins it exactly ([notes/02](notes/02-gpui-on-linux.md), [Stack](../decisions.md#stack)):

```toml
gpui = { package = "gpui-pre", version = "=0.3.9" }
```

`gpui-pre-platform` comes at the same version, with the `wayland` and `x11` features that notes/02 recommends.

A git dependency on zed, which zlaunch uses, is the same code in a repo of about 536 MB. `gpui-ce` 0.2.2 is a diverging fork whose releases lag its repo. `gpui-component` 0.7.3 builds on `gpui-pre`, and notes/02 adds it only for its IME-ready `Input`.

notes/02 rates weekly API churn as the second-largest risk.

## The wgpu renderer accepts lavapipe

PR #46758 replaced blade with wgpu 29, and the instance asks for `VULKAN | GL` (`wgpu_context.rs:474`). GPUI ranks CPU adapters last but accepts them (`wgpu_context.rs:616-623`), and notes/02 measured lavapipe working on X11 and Wayland. Without Vulkan ICDs, GPUI fell back to GL through EGL on Wayland.

Open PR #63346 would disable software ICDs at startup. It should fail open, but it could affect lavapipe in CI (unverified). The [budgets](../decisions.md#budgets) run on lavapipe, so #63346 needs a check before the pin moves.

## Layer shell works, with open bugs

The API is `WindowKind::LayerShell(LayerShellOptions)` (`platform.rs:2725-2728`). Its `layer` option defaults to Overlay, and `keyboard_interactivity` defaults to `OnDemand`. Without layer shell, GPUI returns `LayerShellNotSupportedError` (measured on weston, [notes/02](notes/02-gpui-on-linux.md)).

Under anvil, the summon probe rendered an Overlay window, but anvil never gave it keyboard focus. The note infers that niri gives Exclusive layer surfaces focus ([notes/05](notes/05-probe-summon-latency.md)). lyncast uses the Overlay layer with exclusive focus ([Process model](../decisions.md#process-model)).

A launcher hits these open issues ([notes/02](notes/02-gpui-on-linux.md)):

| Issue | Problem |
|---|---|
| #61420 | Keyboard interactivity cannot change at runtime. Tested on niri. |
| #64994, #61368, #48466 | Sizing. zlaunch works around them with an 8K surface anchored to all edges. |
| #46378 | `displays()` is empty at startup, so you cannot pick an output. |
| #53746, #59842 | Open PRs for ext-background-effect, which niri blur uses. `Blurred` speaks only `org_kde_kwin_blur`, so set blur in a niri `layer-rule`. |

## The clipboard API cannot watch for changes

`ClipboardEntry` is `String`, `Image`, or `ExternalPaths` (`platform.rs:3010-3017`). GPUI reads text or images on Wayland. Both backends write text only, and no API reports a change ([notes/02](notes/02-gpui-on-linux.md)). lyncast runs a `wl-clipboard-rs` `Watcher` on its own thread and captures text, images, and files ([Clipboard and paste](../decisions.md#clipboard-and-paste)). To put an image or a file back, lyncast needs a writer other than GPUI (inferred).

## Lists virtualize, and text input is yours to build

`uniform_list` renders only the visible range (`elements/uniform_list.rs:22`), and `list()` with `ListState` handles rows of different heights. In the 0.2.2 probe, 10 rows and 1000 rows cost the same ([notes/04](notes/04-probe-gpui-0.2.2-headless.md)). Core GPUI has no reusable text input. `examples/input.rs` is 784 lines to copy, with an open bug (#64389).

## Tests run without a display, up to the compositor

The `test-support` feature gives external crates `TestAppContext`, `HeadlessAppContext`, and `#[gpui::test]`. With no display, notes/02 measured 7 of 7 passes for the `testing` example and 4 of 4 for the `gpui_wgpu` headless renderer tests on lavapipe. `App::request_windowing` with `WindowingRequest::Headless` lays out windows and takes input but draws nothing. Headless tests cover actions, keymaps, focus, keystrokes, layout, and offscreen pixels. They miss the layer-shell protocol, compositor focus, IME, and the clipboard between processes.

GPUI panics without a `wl_seat`, which headless weston lacks. notes/02 measured the panic, and the 0.2.2 probe hit it at `wayland/client.rs:503` ([notes/04](notes/04-probe-gpui-0.2.2-headless.md)). Tier 2 of the [Testing](../decisions.md#testing) plan runs under Xvfb plus anvil, which offers `wl_seat` and layer shell.

## Footprint on lavapipe is mostly the software stack

notes/02 measured a hello_world at 140 MB RSS on Wayland, 51 MB of it lavapipe's libLLVM JIT. #63327 (niri, Intel) reports that unused lavapipe, LLVM, EGL, and gallium stay mapped even with a hardware GPU. PR #63346 would save about 35 MiB. notes/02 inferred a memory floor of about 100 MB or more.

## Linux cannot hide a window, so lyncast closes and reopens one

`App::hide` and `activate` are no-ops on Linux (`gpui_linux/src/linux/platform.rs:701-707`), and only the Windows backend reads `WindowOptions.show`. zlaunch calls `remove_window` and opens a new window later. The new window reuses the GPU device (`wgpu_renderer.rs:329-336`) but builds a new glyph atlas. notes/02 called the missing hide the biggest risk.

lyncast keeps one resident process that opens a layer-shell window per summon and closes it per dismiss. GPUI on Linux quits when the last window closes, so the app runs with `QuitMode::Explicit` ([Process model](../decisions.md#process-model)).

notes/02 also lists blur at fractional scale (#65032), IME (#65302, #64850), atlas blur (#57777), and AMD artifacts on niri (#63358).

## The 0.2.2 probe shows where startup time and memory go

notes/04 built crates.io `gpui` 0.2.2, which uses Blade over Vulkan. For Wayland the probe used weston's X11 backend inside Xvfb. Bare Xvfb presented only after an outside X event, so the probe sent one (`PROBE_X11_KICK`).

The probe measured 3 runs, as median [min to max]:

| | Wayland (weston-x11) | X11 (Xvfb plus kick) |
|---|---|---|
| First frame presented (ms since exec) | 132.3 [130.2 to 173.4] | 115.6 [112.9 to 117.3] |
| PSS (MiB) | 79.7 [79.3 to 80.0] | 64.2 [63.9 to 73.0] |

These are the largest parts of the Wayland PSS:

| MiB | What |
|---|---|
| 29.8 | libLLVM-15, lavapipe's shader compiler |
| 20.8 | Heap |
| 12.0 | Probe binary code |
| 6.2 | libvulkan_lvp |

A cold Mesa shader cache pushed the first frame to 582 ms.

notes/04 estimates the rest without measurement. About 45 to 50 MiB of the PSS is lavapipe-specific, and GPUI's own 30 to 35 MiB carries over. Startup splits into about 40 ms of platform init (35 ms of it is lavapipe device setup), 60 ms for the window and pipelines, and 18 ms of CPU rasterization. A real GPU shrinks the first and last parts. The real-GPU estimates are about 35 to 60 MiB PSS and 60 to 100 ms from cold exec to first frame. GPUI scans every installed font at startup. The dev box has 142 faces, and a desktop with Noto has 1000 or more.

## A resident process summons in 45 ms against 137 ms for a fresh one

notes/05 compared two designs on `gpui-pre` 0.3.9 under anvil. Design A keeps one resident process that opens a window per summon. Design B execs a fresh process per summon. Present means the buffer attach and commit. A `WAYLAND_DEBUG` trace matched the logged time within 0.1 ms.

The probe measured these, as median [min to max]:

| Case | To present (ms) | n |
|---|---|---|
| A summon #1 | 120.8 [120.1 to 134.5], rerun 120.0 | 3+3 processes |
| A summons #2 to #20 | 44.9 [44.1 to 48.9], rerun 45.3 | 57+57 |
| B exec, warm cache | 137.0 [133.7 to 147.7] | 10 |
| B, shader cache disabled | 571.8 [567 to 579] | 3 |

In warm A, 47 of 60 stack samples (78%) were in per-window pipeline creation (`vkCreateGraphicsPipelines`), mostly lavapipe's SPIR-V to NIR compile. The note read gpui-wgpu 0.3.9 in source. Each new window compiles its pipelines and gets its own glyph atlas, and only the GPU device is shared. With the Mesa cache off, each A summon pays about 390 ms more. The note infers that the extra 58 ms of summon #1 is one-time GPU setup.

Memory is in MiB, median of 3 processes:

| State | PSS | Anon (Pss_Anon) |
|---|---|---|
| A hidden, before any summon | 11.9 | 3.0 |
| A hidden after 1 close | 93.6 | 44.2 |
| A hidden after 70 cycles | 106.9 | 57.3 |
| B shown, warm cache | 94.3 | 44.0 |

notes/05 takes the Anon column as the real cost, because PSS counts the binary on tmpfs.

look's reported leak of about 4 MB per summon ([notes/01](notes/01-landscape.md)) did not reproduce. Memory plateaus by about cycle 10, and from cycle 21 to 70 the slope was -0.6 to +7 kB per cycle across 6 processes. Threads go from 40 to 172 at the first summon and stay flat. Idle CPU while hidden was under 0.1%. The [Budgets](../decisions.md#budgets) table's first limits came from these runs.

## What transfers to a real GPU

notes/05 expects these structural results to hold on niri with a hardware GPU:

- A is small before the first summon. After it, A holds about what B holds while shown and never drops back.
- Each window open rebuilds pipelines and the glyph atlas.
- Memory has no per-cycle drift, and idle CPU stays near zero.
- Shader-cache warmth dominates cold starts.

It infers that the absolute values are lavapipe artifacts:

- About 30 ms of pipeline creation and 9 to 22 ms of first draw are software work. A warm radv or anv cache should make A faster than 45 ms.
- The real driver's libraries replace libLLVM, gallium, and memfd in the memory split.
- anvil sent frame-done 2.2 ms after commit. niri shows the frame at the next vblank, up to about 16.7 ms at 60 Hz.

The structural results decide the design. B pays everything a warm A summon pays, plus process start and GPU setup, so a faster driver cannot reverse the order. The absolute numbers still need a run on Hongyi's niri machine ([Budgets](../decisions.md#budgets)).
