# Decision log

This file is the source of truth for what lyncast has decided. Other docs explain and expand these entries. When a doc disagrees with this file, this file wins and the doc is wrong.

Each entry says who decided it and on what evidence:

- **User**: Hongyi decided it.
- **Measured**: a prototype or benchmark on the dev box settled it. The note holds the numbers.
- **Lead**: the lead engineer chose a default. Hongyi can override it.

All entries below were made on 2026-10-14 unless they say otherwise.

## Project

| Decision | Who | Evidence |
|---|---|---|
| Build a new project instead of contributing to look or forking zlaunch. | User | [notes/01-landscape.md](research/notes/01-landscape.md) |
| Name it `lyncast`: Lyn(ux) + lynx + cast. | User | |
| License it GPL-3.0-or-later. The GPL `niri-ipc` crate is allowed. | User | |
| Model it on TinyCast (`abue-ammar/tinycast`). TinyCast is AGPL-3.0, so its docs are a behaviour spec only and none of its code is ported. | User | [notes/06-tinycast-behaviour-spec.md](research/notes/06-tinycast-behaviour-spec.md) |
| Code from look (GPL-3.0) and zlaunch (MIT) may be reused with attribution. | Lead | Licences are compatible with GPL-3.0. |

## Scope of the MVP

| Decision | Who |
|---|---|
| Features: app launcher, fuzzy search, clipboard history, calculator. | User |
| No extensions, no AI, no window management, no file search. System actions (lock, suspend, log out, reboot, power off) are in, as built-in commands. | User |
| niri on Wayland only. X11 (a DCV session) and other wlroots compositors come later. | User |
| Clipboard delivery is copy only. Enter puts the entry back on the clipboard and closes the palette. | User |
| Paste must plug in later per platform without a redesign. | User |
| Calculator supports `+`, `-`, `*`, and `/` only. Square root left on 2026-10-15. | User |
| Ranking uses frecency and TinyCast's matching rules. User aliases and favorites come right after the MVP, with a settings surface to edit them. | User |
| The calculator reads the root search input and answers in a card at index 0, as TinyCast does. Units and currency come later in the same engine, as new kinds of quantity, not as separate features. | User |

## Process model

| Decision | Who | Evidence |
|---|---|---|
| One resident GPUI process. Each summon opens a new layer-shell window and each dismiss closes it, because GPUI on Linux cannot hide a window. | Measured | Warm summon 45 ms vs 137 ms for spawn-per-summon, on lavapipe. [notes/05-probe-summon-latency.md](research/notes/05-probe-summon-latency.md) |
| The window is a wlr-layer-shell surface on the Overlay layer with exclusive keyboard focus. | Lead | niri hides the Top layer behind fullscreen windows. [notes/03-wayland-niri-integration.md](research/notes/03-wayland-niri-integration.md) |
| A thin `lyncast toggle` client talks to the daemon over a Unix socket. A niri bind runs it. | Lead | Wayland has no global hotkeys. |
| `--headless` runs the core, the clipboard watcher, and the socket without GPUI, for tests. | Lead | |
| Every palette action also exists as a CLI command over the same socket protocol. | Lead | Makes CLI integration tests exercise the real path. |
| The GPUI app runs with `QuitMode::Explicit`. | Lead | GPUI quits when the last window closes on Linux by default (`gpui-pre` 0.3.9 `src/app.rs:3180-3187`), which would drop the clipboard selection. |

## Input box

