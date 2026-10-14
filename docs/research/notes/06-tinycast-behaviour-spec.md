# TinyCast behaviour spec extract (research agent, 2026-10-14)

Raw agent report, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.

I fetched all 8 files into `/tmp/tinycast-docs/` and read them in full. Citation keys: **P** palette.md, **N** navigation.md, **L** launcher.md, **C** clipboard.md, **K** calculator.md, **A** architecture.md, **S** standards.md, **T** testing.md. One mismatch: navigation.md is about Switch Windows and Menu Search, not palette navigation. Palette navigation is in P §Navigation.

## 1. Palette and navigation

- **Summon:** ⌥Space toggles the palette. Once per summon it records the previous app (the paste target), the screen anchor and the input source (P §Summoning).
- **Dismiss:** losing key focus hides it (A §Entry points). Clicking away closes it (T §Core).
- **Escape:** clears a non-empty query first. The next press pops a pushed screen. On the root screen it hides (P §Navigation).
- **Backspace / ⌘⎋:** a bare Backspace in an empty field takes the same step but never closes. ⌘⎋ goes to root search from any depth (P §Navigation).
- **Pop to Root:** hiding schedules a reset after `popToRootTimeout`. A re-summon inside that window keeps state (P §Screens).
- **Push vs root:** a command run while the palette is visible pushes a screen. A hotkey into a hidden palette opens a root. The launcher is always root (P §Navigation).
- **Tab:** cycles launcher → AI → clipboard. With AI off it flips launcher ↔ clipboard. The query carries into clipboard. Each hop is one Escape step, and the stack never exceeds two (P §Navigation).
- **Selection:** one flat index equals visible row order. The calculator card is index 0. Headers take no index. Every screen lands on row 0, except clipboard, which lands on the newest clip below the pins (P §Invariants, §Screens).
- **Input handling:**
  - The palette handles ↵; the field never submits (P §↵ never commits).
  - Line breaks in a pasted query become spaces (P §The query is one line).
  - ⌃N/⌃P act as ↓/↑ (P §Emacs).
  - ⌘1…⌘0 are bound to physical keys (L §⌘-digit slots).
  - The placeholder hides during IME composition (P §IME).
- **Compact bar:** shown only while the query is empty. The top edge never moves (A §Entry points).
- **Before typing:** Favorites, Meetings, then up to 5 Suggestions, then one section per kind sorted by frecency, alias, kind priority, name (L §The empty list).
- **Merge order:** calculator card (or meeting card, never both) at index 0 (P §Inline row arguments). Then the colour card (C §Colours), then "Open in Browser" for URLs (L §Contextual commands), ranked matches, category sections, and fallbacks last (L §Fallbacks). Clipboard items are not merged into root search (inferred from the P §Screens table).
- **Latency:** there is no millisecond target. The rule is "The palette must feel instant", and summon-path work is resolved once per show (S §Performance). Nothing is built per keystroke (L §Invariants).

## 2. Launcher

- **Index:** an off-main scan of user scopes, one subfolder deep, skipping symlink cycles. Duplicates are resolved by bundle ID: the earliest scope wins, then the newest version within a folder (L §Search scopes).
- **Fields:** title, alternate titles, subtitle, keywords. Keywords only make an entry appear; they never rank. Bundle IDs and executable names are not matched (L §Search fields).
- **Scorer points:** first character on first character 4, word start 3, other 2, mismatched separator 1, gap −1. An exact match beats all. Word starts include camelCase and a letter after a digit (L §The scorer).
- **Sensitivity** (L = query length): Low is any alignment. Medium, the default, is `score ≥ 1.5·(L−2)+4`. High is `score > 2·L` (L §Sensitivity).
- **Comparator** (first rule that separates two entries wins), in order:
  1. exact alias
  2. boosted term
  3. exact title, past 3 characters
  4. exact past search term
  5. exact subtitle
  6. alias prefix
  7. past-term prefix
  8. past term run at most 3 characters past
  9. best field score
  10. frecency
  11. the title's own score
  12. title prefix
  13. kind priority (apps 4 > commands 3 > quicklinks 2 > panes 1)
  14. name, compared numerically

  (L §The comparator, §Kind priority)
- **Fold:** NFC, then case-, diacritic- and width-insensitive, applied once per field and once per query (L §One fold). Non-Latin names match through romanization (L §Non-Latin names).
- **Frecency:** `score(t) = max(1, e^(λ·(anchor − t)))`, with λ = ln 2 per 10 days. A visit adds 100. The last 3 distinct search terms are kept. They steer ranking only while score > 1 and the entry was opened in the last 17 days. Global shortcuts, fallbacks and contextual rows do not count as visits. Data is stored in `launcher-ranking.json` with per-item and global reset (L §Learned ranking).
- **Aliases:** an exact alias is rule 1 and a prefix is rule 6. Anywhere else it ranks as an alternate title (L §User aliases).
- **Favorites:** an ordered array, shown first only when the query is empty (L §Favorites).
- **Refresh:** rescan on every open, with a per-bundle name cache keyed on mtime. Overlapping refreshes collapse into one trailing scan (L §Search scopes, §Alternate names).
- **Edge cases:**
  - No "background agent" filter: every heuristic tried also hid real apps (L §Search scopes).
  - The label is the localized name, else the file name. The declared name becomes a keyword (L §Alternate names).

