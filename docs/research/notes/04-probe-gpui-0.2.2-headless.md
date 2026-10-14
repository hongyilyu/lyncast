# Headless GPUI 0.2.2 probe: build, render, memory (prototype agent, 2026-10-14)

Raw agent report, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.

GPUI builds and renders on this box, on both Wayland and X11, through lavapipe. Neither path works out of the box headless: each needed a workaround, described below.

One note first: the `hongyi-mode` skill couldn't be loaded (it's disabled for model invocation), so I ran this as a plain throwaway probe. About 17 minutes wall-clock in total.

**Installed:** the 11 requested packages, plus `libxkbcommon-x11-devel` (the link step needs `-lxkbcommon-x11`), plus 34 dependencies dnf pulled in automatically (glib2-devel, harfbuzz-devel, libicu-devel, weston-libs, freerdp-libs and others). Nothing was upgraded or removed, and there was no GPG prompt. The full list is in `logs/../dnf.log` and `logs/dnf2.log`.

**Source and build:** gpui 0.2.2 from crates.io (Blade renderer over Vulkan). A fresh dependency resolution picked libc 0.2.190, which breaks `xattr` 0.2.3 (`ENOATTR` is missing). I reused the `Cargo.lock` that ships inside gpui 0.2.2 (libc 0.2.177) and it built. Cold `--release` build: **48.0 s wall** (726 s CPU, 495 crates, downloads already cached). Binary: **24.4 MB**, 17.7 MB stripped.

**Did each backend render?**
- **(a) `weston --backend=headless`: no.** gpui panics at `wayland/client.rs:503`, `seat.unwrap()` on `None`. gpui needs a `wl_seat`, headless weston advertises none, and the test-seat plugin isn't packaged.
  - **Workaround:** run `weston --backend=x11` inside Xvfb, which does provide a seat. That rendered. Screenshot: `/tmp/gpui-probe/shots/wayland-screenshot-2026-10-14_14-48-53.png` (header plus about 19 visible rows, text correct).
- **(b) Xvfb with no window manager: only with a nudge.** Layout and paint run, but nothing is ever presented. During `open_window`'s synchronous round trips, the MapNotify event gets read into x11rb's in-process queue. After that the socket never wakes gpui's event loop again, so its refresh loop never starts. Any outside X event unsticks it.
  - **Evidence:** `xprop -set` on the window at 1.5 s produced the first frame at 1495 ms.
  - **Workaround for measuring:** the probe opens a second X connection and changes a property on the window (`PROBE_X11_KICK`). A real window manager would send such events anyway.

**Measurements** (3 runs each, median [min–max], times in ms since exec):

| | Wayland (weston-x11) | X11 (Xvfb + kick) |
|---|---|---|
| first `render()` call | 107.8 [105.8–149.4] | 90.6 [87.8–93.5] |
| first frame presented | **132.3** [130.2–173.4] | **115.6** [112.9–117.3] |
| RSS | **104.4** MiB [104.1–104.7] | 97.0 [96.7–97.0] |
| PSS | **79.7** MiB [79.3–80.0] | 64.2 [63.9–73.0] |
| idle CPU (5 s) | 0.2% [0.2–0.4] | 0.0% [0.0–0.2] |
| threads | 102 | 105 |

Memory was flat across the 2.0/2.5/3.0 s samples (at most 0.04 MiB drift).

**Where the PSS goes** (Wayland, 79.9 MiB total):

| MiB | What |
|---|---|
| 29.8 | libLLVM-15 (lavapipe's shader compiler) |
| 20.8 | heap |
| 12.0 | the probe binary's code |
| 6.2 | libvulkan_lvp |
| 4.6 | other anonymous memory (thread stacks, malloc arenas) |
| 3.2 | swapchain: 4 images of 1.5 MiB, shared with weston |
| ~2 | font files (every DejaVu face is memory-mapped) |

Pss_Shmem shows 14 MiB mostly because `/tmp` is tmpfs, so the binary counts as shared memory rather than file pages.

**Memory by startup stage** (RSS): 5 MiB at `main`, 64 MiB after platform init, 81 MiB once the window and GPU pipelines exist, 98 MiB after the first present. Bare lavapipe init on its own (`vulkaninfo`) peaks at 57 MiB RSS and takes 35 ms.

**Ablations** (Wayland):
- **One lavapipe thread:** threads drop from 102 to 40, PSS only falls by 3 MiB. The other 32 threads are gpui's background workers, sized to the core count.
- **10 vs 1000 rows:** identical, so `uniform_list` virtualizes as expected.
- **100k rows:** +12 MiB, which is the probe's own string data.
- **Cold Mesa shader cache:** first frame at **582 ms** and PSS 95.7 MiB.

**What limits each number, and what carries over to a real niri desktop** (these are estimates, not measured):
- **Memory:** roughly 45–50 MiB of the 80 MiB PSS is lavapipe-specific (LLVM, lvp, shader-compile heap, rasterizer threads, swapchain buffers in CPU memory). That doesn't carry over. Intel's driver doesn't load LLVM, and AMD's uses its own compiler, so LLVM would be mostly untouched and shared. gpui's own share is about 30–35 MiB: binary code (file-backed, reclaimable), font database, glyph atlas and scene. That part does carry over. **Real-GPU estimate: about 35–60 MiB PSS.**
- **Font database:** gpui scans every installed font at startup. This box has 142 faces; a typical desktop with Noto installed has 1000+, so startup time and memory get worse there.
- **Startup:** about 40 ms platform init (35 ms of it is lavapipe device setup), about 60 ms creating the window, surface and pipelines, and about 18 ms of CPU rasterization for 800×500. A real GPU shrinks the first and last; pipeline creation depends on the driver's shader cache, as the 582 ms cold run shows. **Real-GPU estimate: about 60–100 ms cold exec to first frame.** For a Raycast-style resident app, the latency that matters is showing an already-running window, which I didn't measure.
- **Idle CPU:** about 0.2% carries over. gpui doesn't redraw when nothing has changed, though on X11 it still ticks a 16 ms timer while the window is visible.

**For CLI-only testing:** neither issue blocks anything. Headless rendering in CI works if you use Xvfb plus nested weston, or Xvfb plus a kick. gpui also ships a test platform (`src/platform/test`) that needs no display.

Everything is in `/tmp/gpui-probe/`:
- `app/src/main.rs` (the probe)
- `measure.py` (the harness)
- `run.sh` (starts Xvfb and weston, then runs the harness)
- `logs/` (raw results and per-run JSON)
- `shots/` (the screenshot)

I stopped my Xvfb and weston processes.