| Decision | Who | Evidence |
|---|---|---|
| Two steps. An explicit scope (Tab, or a registered prefix) routes the query to one feature. Otherwise every root feature evaluates the query and a merge step orders the rows by a fixed section table. | User, after discussion | Fan-out costs under 0.1 ms per keystroke. The calculator takes 1.1 µs ([notes/08](research/notes/08-calc-engine-workflow.md)) and frizbee 34 to 73 µs on 10k items ([notes/03](research/notes/03-wayland-niri-integration.md)). zlaunch and look work this way ([notes/09](research/notes/09-input-box-prior-art.md)). |
| No keyword sniffing to pick a feature. Each feature's own parser decides whether the query is its input. | User, after discussion | "Contains x" fires on `firefox`. The calculator parser keeps `xterm`, `7zip`, `0x`, and `12 x` silent. |
| Empty query shows **Suggestions** (top 5 apps and commands by frecency; on a fresh install, Clipboard History then the first apps alphabetically), then **Commands** (every built-in command, alphabetical). Each row shows its kind (Command or Application) on the right, as Raycast does. | User | Raycast root screenshot (2026-10-14). |
| Built-in commands are searchable root rows like apps. MVP commands: Clipboard History; Open Config, Reload Apps, Quit lyncast; Lock Screen (`loginctl lock-session`), Suspend, Log Out (`niri msg action quit`), Reboot, Power Off. Log Out, Reboot, and Power Off ask for confirmation. | User (set), Lead (mechanisms) | |
| Section order for a typed query: Answer (calculator card), then Results (apps and commands ranked together). A Fallback section, such as web search, can come later. The MVP has no fallback rows. | Lead | TinyCast puts the card at index 0 ([notes/06](research/notes/06-tinycast-behaviour-spec.md)). |
| The MVP registers no text prefixes. Tab is the only scope, and it opens the clipboard screen, as the Clipboard History command does. | Lead | |

## Row shapes

These entries were made on 2026-10-15.

| Decision | Who | Evidence |
|---|---|---|
| Keys are a typed enum, parsed at the CLI and the socket. Parsing splits at the first colon. `clip:` takes ASCII digits only. Command ids are kebab-case, such as `cmd:power-off` and `cmd:quit`. | Lead | A string key makes every resolver split the prefix again. The notes/07 judge marked raw-string ids down ([notes/07](research/notes/07-paste-seam-workflow.md)). |
| The chord table reads a `Subject`, which is the row's kind plus the facts its menu needs, such as an app's Desktop Actions and a clip's pin state. `Row` stores the subject, and its key and kind come from it. The daemon rebuilds the subject from current state at every activation. It runs a menu pick only if the rebuilt list has it. | Lead | A table keyed only on kind cannot list Desktop Actions or choose Pin or Unpin ([notes/08](research/notes/08-calc-engine-workflow.md) review). The notes/07 judge's dispatch never checked an app action against its entry. |
| Each action item is bound to its row's ids, such as `Pin(ClipId)` and `Run(Command)`, so the daemon's dispatch is an exhaustive match. A menu pick sends only the verb. | Lead | A dispatch on key and verb pairs needs an unreachable arm for a pair such as an app and Pin. |
| Ctrl+Enter on app and command rows does what Enter does. | User | One rule covers every row. Ctrl+Enter runs the row's own Ctrl+Enter item if it has one, else the default, so it is never a dead key. |
| Confirmation is a fact on `Command`. The daemon checks it after the verb resolves and before the runner. | Lead | TinyCast keeps confirmation in the coordinator, not the runner ([notes/06](research/notes/06-tinycast-behaviour-spec.md)). |
| One `Section` enum orders both lists. Its order is Answer, Results, Suggestions, Commands. The empty and typed lists never share a section. | Lead | TinyCast describes the two lists separately ([notes/06](research/notes/06-tinycast-behaviour-spec.md)). |
| The calculator card and clip rows show no kind label. Only app and command rows show one, as the input box entries say. | Lead | The card has its own layout, and clip rows live on the clipboard screen. |
| A clip id is an `i64`, the sqlite rowid type. | Lead | |

## Calculator engine

Full design: [design/calculator.md](design/calculator.md). Evidence: [notes/08-calc-engine-workflow.md](research/notes/08-calc-engine-workflow.md).