## 3. Clipboard history

- **Types:** only `text`, `image` and `file` are stored. Link, email and colour are derived on read (C §Invariants).
- **Capture:**
  - Polls every 0.5 s.
  - Reads a file URL before text.
  - Rejects files under temp roots.
  - Caps files per batch with `maxCapturedFiles` (value not stated; durability checks stop at 32).

  (C §Poll-based capture)
- **Own writes:** every write the app makes carries a private marker type, and the poller skips it (C §Invariants).
- **Privacy:** the "sensitive-type guards" always apply, but the types are not listed (C §Poll-based capture). Password-manager copies and copies from "Disabled Applications" are not recorded (T §Clipboard). That list ships with seeded entries, which are not named (N §Wiring).
- **Limits:**
  - The newest 1000 unpinned rows stay in memory (C §Store).
  - The table is capped by age only, and retention includes "Forever" (C §Store).
  - Search returns at most 200 rows (C §Type filter).
  - Retention presets, a size cap and dedupe at capture time are **not specified**.
  - Pasting moves an existing row to the top (C §Store).
- **Pins:** ordered by when they were pinned. Pins survive retention and Clear History and are not moved to the top on paste (C §Pinned entries).
- **Search:** SQLite FTS5 trigram for 3 or more characters. Shorter queries filter the in-memory window (C §Store).
- **Storage:**
  - `clipboard.sqlite3` plus loose PNG files.
  - Kept in Application Support, not Caches.
  - A database that won't open is recreated.
  - File entries point at the original path and are never copied.

  (C §Store, §Referenced files)
- **Paste-back:** ↵ pastes, ⌘↵ copies, ⌃⌘↵ pastes plain text, ⌥↵ pastes and keeps the window open (C §Invariants). Paste activates the previous app and posts a synthetic ⌘V, which needs Accessibility (P §Focus restoration).

## 4. Calculator

- **Syntax:**
  - Arithmetic, implicit multiplication, `mod`, functions, bitwise, comparisons.
  - Number bases.
  - Typed units.
  - Currency and crypto.
  - Dates (grammars A to G) and time zones.
  - Percent and list phrases.
  - Unix and RFC 3339 timestamps.

  (K §Evaluation pipeline onward)
- **Inline card:** sits at index 0. ↵ copies the answer and records it to history. ⌘↵ makes the answer the new query for chaining; date, time and boolean answers don't offer this (K §Result and rendering). A trailing operator keeps the last valid result visible (K §Modulo).
- **Format:**
  - Grouped thousands.
  - Money has 2 decimals, widening to 4 significant digits below a cent, never scientific notation (K §Exchange rates).
  - The engine works in canonical English. Locale separators apply only at display time (K §Number format).
- **Errors:** shown only for definite mistakes, such as `Cannot convert Currency to Weight.` or `No exchange rate for <CODE>.`. Everything else stays silent. A single ASCII word never shows a card (K §Evaluation pipeline, §Currency).
- **Rates:** fetched outside the engine and refreshed every 24 h. Offline, the last snapshot keeps answering (K §Exchange rates).

**Test fixtures (verbatim from K):**

| Input | Output |
|---|---|
| `5feet + 1m` | `2.524 m` |
| `10kg + 500g` | `10,500 g` |
| `10kg + 20%` | `12 kg` |
| `5m * 4m` | `20 m²` |
| `100km / 2h to km/h` | `50 km/h` |
| `1000 / (3/s)` | `333.3333333 s` |
| `4(2+3)` | 20 |
| `-10 mod 3` | -1 |
| `20% off 500` | 400 |
| `15% tip on 42` | 6.3 |
| `ratio of 1920 to 1080` | `16 : 9` |
| `1e5` | `100,000` |
| `145 mins to timespan` | `2 hr 25 min` |
| `3 months to days` | `91.310625 day` |
| `1km == 1000m` | `true` |
| `10 +` | `10` |

**Must stay silent:** `1hr 30`, `20 eur to usd * 30`, `5 3`, `10 of`, `1.2.3 + 1`, `1e400`.

## 5. Budgets and standards

