# Landscape of Linux launchers (research agent, 2026-10-14)

Raw agent report, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.

I checked the projects mostly through `gh`, plus a few web pages. WebSearch wasn't available, so I couldn't look at Reddit or HN reports. The `hongyi-mode` skill can't be called by an agent (it is set to "disable-model-invocation"), so the user would have to run `/hongyi-mode` themselves. Scratch notes are in `/tmp/research-landscape/notes.md`.

## Comparison (as of 2026-10-14)

Features column: A=app launch, F=fuzzy search, C=clipboard history, K=calculator, Fi=file search, X=extensions. All numbers are reported by the project or its users; I measured none myself.

| Project | Stack | Activity | Features | Architecture / hotkey / window | Numbers (reported) |
|---|---|---|---|---|---|
| [Vicinae](https://github.com/vicinaehq/vicinae) | C++/Qt, QML UI since [v0.20](https://github.com/vicinaehq/vicinae/releases/tag/v0.20.0); Node runs extensions | 10.2k★, v0.29.1 2026-09-30; macOS since [v0.23](https://github.com/vicinaehq/vicinae/releases/tag/v0.23.0) | all; calculator uses [qalculate](https://github.com/vicinaehq/vicinae/tree/main/src/server/src/services/calculator-service/qalculate); X runs Raycast extensions | `vicinae server` daemon, bind `vicinae toggle`. Layer shell; on niri switch to `overlay` to show over fullscreen apps ([FAQ](https://docs.vicinae.com/faq)). Its own hotkey protocol is [merged in Hyprland](https://github.com/hyprwm/Hyprland/pull/15010) but [still open for niri](https://github.com/niri-wm/niri/pull/4145) | No idle figure found. 1.2–2.7 GB while scrolling images on niri, about 525–750 MB after the fix ([#998](https://github.com/vicinaehq/vicinae/issues/998)) |
| [Walker](https://github.com/abenz1267/walker) + [Elephant](https://github.com/abenz1267/elephant) | Rust GTK4 front end, Go backend | v2.17.2 2026-10-08; Elephant says "[Development is on hold](https://github.com/abenz1267/elephant/commit/a35b3b54ae8bfddaa3bae82d1409cf2d6dfebf3b)" (2026-09-29) | all; calculator via qalc; X = Elephant providers | `--gapplication-service`, then D-Bus or `nc -U` socket ([README](https://github.com/abenz1267/walker)); gtk4-layer-shell | 300 ms before `main()`, so the first keystrokes are lost ([#494](https://github.com/abenz1267/walker/issues/494)); old versions grew to 9–14 GB ([#487](https://github.com/abenz1267/walker/issues/487)) |
| [Gauntlet](https://github.com/project-gauntlet/gauntlet) | Rust iced, Deno/React plugins | "no longer being developed" (2025-10-02) | A, K (numbat), X; no C or Fi ([#15](https://github.com/project-gauntlet/gauntlet/issues/15)) | `gauntlet open`; no global hotkey on Wayland ([support matrix](https://gauntlet.sh/docs/feature-support)) | none |
| [Anyrun](https://github.com/anyrun-org/anyrun) | Rust GTK4 | v26.6.1 2026-06-09 | A, K (rink), Fi (kidex), X (.so plugins); no C | Daemon mode since [v25.12.0](https://github.com/anyrun-org/anyrun/releases/tag/v25.12.0); layer shell | none |
| **[zlaunch](https://github.com/zortax/zlaunch)** | Rust, GPUI (Zed git) + gpui-component | 227★, v0.5.1 2026-03-27, last commit 2026-06-18 | A, F, C, K (fend); windows incl. niri; no X | Daemon + `zlaunch toggle`; full-screen layer shell | 270 MB, rising to 1.1 GB after emoji; owner sees about 300 MB ([#28](https://github.com/zortax/zlaunch/issues/28)); about 220 MB VRAM at 1080p ([#27](https://github.com/zortax/zlaunch/issues/27)) |
| **[look](https://github.com/kunkka19xx/look)** | Rust core; Linux UI on GPUI-CE | 1.16k★; GPUI port [merged 2026-10-12](https://github.com/kunkka19xx/look/pull/594), not yet in a release (v0.7.2 is still Tauri) | A, F, C, K, Fi; no store | Resident; D-Bus `Toggle` bound in niri config; layer surface created on show, destroyed on hide ([gpui README](https://github.com/kunkka19xx/look/blob/main/apps/linows/gpui/README.md)) | 43 MB PSS / 72 MB RSS when hidden on swayfx, 0 CPU; frames 16.6 ms on niri (same README) |
| [Loungy](https://github.com/MatthiasGrandl/Loungy) | GPUI | **archived**; author points to Gauntlet and Vicinae | A, C (macOS only), K | Linux "very rough" | none |
| [sherlock-gpui](https://github.com/Skxxtz/sherlock-gpui) | GPUI rewrite of the GTK4 [Sherlock](https://github.com/Skxxtz/sherlock) | 6★, v0.2.3 | A, K (meval) | layer shell | none |
| [Albert](https://github.com/albertlauncher/albert) | C++/Qt | v35.1.0 2026-07-25 | all, via [plugins](https://github.com/albertlauncher/plugins) | Resident; `toggle` over an IPC socket ([source](https://github.com/albertlauncher/albert/blob/main/src/app/application.cpp)) | none |
| [Ulauncher](https://github.com/Ulauncher/Ulauncher) | Python GTK | v6 beta35 2026-09-21 | A, F, K, Fi, X; clipboard only via extension | `ulauncher-toggle`; layer-shell helper | none |

Others:
- **Active:** [fuzzel](https://codeberg.org/dnkl/fuzzel) (C, starts fresh each call, 1.15.0) and [raffi](https://github.com/chmouel/raffi) (iced or fuzzel front end).
- **Stale:** [onagre](https://github.com/onagre-launcher/onagre) (last commit 2025-04), [Kunkun](https://github.com/kunkunsh/kunkun) (Tauri, last release 2025-04) and [Sunbeam](https://github.com/pomdtr/sunbeam) (last commit 2025-05).
- **Raycast:** "We currently don't have plans to bring Raycast to Linux" ([FAQ](https://www.raycast.com/faq)).

**Not verified:** idle memory for Vicinae, Walker and Albert; whether Albert uses layer shell or a Wayland hotkey.

## GPUI on Linux: what people ran into
- **Crate versions:** the crates.io `gpui` 0.2.2 ([2025-10-22](https://crates.io/crates/gpui)) predates layer-shell support, which [merged 2025-10-29](https://github.com/zed-industries/zed/pull/35610). Layer-shell sizing fixes are still open ([#48466](https://github.com/zed-industries/zed/pull/48466), [#64994](https://github.com/zed-industries/zed/pull/64994)), and so is color emoji ([#62803](https://github.com/zed-industries/zed/pull/62803)). So you either pin a Zed git revision or use [gpui-ce](https://github.com/gpui-ce/gpui-ce).
- **Loungy:** couldn't hide or position its window, and `global-hotkey` doesn't work on Wayland, so the author added `loungy toggle` ([#7](https://github.com/MatthiasGrandl/Loungy/issues/7)). Polling the clipboard hashed big images every second and burned CPU ([#29](https://github.com/MatthiasGrandl/Loungy/issues/29)).
- **zlaunch:** creates a 5680×4320 surface ([window.rs](https://github.com/zortax/zlaunch/blob/main/src/app/window.rs)), which caused crashes and high VRAM use ([#34](https://github.com/zortax/zlaunch/issues/34)). Also froze on niri with NVIDIA ([#15](https://github.com/zortax/zlaunch/issues/15)), and reopening within about 500 ms closes it immediately on niri ([#26](https://github.com/zortax/zlaunch/issues/26)).
- **Crates used:**
  - Loungy: nucleo (fuzzy), numbat (calculator), arboard (clipboard), freedesktop-icons.
  - zlaunch: fuzzy-matcher, fend-core (calculator), arboard with `wayland-data-control`, plus raw wlr data-control for watching the clipboard ([Cargo.toml](https://github.com/zortax/zlaunch/blob/main/Cargo.toml)).
  - look: wl-clipboard-rs and its own matcher ([backend Cargo.toml](https://github.com/kunkka19xx/look/blob/main/apps/linows/backend/Cargo.toml)).

## Lessons to steal
1. **Daemon plus a tiny client.** Make the toggle client tiny: Walker loses about 300 ms before `main()` from dynamic linking, and zlaunch [#45](https://github.com/zortax/zlaunch/issues/45) asks for a separate light client. Bind it in niri with `allow-inhibiting=false` so fullscreen apps can't swallow the key ([look README](https://github.com/kunkka19xx/look)).
2. **Size the layer surface to the panel.** Create it on show and destroy it on hide, but keep the process alive (`QuitMode::Explicit`). This is look's setup at 43 MB PSS.
3. **Use `KeyboardInteractivity::Exclusive`.** GPUI caps an inactive window at 30 fps; Exclusive avoids it (look README).
4. **Watch the clipboard by event, not by polling.** Use data-control events. GNOME doesn't offer data-control, so you need a shell extension there ([Vicinae #1078](https://github.com/vicinaehq/vicinae/issues/1078)). Auto-paste on niri is broken ([Vicinae #869](https://github.com/vicinaehq/vicinae/issues/869), [#994](https://github.com/vicinaehq/vicinae/issues/994)), so ship copy-only first.
5. **Put hard limits on image, icon and emoji caches** (Vicinae #998, zlaunch #28).
6. **Cache GPU pipelines.** look found a shader recompile per window costing about 4 MB per summon, and a new-window-per-summon leak of about 100 MB on Windows.
7. **Build GPUI deps at `opt-level=2` in debug.** Unoptimised text painting made frames take 110 ms (look).

## The gap, honestly
It is **small**. On niri, Vicinae already covers all four MVP features and much more, ships every few weeks, and runs Raycast extensions; that last part is its lead and not worth competing on. Two GPUI launchers already aim at this exact niche:
- **zlaunch:** has the full MVP and niri support, but is MIT, quiet since June, and uses about 300 MB.
- **look:** its GPUI shell is days old but measures 43 MB.

Gauntlet is dead and Walker's backend is on hold, which helps any new entrant.

What's left to win is quality, not features:
- A strict, niri-first footprint target, around 50 MB or less idle, which look shows GPUI can hit.
- A client that adds no measurable delay.
- Limited caches.
- Published, reproducible startup and memory benchmarks.

That is a real but narrow niche. Contributing to or forking zlaunch is worth weighing against starting from scratch, unless the goal is mainly learning or full control.
