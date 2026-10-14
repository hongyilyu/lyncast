# Why lyncast exists when Linux already has launchers

The lyncast MVP has four features on niri. They are app launch, fuzzy search, clipboard history, and a calculator ([decisions](../decisions.md#scope-of-the-mvp)). Five Linux launchers already list all four, and two of them use GPUI. The features are taken, so lyncast competes on quality.

[notes/01](notes/01-landscape.md) is a research agent's survey from 2026-10-14. The agent worked mostly through `gh`, without Reddit or HN. Projects or their users reported every number in the first table, and the agent measured none.

## Five launchers already ship all four MVP features

Coverage lists only what the note records.

| Project | Stack | MVP coverage | Architecture | Reported memory | Status |
|---|---|---|---|---|---|
| [Vicinae](https://github.com/vicinaehq/vicinae) | C++, Qt, QML. Node runs Raycast extensions. | All four. qalculate calculator. | `vicinae server` daemon plus `vicinae toggle`. Layer shell. | No idle figure. 1.2 to 2.7 GB while scrolling images, 525 to 750 MB after a fix ([#998](https://github.com/vicinaehq/vicinae/issues/998)). | Active, v0.29.1 on 2026-09-30 |
| [Walker](https://github.com/abenz1267/walker) and Elephant | Rust GTK4 front end, Go backend | All four. qalc calculator. | `--gapplication-service`, then D-Bus or a socket. gtk4-layer-shell. | No idle figure. Old versions grew to 9 to 14 GB ([#487](https://github.com/abenz1267/walker/issues/487)). | Walker v2.17.2 on 2026-10-08. Elephant development "is on hold". |
| [Gauntlet](https://github.com/project-gauntlet/gauntlet) | Rust iced, Deno plugins | Apps, numbat calculator. No clipboard. | `gauntlet open`. No global hotkey on Wayland. | None | "no longer being developed" since 2025-10-02 |
| [Anyrun](https://github.com/anyrun-org/anyrun) | Rust GTK4 | Apps, rink calculator. No clipboard. | Daemon mode, layer shell | None | v26.6.1 on 2026-06-09 |
| [Albert](https://github.com/albertlauncher/albert) | C++, Qt | All four, through plugins | Resident, `toggle` over an IPC socket | Not verified | v35.1.0 on 2026-07-25 |
| [Ulauncher](https://github.com/Ulauncher/Ulauncher) | Python GTK | Apps, fuzzy search, calculator. Clipboard by extension. | `ulauncher-toggle`, layer-shell helper | None | v6 beta35 on 2026-09-21 |
| [zlaunch](https://github.com/zortax/zlaunch) | Rust, GPUI from Zed git | All four. fend calculator. | Daemon plus `zlaunch toggle`. Full-screen layer shell. | 270 MB, 1.1 GB after emoji, about 300 MB for the owner ([#28](https://github.com/zortax/zlaunch/issues/28)) | MIT. Last commit 2026-06-18. |
| [look](https://github.com/kunkka19xx/look) | Rust core, GPUI-CE UI on Linux | All four, plus file search | Resident. niri binds a D-Bus `Toggle`. | 43 MB PSS, 72 MB RSS when hidden on swayfx ([README](https://github.com/kunkka19xx/look/blob/main/apps/linows/gpui/README.md)) | GPUI port merged 2026-10-12, unreleased |
| [Loungy](https://github.com/MatthiasGrandl/Loungy) | GPUI | Apps, calculator. Clipboard on macOS only. | Linux is "very rough" | None | Archived |
| [sherlock-gpui](https://github.com/Skxxtz/sherlock-gpui) | GPUI | Apps, meval calculator | Layer shell | None | v0.2.3 |

Raycast's [FAQ](https://www.raycast.com/faq) says "We currently don't have plans to bring Raycast to Linux." lyncast models its behaviour on TinyCast instead and ports none of TinyCast's AGPL-3.0 code ([decisions](../decisions.md#project)).

## The gap is small, so lyncast competes on footprint

[notes/01](notes/01-landscape.md) calls the gap small. On niri, Vicinae already covers the MVP and ships every few weeks. The note judges its lead, Raycast extensions, not worth competing on. zlaunch has the full MVP and niri support, but it is MIT, quiet since June, and uses about 300 MB. look's GPUI shell is days old but reports 43 MB. Gauntlet is dead and Walker's backend is on hold.

The note says the rest is quality:

- An idle footprint of around 50 MB or less, which look shows GPUI can hit.
- A toggle client that adds no measurable delay.
- Limited caches.
- Published, reproducible startup and memory benchmarks.

The note calls the result "a real but narrow niche". Hongyi chose a new project over contributing to look or forking zlaunch ([decisions](../decisions.md#project)). lyncast may still reuse code from both, with attribution.

The [budgets](../decisions.md#budgets) cap a warm summon at 50 ms to first frame, PSS before the first summon at 15 MiB, and private memory after summons at 60 MiB. On lavapipe, the dev box measured 45 ms, 12 MiB, and 57 MiB. Real numbers come from Hongyi's niri machine, so these figures do not yet compare with look's 43 MB.

The budgets carry most of the case for lyncast. If lyncast misses them, look covers the same four features once its GPUI port ships.

## zlaunch and look route prefixes to one feature and fan out the rest

[notes/09](notes/09-input-box-prior-art.md) read both input boxes in source. zlaunch was at commit `dac7173` from 2026-06-19, and look was at its main branch on 2026-10-14.

| Concern | zlaunch | look |
|---|---|---|
| Prefix routing | `src/search/detection.rs` takes the first matching prefix, such as `!ai` or `!g`, and shows only that row or provider. | Prefixes such as `c"` for clipboard and `a"` for apps come from one `MODES` table in `core/engine/src/modes.rs`. A scoped query skips URL and calculator rows. |
| Plain query | `src/ui/delegates/dynamic_items.rs:34-92` filters apps, windows, and commands, runs the calculator, and adds AI and web-search fallbacks. | `apps/linows/gpui/src/query.rs:93-127` searches apps and files, prepends quick folders, classifies URLs, and merges recent URLs by score. |
| Calculator gate | `fend_core::evaluate_with_interrupt` with a 50 ms timeout, only if the query has a digit | `is_math`, tested against a corpus of queries that must not be math |
| Matcher | `fuzzy-matcher` 0.3 `SkimMatcherV2`, plus name and prefix bonuses in `item_filter.rs` | Its own `look-matching` DP scorer, with Unicode folded first |
| Calculator row | Second section, after Best Match | Index 0, when `calc_inline` returns an answer |

Both launchers route an explicit prefix to one feature and run every feature on a plain query. Neither picks a feature by sniffing keywords. look explains the top slot in a code comment. It reads "an expression is a question, and the answer outranks a file that fuzzy-matched some of its digits."

lyncast takes the same two steps ([decisions](../decisions.md#input-box)). Tab or a registered prefix routes the query to one feature. Otherwise every root feature evaluates the query, and a merge step orders the rows by a fixed section table of Answer, Apps, and Fallback. Fan-out costs under 0.1 ms per keystroke. Each feature's own parser decides whether the query is its input, so the calculator stays silent on `xterm`, `7zip`, `0x`, and `12 x`.

look's corpus in `core/calc/tests/intent.rs` is the idea most worth copying. It holds real launcher queries that must not be math, such as `2026-08-04`, `1.2.3`, `192.168.1.1`, and `covid-19`. Its rule is "shape decides, spacing never does". The corpus also lists `1920x1080` as not math. Hongyi decided the other way, so any `NxN` shape is math in lyncast ([decisions](../decisions.md#calculator-engine)).

zlaunch shows "Infinity" for division by zero. lyncast shows no card, because `10/0` is a prefix of `10/0.5` ([lead default](../decisions.md#lead-defaults-hongyi-can-override)). lyncast matches with `frizbee` plus TinyCast's ranking rules ([decisions](../decisions.md#stack)).

## lyncast copies look's window model and learns from the others' bugs

The lessons come from [notes/01](notes/01-landscape.md).

- **look.** look keeps its process alive and creates the layer surface on each show, at 43 MB PSS. lyncast uses the same model with `QuitMode::Explicit` ([decisions](../decisions.md#process-model)). `KeyboardInteractivity::Exclusive` avoids GPUI's 30 fps cap on inactive windows. A shader recompile per window cost about 4 MB per summon, so GPU pipelines need a cache. In debug builds, GPUI dependencies need `opt-level=2`, or text painting takes 110 ms a frame. Its README binds the toggle in niri with `allow-inhibiting=false`, so fullscreen apps cannot swallow the key.
- **Walker.** Walker loses about 300 ms before `main()` to dynamic linking and drops the first keystrokes ([#494](https://github.com/abenz1267/walker/issues/494)). The `lyncast toggle` client stays thin and talks to the daemon over a Unix socket ([decisions](../decisions.md#process-model)).
- **zlaunch.** Its 5680×4320 surface caused crashes and high VRAM use ([#34](https://github.com/zortax/zlaunch/issues/34)). The lesson is to size the layer surface to the panel.
- **Vicinae.** Its image memory and zlaunch's emoji growth argue for hard limits on image, icon, and emoji caches. Vicinae needs the Overlay layer on niri to show over fullscreen apps, and lyncast uses the Overlay layer for the same reason ([decisions](../decisions.md#process-model)). Its auto-paste on niri is broken ([#869](https://github.com/vicinaehq/vicinae/issues/869)), so the note advises copy only first, as the lyncast MVP does ([decisions](../decisions.md#scope-of-the-mvp)).
- **Loungy and Gauntlet.** Neither has a global hotkey on Wayland, so Loungy added `loungy toggle` ([#7](https://github.com/MatthiasGrandl/Loungy/issues/7)). In lyncast, a niri bind runs `lyncast toggle` ([decisions](../decisions.md#process-model)). Loungy's clipboard polling hashed big images every second and burned CPU ([#29](https://github.com/MatthiasGrandl/Loungy/issues/29)). lyncast watches by event, with a capture thread that owns the `wl-clipboard-rs` `Watcher` ([decisions](../decisions.md#clipboard-and-paste)).
- **The rest.** The note draws no lesson from Anyrun, Albert, Ulauncher, or sherlock-gpui.