| Decision | Who | Evidence |
|---|---|---|
| `calc::evaluate(text, &Env) -> Outcome`, where `Outcome` is `Card(Answer)`, `Error`, or `Silent`. The MVP never returns `Error`. | Measured | Base prototype "quantity" passed 61 of 61 tests and took units and currency with +24/-5 lines in old files. |
| Values are quantities with a dimension. The dimension is always empty in the MVP. | Measured | Same bake-off. |
| One operator table owns spelling, binding power, echo glyph, badge name, and the arithmetic. | Measured | Same bake-off. |
| Parsing never reads `Env`. Atoms are priced during evaluation. | Lead | Keeps "No exchange rate" and "no snapshot" error cards possible later. |
| Numbers are exact `i128` rationals. Overflow is silent. No big-number crates. | Lead | TinyCast rejects overflow. Zero dependencies. |
| The engine returns only its `Outcome`. The daemon builds the row: key `calc:<raw query>`, index 0, section, and actions from the chord table. | Lead | The engine knows no verbs, which keeps the paste seam intact. |
| `0x`, `0b`, and `0o` followed by letters or digits form one radix token, and the MVP stays silent on it. | Lead | `0x10` gave a `0 × 10 = 0` card in all three prototypes. |
| Date-shaped input (`dddd-dd-dd`, `dd/dd/dddd`) stays silent until dates are supported. | Lead | `2026-10-14` gave `Difference 2,002`. |
| Full-width characters (U+FF01 to U+FF5E, U+3000) are folded to ASCII first. | Lead | TinyCast canonicalizes first. |
| The input cap is 1024 bytes, with a parse depth cap of 32. | Lead | 256 bytes silenced ordinary pasted sums. |
| Exact decimals keep every digit. | Lead | `123456789012.34+0` lost its cents. |
| Any `NxN` shape is math. `1920x1080` shows 2,073,600, and an app or file with that name stays one row below the card. look's corpus treats it as a search, and lyncast does not follow look here. | User | "Why would anyone type 1920x1080 not mean to calculate." |
| Queries that start with a hyphen are accepted by the CLI (`allow_hyphen_values`). A usage error exits 2. A stale rate snapshot will exit 3. | Lead | clap rejected `lyncast query -5+3`. |

These entries were made on 2026-10-15.

| Decision | Who | Evidence |
|---|---|---|
| The words badge has at most 40 characters. Longer words are dropped, so `1920x1080` shows no words. | User | A 64-character cap keeps words on every number below a million. At 40, 8% of 4- to 6-digit numbers keep them (measured on a seeded sample). |
| The engine test reads the `fixtures` blocks in [design/calculator.md](design/calculator.md), so that page is the only copy of the fixtures. | Lead | The base's rule fixtures lived only in a scratch file under `/tmp`. |
| The echo writes literals as the engine reads them. `.5` echoes `0.5`, and `007` echoes `7`. | Lead | The echo already rewrites `x` as `×` and groups digits. The notes/08 judge proposed it. |
| A date shape anywhere in the query is silent, not only a whole-query date. A minus sign counts as a hyphen. | Lead | The date stage will read the whole query, so `2026-10-14+1` would change its answer. `2026−10−14` showed `Difference 2,002` until the minus sign counted. |
| An exact value never drops an integer digit, so it prints in plain notation however large it is. A terminating decimal also keeps every fraction digit, and only a repeating decimal below 10^-7 prints in scientific notation. | Lead | `123456789012345678901234+1` shows all 24 digits instead of `1.23456789012e23`. |
| The MVP has no square root, so every value is an exact fraction and nothing is approximate. `√`, `sqrt`, and `square root of` spell nothing, so a query with them is silent. | User | With `f64` roots, `√2×√2−2` showed `4.4408920985e-16` and `√2+10000000000000000−10000000000000000` showed `2` (measured). People rarely type roots. |
| The MVP has no `Rates` type. `catalog::lookup(word)` takes no `Env` and finds nothing yet. | Lead | The base's `Rates` held `f64` rates, which the notes/08 review rejected. The parser's two lookup calls keep the measured extension cost. |

## Clipboard and paste

Full design: [design/clipboard-and-paste.md](design/clipboard-and-paste.md). Evidence: [notes/07-paste-seam-workflow.md](research/notes/07-paste-seam-workflow.md).