- **Memory:** "Resident memory under 100 MB, always". Memory returns to baseline after the palette closes (S §Performance). Normal use is 40 to 80 MB (T §Recorded baselines). There are no separate idle or peak numbers.
- **Leaks:** "Zero leaks and no retain cycles." Ownership is a tree rooted at the core. Every long-lived task has an owner and a teardown (S §Performance, §Concurrency).
- **Other budgets:** no frame budget. Launch is protected most (S §Performance). Release binary growth stays under 2% (T §Build).
- **Measurement:** 7 signpost intervals viewed in Instruments, plus standalone `-O` benchmarks that output JSON. Cold launch is the median of 3 runs (T §Performance measurement). Launch time and RSS have never been recorded as numbers (T §Recorded baselines).
- **Testing:**
  - No XCTest and no UI tests. Standalone harnesses compile the shipped pure sources, so a compile failure means a layering leak (T §The harnesses).
  - Harnesses never touch shared system state, including the real pasteboard (T §The harnesses).
  - A ranking complaint becomes a new fuzz case (T §What to run when).
  - Definition of done: harnesses, lint, the purity grep, a clean build with no new warnings, and docs fixed in the same commit, plus a manual sweep (T §Definition of done).

## 6. Architecture worth borrowing

- **Four layers:** PURE (no UI, clock, network or filesystem; all injected), then EFFECT, then OBSERVABLE STATE, then VIEW. Purity is checked mechanically (A §The layering).
- **One owner:** a single core object with one boot wiring point (A §Single-owner core).
- **Coordinators:** views never mutate stores. Confirmation steps live in the coordinator, not the runner (A §Single-owner core).
- **Screens:** each mode is one implementation of a screen interface. Its `rows` are the single source of visible order (P §Screens).
- **Concurrency:** prefer synchronous code and move blocking IO off the main thread (S §Posture).
- **Helper processes:** heavy libraries run in a helper process that exits, so their memory leaves with it (A §Single-owner core).
- **Generated data:** data tables come from scripts and are never hand-edited (K §Invariants).
- **Naming:** a type's suffix says what it is (S §Naming).

## 7. Port to Linux/niri

| Item | Verdict | How |
|---|---|---|
| ⌥Space hotkey | Adapt | A niri keybind runs `lyncast toggle` over a socket (inferred) |
| Panel and anchor | Adapt | Layer-shell overlay with exclusive keyboard (inferred; GPUI support needs checking). Drop drag, snapping and per-display positions |
| Escape/Backspace walk, flat selection, landing, ↵ handling, one-line query, ⌃N/⌃P, IME | Keep | |
| Tab ring | Adapt | Launcher ↔ clipboard only |
| ⌘ chords | Adapt | Map to Ctrl (inferred) |
| Input-source switching, cursor policy, ⌘⎋ tap, window switcher | Drop | macOS-only or out of scope |
| App index | Adapt | `.desktop` files from XDG dirs. Dedupe by desktop-file ID, earliest XDG directory wins. Respect `NoDisplay`, `Hidden`, `OnlyShowIn`. Index `Name[locale]`, `GenericName`, `Keywords`. Watch with inotify (inferred) |
| Scorer, comparator, fold, frecency, aliases, favorites | Keep | Rewrite from the spec, not the code |
| Romanization, running dot, quit/restart, drag-out, fallbacks | Drop for MVP | |
| Capture | Adapt | Event-driven `ext-data-control-v1` or `wlr-data-control` (inferred; check niri). Private MIME marker for own writes |
| Sensitive guard | Adapt | Skip `x-kde-passwordManagerHint` (inferred) |
| Disabled apps | Defer | data-control doesn't expose the source app. A heuristic could use niri IPC focused app-id (inferred) |
| Storage, pins, limits | Keep | SQLite FTS5 under `$XDG_DATA_HOME/lyncast/` |
| Paste-back | Adapt | MVP is copy-only. Optional later: Ctrl+V through `virtual-keyboard-unstable-v1`; terminals need Ctrl+Shift+V (inferred) |
| OCR, colours, Paste Sequentially, previews | Drop | |
| Calculator engine | Keep | Pure crate. Inject clock, time zone, rates, locale |
| Time zones, region currency, number format | Adapt | chrono-tz, `LC_MONETARY`, `LC_NUMERIC` (inferred) |
| Currency fetch | Keep or defer | Cache in `$XDG_CACHE_HOME` |
| Memory and leak rules | Keep | Measure with `/proc/self/status` and heaptrack (inferred) |
| Test harnesses | Adapt | Core crate with no gpui or Wayland dependencies; check with `cargo tree`. A fake clipboard source in tests (inferred) |
| Signposts | Adapt | `tracing` spans (inferred) |

**Gaps to settle in the spec:** the docs give no clipboard retention presets, per-item size cap, capture-time dedupe rule, list of sensitive types, or seeded list of disabled apps.
