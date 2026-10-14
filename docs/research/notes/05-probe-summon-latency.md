# Summon latency, resident vs spawn-per-summon, gpui-pre 0.3.9 (prototype agent, 2026-10-14)

Raw agent report, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.

I've got numbers for both designs. At the first visible frame, A (resident process) is about 45 ms per warm summon and B (fresh process) is about 137 ms per exec. In A, memory goes from 3 MiB to about 57 MiB of anonymous memory (Pss_Anon) after the first summon and never comes back down. There's no per-cycle leak, and idle CPU is roughly zero.

**Setup**
- **GPUI:** crates.io `gpui-pre =0.3.9` + `gpui-pre-platform =0.3.9` (features font-kit, wayland, runtime_shaders). It built and ran in release on the first try, so no fallback to the zed path deps.
- **Adapter:** wgpu found both Vulkan lavapipe and GL llvmpipe and picked **Vulkan lavapipe** (LLVM 15). It also creates a GL/EGL context during enumeration, which adds a second set of 32 llvmpipe threads.
- **Layer shell under anvil:** it works. The window is 800x500, horizontally centred, 120 px from the top, on the Overlay layer. Both screenshots show it rendered. One gap: anvil never sent keyboard focus to the window despite Exclusive keyboard mode.

**Present hook.** "Present" means the client's buffer attach + commit, logged from a task spawned in `on_next_frame` that runs after GPUI's draw and present return. A Wayland protocol trace (`WAYLAND_DEBUG`, n=3) confirmed this timestamp matches the real attach + commit within 0.1 ms. The compositor's frame-done event arrived **+2.2 ms** later in all 3 traces. My own frame-done hook reads about 80 ms, but that's inflated: because the window never got focus, GPUI treats it as inactive and caps it at 30 fps. Ignore that number.

**Latency** (all measured; median [min–max])

| Case | to present (ms) | n |
|---|---|---|
| A summon #1 (first window in the process, warm shader cache) | 120.8 [120.1–134.5]; rerun 120.0 | 3+3 procs |
| A summons #2..#20 | 44.9 [44.1–48.9]; rerun 45.3 | 57+57 |
| A, shader cache disabled: #1 / warm | 567 (n=1) / 424.5 [423–427] | 1 proc |
| B exec, warm cache | 137.0 [133.7–147.7] | 10 |
| B cold (`MESA_SHADER_CACHE_DISABLE`) | 571.8 [567–579] | 3 |
| B cold (empty cache dir) | 649.4 [635–652] | 3 |

- **Where B's time goes:** exec to `main` 2.5 ms, to app ready 10.9 ms, to first render 106.6 ms.
- **Warm A breakdown (protocol trace, n=3):** swapchain buffers at 5.6 ms, initial commit at about 38 ms, configure about 2.4 ms later, buffer commit at 48.5–50.4 ms.
- **What the warm A time is (sampled):** in 60 stack samples taken 6–36 ms after toggle, 47 (78%) were in GPUI's per-window pipeline creation going into `vkCreateGraphicsPipelines`. Of those, 38 were inside lavapipe's SPIR-V→NIR compile and 9 on the wgpu/naga side.
- **Why that repeats every summon:** reading gpui-wgpu 0.3.9, each new window compiles its shader module and pipelines and gets its own glyph atlas. Only the GPU device is shared, and it stays alive after the window closes. With the Mesa cache off, every A summon pays about 390 ms extra.
- **Inferred:** summon #1 costs about 58 ms more than a warm summon; that difference is the one-time GPU setup (Vulkan instance, adapter enumeration including GL, device creation).

**Memory** (MiB, median of 3 processes, spread ≤0.6)

| State | PSS | Private_Dirty | Anon (Pss_Anon) |
|---|---|---|---|
| A hidden, before any summon | 11.9 | 11.8 | 3.0 |
| A shown #1 | 94.3 | 64.5 | 44.2 |
| A hidden after 1 close | 93.6 | 64.6 | 44.2 |
| A shown #20 | 107.6 | 77.8 | 57.3 |
| A hidden after 70 cycles | 106.9 | 77.8 | 57.3 |
| B shown, warm cache | 94.3 | 64.5 | 44.0 |
| B shown, cold cache | 111.7 | 79.3 | 58.9 |

- **Leak check:** memory grows +5 MiB on cycle 2, the growth shrinks, and it plateaus by about cycle 10. From cycle 21 to 70 the slope was −0.6 to +7 kB/cycle across 6 processes, so the ~4 MB/summon leak does not reproduce here. Threads go from 40 to 172 at the first summon and stay flat.
- **Caveat:** `/tmp` is tmpfs, so the binary's own pages (about 10 MiB, 17 MiB after a summon) show up as Private_Dirty. On a normal disk install they'd be clean page cache, so use the Anon column as the real cost.
- **A hidden, before any summon (PSS):** binary 9.8 MiB (the tmpfs artifact), [anon] 1.1, [heap] 0.9, libxkbcommon 0.14. GPU setup is lazy, so nothing GPU-related is loaded yet.
- **A hidden, after 70 cycles (PSS):** [heap] 41.8 (29.0 after the first close), libLLVM 23.0 (6.6 dirty), binary 16.9, [anon] 7.7, libvulkan_lvp 6.4, [memfd] 5.0 (lavapipe device memory), libgallium 4.5.

**Idle CPU (A hidden, 10 s, n=3 each):** 0.0%, which is below the 10 ms tick resolution, so under 0.1%. 2.0 context switches/s, both before any summon and after 70 cycles.

**What should carry over to a real niri GPU desktop**
- **Should hold (structural, measured):**
  - A's footprint before the first summon is tiny.
  - After the first summon A holds about what B holds while shown, and never returns to the pre-summon level.
  - Idle CPU is near zero.
  - No per-cycle drift in this GPUI version.
  - GPUI rebuilds pipelines and the glyph atlas on every window open.
  - B always pays process start, GPU setup and pipeline creation.
  - Shader-cache warmth dominates cold starts.
- **Lavapipe-only, so don't trust the absolute values (inferred):**
  - The ~30 ms of pipeline creation and the ~9–22 ms first draw are software rendering and software compile. A warm driver cache on radv/anv should make A noticeably faster than 45 ms.
  - The memory split (libLLVM, gallium, memfd) will be replaced by whatever the real driver uses. Re-measure on the real machine.
  - Frame-done at +2.2 ms is anvil-specific. niri shows the frame at the next vblank, up to about 16.7 ms at 60 Hz.
  - niri does give Exclusive layer surfaces keyboard focus, so the 30 fps cap shouldn't apply there. It never affected the first present anyway.

Xvfb, anvil and all probe processes are stopped.

Files are in /tmp/gpui-summon:
- **Source:** `app/src/main.rs`, `app/Cargo.toml`
- **Harness:** `measure.py`, `wldebug.py`, `pmp.py`, `xwd2png.py`, `start_display.sh`, `run.sh`
- **Logs** (in `logs/`):
  - `a_summary.txt`, `a_rerun_summary.txt`, `a_rerun.json`
  - `b_summary.txt`, `b.json`
  - `a_nocache_summary.txt`, `a_nocache.json`
  - `wldebug_resident.err`, `pmp_stacks.txt`
- **Screenshots:** `shots/oneshot.png`, `shots/resident_summon3.png`