| Decision | Who | Evidence |
|---|---|---|
| The palette reports chords (Enter, Ctrl+Enter), never verbs. One pure table maps a row's subject and a chord to an action. | Lead | Paste adds a verb to the table, not a palette change. |
| Ctrl+K opens a minimal, searchable Actions menu for the selected row. The default action is listed first with its key. Apps: Open, the entry's Desktop Actions, then Copy Path to .desktop. Clipboard: Copy, Pin or Unpin, Delete. Calculator: Copy Answer. Commands: Run. The menu lists the verbs the chord table allows for that row. | User (scope), Lead (contents) | Raycast and TinyCast screenshots. |
| Palette requests to the daemon are async from the start. | Lead | A paste result arrives after the window closes. |
| The palette closes when the activating keys are released. | Lead | A held Shift or Alt would merge with an injected Ctrl+V. |
| After a successful action, the daemon schedules the close with `cx.defer`. `on_window_closed` clears the summon. Losing keyboard focus cancels the summon. | Lead | A nested close fails in GPUI (`src/app.rs:2215`). TinyCast hides on focus loss. |
| `Clips` is the only owner of clip rows. Core ranks a borrowed slice. | Lead | A second copy went out of sync in review. |
| A separate capture thread owns the `wl-clipboard-rs` `Watcher` and sends entries over a channel. | Lead | `Watcher::next_event` blocks with no timeout. |
| lyncast's own writes carry `application/x-lyncast-own`, holding the same bytes as the payload. The watcher skips them and the password-manager hint `x-kde-passwordManagerHint`. | Measured | An empty marker made first-type readers get 0 bytes in 5 of 12 runs. |
| Capture every type: text, images, and files (`text/uri-list` and `x-special/gnome-copied-files`, pointing at the original paths). Links and emails are derived from text when read, as TinyCast does. | User | TinyCast clipboard screen (screenshot, 2026-10-14). |
| The clipboard screen matches TinyCast: a filter input, a type filter (All Types, Text Only, Images Only, Files Only, Links Only, Emails Only), rows grouped by day, a preview pane, and an information panel (Source, Type, Path, Size, Copied). | User | Same screenshot. |
| Preview pane in the MVP: text shows its content, images show a capped-size preview, files show their icon and the information panel (plus an image preview when the file is an image). No PDF or document rendering. Reading the freedesktop thumbnail cache (`~/.cache/thumbnails`) is the first follow-up after the MVP. | User | |
| Unpinned entries expire after 3 months by default (configurable). Pinned entries never expire. Entries over 25 MB are not recorded. | User (retention), Lead (size cap) | |
| Source is the app that was focused (niri IPC) when the selection changed. data-control does not report the real source, so this is a best guess. | Lead | ext-data-control and wlr-data-control carry no source client. |
| Paste backends live later in the daemon's `paste/` module. The first one is the niri virtual keyboard. | Lead | [notes/07-paste-seam-workflow.md](research/notes/07-paste-seam-workflow.md) |

## Stack

| Decision | Who | Evidence |
|---|---|---|
| GPUI from `gpui-pre = "=0.3.9"` with `gpui-pre-platform`, pinned exactly. | Measured | Built and rendered a layer-shell window under anvil. [notes/02-gpui-on-linux.md](research/notes/02-gpui-on-linux.md) |
| Fuzzy matching with `frizbee`, with TinyCast's ranking rules on top. | Measured | 100k items in 0.4 to 1.2 ms vs 2.9 to 5.2 ms for `nucleo-matcher`. [notes/03-wayland-niri-integration.md](research/notes/03-wayland-niri-integration.md) |
| A hand-written calculator, no `fend-core` or `numbat`. | Measured | Neither matched TinyCast's fixtures. |
| Clipboard watching with `wl-clipboard-rs` 0.9.4 `Watcher`. Storage with `rusqlite`, blobs as files named by hash. | Lead | |
| Desktop entries with `freedesktop-desktop-entry`, plus our own `Exec` tokenizer. | Measured | Its parser split `sh -c "echo a b"` on every space. |
| App launching through niri IPC `Action Spawn`, using the `niri-ipc` crate. | Lead | Gives the activation token and the systemd scope. |

## Testing

| Decision | Who | Evidence |
|---|---|---|
| Tests are CLI integration tests that assert literal output. Palette behaviour that the CLI cannot reach is tested with GPUI's `TestAppContext`. | User (CLI), Lead (palette) | The dev box is headless. |
| Tier 1 needs no display. Tier 2 runs under Xvfb plus smithay `anvil`. Tier 3 uses GPUI's `TestAppContext`. | Measured | anvil offers `wl_seat`, layer shell, wlr data-control, and xdg-activation. Headless weston has no `wl_seat`, and GPUI panics without one. |
| Tier 1 tests clear `WAYLAND_DISPLAY` so they cannot touch the developer's clipboard. | Lead | |
| `scripts/test-env.sh` is the one way to start the Tier 2 compositor. `start` prints shell lines that point at a new environment, once a client roundtrip sees `wl_seat`, layer shell, and wlr data-control. `stop DIR` ends it, and `run -- CMD` wraps one command. [design/testing.md](design/testing.md) lists the lines. From step 6, each Tier 2 Rust test calls `start` and `stop` itself. | Lead | Each test gets its own anvil ([design/testing.md](design/testing.md)). GPUI panicked on a compositor without `wl_seat` ([notes/04](research/notes/04-probe-gpui-0.2.2-headless.md)). |
| anvil is pinned to smithay rev `8a270232f1bf` and to `scripts/anvil.Cargo.lock`, the lockfile of the tested build. `scripts/anvil.sh` builds it with `--locked` and caches the binary in `${XDG_CACHE_HOME:-~/.cache}/lyncast/`, keyed by a hash of the script and the lockfile. | Lead | smithay does not commit its `Cargo.lock`, so a rev alone leaves 448 registry and git crates free to move. A fresh build took 32 s. |
| The purity test runs `cargo tree -p lyncast-core --all-features --target all` and follows every edge kind. A fixture proves that it finds a banned crate through a transitive, an optional, a target-specific, and a dev dependency. | Measured | With its default flags, `cargo tree` missed the optional and the `cfg(windows)` dependency in the fixture. |
| The bench is `bench/summon.py`, the notes/05 harness without its spawn-per-summon mode. It measures `bench/probe` until the palette exists. | Measured | The port reproduced notes/05: a 44.7 ms warm summon (notes/05: 44.9), 11.6 MiB PSS before the first summon (11.9), and 57.2 MiB `Pss_Anon` after 100 cycles (57.3 after 70). |
| The CI gate is `scripts/check.sh`: fmt, clippy, `cargo test`, shellcheck, and a Tier 2 smoke test. The bench joins it in step 9, when it measures lyncast. | Lead | The probe does not change with lyncast commits, so a bench of the probe cannot catch a lyncast regression. |

## Budgets

Measured on lavapipe in CI. They catch regressions. Real numbers come from Hongyi's niri machine, which has an AMD GPU (Mesa radv).

| Budget | Limit | Measured on the dev box |
|---|---|---|
| Warm summon to first frame | 50 ms or less | 45 ms |
| PSS before the first summon | 15 MiB or less | 12 MiB |
| Private anonymous memory (`Pss_Anon`) after summons | 60 MiB or less | 57.3 MiB |
| Growth trend across 100 summon cycles | none | between −0.6 and +7 kB per cycle over cycles 21 to 70 |
| Idle CPU | under 0.1% | under 0.1% |

## Lead defaults Hongyi can override

| Default | Reason |
|---|---|
| The calculator card outranks an app with a matching name (`8x8` shows 64 first, the app second). | TinyCast parity. With fan-out the app stays one row down. |
| The words badge appears for whole numbers only. | Short badges. |
| Division by zero shows no card. | `10/0` is a prefix of `10/0.5`. |
| A copied calculator answer goes into clipboard history. | It is on the clipboard. |
| On clip and calculator rows, Ctrl+Enter copies in the MVP, the same as Enter. | It is the slot paste will use. |
| The MVP ships a dark theme only. Following the system light or dark setting comes later. | The reference screenshots are dark, and one theme halves the visual checks. |
| App icons are shown, resolved lazily for visible rows on a background thread, with a bounded cache. | Raycast and TinyCast show them. zlaunch's uncapped caches cost hundreds of MB. |
| The palette opens on the focused output (niri IPC), centered horizontally, about one third down. Blur comes from a niri `layer-rule`, not from lyncast. | GPUI's blur speaks only the KDE protocol. |
| Config lives in `$XDG_CONFIG_HOME/lyncast/config.toml`. Data lives in `$XDG_DATA_HOME/lyncast/`. | XDG convention. |
| The daemon starts from a systemd user unit, `lyncast.service`. The docs also show niri `spawn-at-startup` as an alternative. The niri bind runs `lyncast toggle`. | Restarts on crash, so the clipboard owner comes back. |
| A dot under the icon of a running app (from niri IPC) comes after the MVP. | Raycast shows it. Not needed to launch. |

## Open questions

Asked one at a time. Answers move into the tables above.

None right now.
