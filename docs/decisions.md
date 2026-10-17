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
| Two steps. An explicit scope (Tab, or a registered prefix) routes the query to one feature. Otherwise every root feature evaluates the query and a merge step orders the rows by a fixed section table. | User, after discussion | Fan-out fits the 0.1 ms budget per keystroke on a 300-app desktop. The calculator takes 0.45 µs on its 26 shared inputs ([design/calculator.md](design/calculator.md#the-ports-cost-per-keystroke)), and on 2026-10-16 ranking's worst bench query took 71 µs on 300 apps ([design/ranking.md](design/ranking.md#the-worst-bench-keystroke-takes-71-µs-on-300-apps-and-254-µs-on-1000)). The budget does not hold everywhere. On 1,000 apps the worst query of every typed group took 139 to 254 µs. When some field is as long as a pasted query, a paste of 109 characters took 228 µs in the worst of six runs, and one of 896 takes about 2.1 ms. The calculator's worst capped input takes 602 µs. zlaunch and look work this way ([notes/09](research/notes/09-input-box-prior-art.md)). |
| No keyword sniffing to pick a feature. Each feature's own parser decides whether the query is its input. | User, after discussion | "Contains x" fires on `firefox`. The calculator parser keeps `xterm`, `7zip`, `0x`, and `12 x` silent. |
| Empty query shows **Suggestions** (top 5 apps and commands by frecency; on a fresh install, Clipboard History then the first apps alphabetically), then **Commands** (every built-in command that Suggestions does not hold, alphabetical). Each row shows its kind (Command or Application) on the right, as Raycast does. | User | Raycast root screenshot (2026-10-14). On 2026-10-15 Hongyi chose that a suggested command leaves Commands, as TinyCast does, so no key appears twice. |
| Built-in commands are searchable root rows like apps. MVP commands: Clipboard History; Open Config, Reload Apps, Quit lyncast; Lock Screen (`loginctl lock-session`), Suspend, Log Out (`niri msg action quit --skip-confirmation`), Reboot, Power Off. Log Out, Reboot, and Power Off ask for confirmation. | User (set), Lead (mechanisms) | |
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
| The CLI accepts a query that starts with a hyphen (`allow_hyphen_values`), unless the query is an option's name. `--json` or `--help`, alone or with `=` and a value, and `--` itself need `lyncast query --json -- <text>`. A usage error exits 2. A stale rate snapshot will exit 3. | Lead | clap rejected `lyncast query -5+3`. |

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

## App index and ranking

Full design: [design/ranking.md](design/ranking.md). Evidence: [notes/10-frizbee-tinycast-scoring.md](research/notes/10-frizbee-tinycast-scoring.md). These entries were made on 2026-10-15.

| Decision | Who | Evidence |
|---|---|---|
| frizbee scores with TinyCast's points as its weights: `match_score` 2, `prefix_bonus` 2, `delimiter_bonus` and `capitalization_bonus` 1, gaps 1 to open and 0 to extend, `mismatch_penalty` 4, no case or exact bonus, no typos. TinyCast's Medium threshold then applies as written, `score ≥ 1.5·(L−2)+4`. | Measured | Against a reference scorer written from TinyCast's doc, 98.4% of 17,596 aligned pairs scored the same and the rest 1 apart, with no order inverted. frizbee's own weights with the best linear threshold agreed on 75% of shown pairs and inverted 2,239 orders. |
| The query loses its separators (space, tab, newline, `- . / ( ) [ ]`) before frizbee, and L counts what is left. Exactness is decided on folded text, not by frizbee. | Measured | 305 of 305 multiword pairs agreed with the reference, against 301 when the separators stayed. frizbee's `exact` is case-sensitive. |
| One fold for fields and queries: full-width forms narrowed (the calculator's step, now shared), format characters dropped, diacritics dropped through NFD and the canonical combining class, then lowercase. `unicode-normalization` joins `lyncast-core` for the tables. | Lead | TinyCast's fold. A hand table would cover Latin-1 at best. Final sigma folds to sigma, as Unicode case folding has it. Half-width katakana, the full-width signs U+FFE0 to U+FFE6, ligatures, and `ß` are not folded. |
| An app's fields: title `Name[locale]`, alternate title the unlocalized `Name`, subtitle `GenericName[locale]`, and keywords `Keywords[locale]` plus `Keywords`, plus the title and subtitle joined both ways. Keywords make an entry show and never rank it. Exec, Icon, and the desktop-file ID are not matched. | Lead | TinyCast's four fields. Its keywords include the title and subtitle joined both ways, so `firefox web` finds Firefox. The joins cost about 31% more haystacks on the bench ([design/ranking.md](design/ranking.md#the-worst-bench-keystroke-takes-71-µs-on-300-apps-and-254-µs-on-1000)). |
| The comparator keeps TinyCast's rules 3, 4, 5, 7 to 14, with the rule 3, 4, and 5 sub-orders and the tiebreak (frecency, kind priority, name). Rules 1, 2, and 6 (aliases and boosted terms) are dropped until aliases arrive. | Lead | Aliases come after the MVP (Scope). lyncast has no boosted command. |
| Each built-in command has a few keywords, such as shutdown for Power Off and restart for Reboot. [design/ranking.md](design/ranking.md) lists them. | Lead | |
| Frecency follows TinyCast: `max(1, e^(λ·(anchor − t)))` with λ = ln 2 per 10 days, +100 per visit, and the last 3 distinct search terms, which steer only while the score is above 1 and the entry was opened in the last 17 days. Anchors are whole milliseconds. The clock is always an argument. | Lead | TinyCast stores the anchor so the order holds as time passes. |
| Nothing persists in step 4. Step 5's daemon owns the visits and writes `$XDG_DATA_HOME/lyncast/ranking.json` with `serde_json`, through a temporary file and a rename. | Lead | No visit is recorded before `activate`. TinyCast uses a JSON file, and `rusqlite` is for clips. |
| "Alphabetical" means the lowercased fold with digit runs compared by value, then the raw title, then the key. There is no locale collation. | Lead | TinyCast compares names numerically. |
| Desktop-file IDs keep `.desktop` and replace `/` with `-`, so keys read `app:firefox.desktop`. The first file for an ID decides before it is read, so a file that cannot be read or does not parse masks the same ID in later directories, as do its `Hidden`, `NoDisplay`, `OnlyShowIn`, `NotShowIn`, `TryExec`, and Exec program. The binary reads only a regular file of at most 1 MiB, after symlinks. The walk follows symlinked directories and does not follow a link back to a directory it is inside. | Measured | The Desktop Entry spec uses "the first one in the $XDG_DATA_DIRS precedence order". GLib 2.82.2 masks by file name before it parses. A user file with a bad line, no `[Desktop Entry]` group, or `Hidden=true`, an unreadable file, a broken symlink, and a symlink to `/dev/zero` each hid the system entry (dev box, 2026-10-16). Before the 1 MiB cap, that link made lyncast read until allocation failed, 2 GB under a 4 GB address-space limit. GLib opens a file before it checks that it is regular, so it skips `/dev/zero` but blocks on a FIFO until a writer opens it, and it reads a file over 1 MiB. `Hidden=true` in `~/.local/share/applications` is how a user deletes a system entry. GLib has no cycle check and follows `loop -> .` until the kernel's limit of 40 links. |
| An entry shows only when its Exec program, the first argument of the tokenized Exec, is found the way `TryExec` is. An absolute path must be a file this user may execute, and a bare name one in the entry's `Path` directory, when that is absolute, or in a `PATH` directory. An unset `PATH` searches `/bin:/usr/bin:.`, and a relative `PATH` entry, such as `.` or an empty one, names a directory under `Path`, or under the working directory without one. A directory is never a program, and a relative path with a `/` is never found. | Measured | GLib 2.82.2's `g_desktop_app_info_new_from_filename` returns NULL when the Exec program is a missing bare name, a missing absolute path, or a file without the execute bit, and also when `TryExec` is found. It looks up only the first argument. `g_find_program_in_path` searched `/bin:/usr/bin:.` with `PATH` unset and only the working directory with `PATH` empty. For an entry with an absolute `Path`, the loader found a bare name there under every `PATH`, and it read `.`, empty, and relative `PATH` entries under any `Path`. It hid `TryExec=/usr` and `Exec=/usr %u`, and it found `TryExec=bin/firefox` and `Exec=bin/firefox %u` from its working directory, where lyncast hides them (dev box, 2026-10-16). |
| `OnlyShowIn` and `NotShowIn` follow GLib's walk over `XDG_CURRENT_DESKTOP`, compared case-sensitively. | Lead | GLib. |
| The locale names are `LANGUAGE` split on `:` when it is not empty, else the first non-empty of `LC_ALL`, `LC_MESSAGES`, and `LANG`. Each name gives the spec's candidates in turn, without repeats. | Measured | GLib 2.82.2's `g_get_language_names` gave `de fr C` for `LANGUAGE=de:fr` beside `LANG=en_US.UTF-8`, and `de C` for `LANGUAGE=de` beside `LC_ALL=C` or `LANG=C`, and it read an empty `LANGUAGE` as unset (dev box, 2026-10-16). POSIX orders the other three. GLib tries `lang@MODIFIER` before `lang_COUNTRY`, and lyncast keeps the spec's order. GLib also tries `Key[C]` last in every locale, `Key[POSIX]` under `POSIX`, and the names with their encoding, such as `Key[de_DE.UTF-8]`, which the spec drops. |
| `Terminal=true` apps show. Step 8 decides which terminal runs them. | User | fuzzel, rofi, and GNOME Shell list them. |
| The Exec tokenizer splits as `g_shell_parse_argv` does once the field codes expand: double quotes, single quotes, backslashes outside quotes, newlines, and `#` comments. A trailing backslash is an error, as in GLib. An unknown field code, a `%` at the end, and `%i` inside a longer argument are errors too, where GLib launches them. An entry whose Exec errors does not show. | Measured | GLib expands field codes and then splits as a shell would. Wine writes `Start\ Menu`. On 2026-10-16 every exec fixture in [design/ranking.md](design/ranking.md) without a field-code error gave the same argv or failure from `g_shell_parse_argv` in GLib 2.82.2 on the dev box, and a scratch fuzz of 667,000 random short command lines found no difference. `g_app_info_launch` still differs on inputs the spec leaves undefined. These are the known differences, measured on GLib 2.82.2: GLib expands a field code before a backslash can escape its `%`, and it shell-quotes `%c` and `%k` before it splits, so inside double quotes the quotes stay, and inside single quotes a value with whitespace splits and one with a quote fails to launch. With Name `Web Browser`, `sh -c 'exec app --name=%c' x` gives GLib `exec app --name=Web` and `Browser`, where lyncast keeps `exec app --name=Web Browser`. |
| The desktop-entry parser reads what GLib 2.82 reads, including the last value of a repeated key, and is more lenient where it differs, so that an app still shows. These are the known differences, measured on GLib 2.82.2: a leading BOM, an unknown escape or a trailing backslash, an empty localized value, a localized list that holds `\;`, a key or group name that GLib rejects, an `Encoding` other than `UTF-8`, a byte that is not UTF-8, which the binary reads as U+FFFD, a NUL, which lyncast keeps with the text after it, and a `\r` that ends the last line. | Measured | Checked against GLib 2.82.2 on the dev box (2026-10-16). GLib fails the whole file for the BOM, for a bad key or group name, and for `Encoding=Legacy-Mixed` or `Encoding=ISO-8859-1` in `[Desktop Entry]`. It fails the key that holds a bad escape, a bad byte, or a localized `\;` list, returns an empty localized value as is, and reads `Name=Nul`, a NUL, and `Tail` as `Nul`. GLib then ignores a failed key and reads a value only up to a NUL, so in `TryExec`, `OnlyShowIn`, and `NotShowIn` a bad escape, a bad byte, or a NUL can hide an app that GLib shows. |
| `lyncast-core` holds the fold, the desktop-entry parser, the Exec tokenizer, the index, frizbee matching, the comparator, the empty list, and frecency. The binary holds the XDG walk, file and environment reads, the `TryExec` and Exec program lookups, the CLI, and the JSON view. | Lead | The core stays pure (Testing). frizbee and `unicode-normalization` have no IO and are not banned. |
| Until step 5, `lyncast query <text> --json` scans and ranks in-process with no visits. `--json` is required until a plain view exists. Step 5 moves the work behind the socket, and the expected output in [design/ranking.md](design/ranking.md) becomes its regression check. | Lead | The plan's step 4 check runs `lyncast query`, and step 5 builds the daemon. |
| `lyncast query --json` prints one compact object per row: `key`, `index`, `kind`, `section`, `title`, and `actions` (each with `verb`, `label`, and `chords`). The binary owns these spellings and uses `serde_json`. | Lead | Escaping and field order come out right with no hand-written JSON. |
| `Row` gains `title`. The kind label waits for the palette, which derives it from the kind. | Lead | Nothing renders a label before step 7. |
| Every ranking fixture, including the CLI's XDG tree and its expected stdout, lives in [design/ranking.md](design/ranking.md). The tests read it and write the tree under `CARGO_TARGET_TMPDIR`. | Lead | The calculator's fixtures work this way. A tree checked in beside the doc could drift from it. |

## Daemon and CLI

Full design: [design/daemon.md](design/daemon.md). These entries were made on 2026-10-17. "Lead default, pending Hongyi" marks a preference call the lead made so work could continue.

| Decision | Who | Evidence |
|---|---|---|
| `lyncast daemon --headless` starts the daemon. `--headless` is required until plan step 7 builds the palette. | Lead default, pending Hongyi | clap parsed a top-level `--headless` beside subcommands only with `args_conflicts_with_subcommands` (scratch mock, clap 4.6.7). A subcommand needs no such rule. |
| The socket is `$XDG_RUNTIME_DIR/lyncast.sock`. The daemon takes an exclusive `flock` on `$XDG_RUNTIME_DIR/lyncast.lock` before it removes a stale socket and binds, and a second daemon exits 1. It opens the lock file without waiting on a FIFO and exits 1 when that file is not a regular file. Without `XDG_RUNTIME_DIR`, the daemon and the clients exit 1. | Lead | The XDG spec puts sockets in `XDG_RUNTIME_DIR`. A socket left by a SIGKILLed daemon makes `bind` fail with `EADDRINUSE`, and only the lock holder may remove it. `sun_path` holds 108 bytes. A FIFO at `lyncast.lock` kept the daemon from binding, with nothing on stderr, and a link to `/dev/null` there let it lock `/dev/null` and serve (reproduced on 2026-10-17). |
| The wire is one compact JSON request line and one reply line per connection, tagged as `{"request":...}` and `{"reply":...}`. Keys travel as strings and are parsed at the socket. A request line holds at most 1 MiB, and a line with no newline in its first 1 MiB fails without running. | Lead | serde is already in the binary. niri's IPC is also one JSON request per line (`niri-ipc` 26.4.0 `src/socket.rs`). A connection with no state lets the palette and CLI clients talk at once. A read capped at 1 MiB ends a longer line early, and serde_json read the cut prefix of a confirmed Reboot padded with spaces as a whole request, which then ran (reproduced on 2026-10-17). |
| The requests are `toggle`, `status`, `query`, and `activate`. `activate` carries the key, `via` (a chord or a verb), `confirmed`, and the query text, which frecency keeps as a search term. | Lead | `Visits::record` needs the query (`rank/frecency.rs`). A status request lets a test see the summon open and close (plan step 7). |
| The CLI: `toggle` and `status` print `open` or `closed`. `activate <key>` takes `--chord enter\|ctrl-enter` or `--verb <verb>`, `--yes`, and `--query <text>`, and prints one line such as `ran cmd:reboot`, `launched app:firefox.desktop`, or `copied 14`. | Lead default, pending Hongyi | Verbs and chords reuse the `query --json` spellings, so one table covers both. `copied 14` and `copied clip 1` come from notes/07. |
| A command that confirms, run without `--yes`, prints `lyncast: Reboot? Run again with --yes to confirm.` and exits 1. There is no prompt on a terminal. | Lead default, pending Hongyi | Exit 1 tells a script that nothing ran. A prompt cannot be tested without a pseudo-terminal. |
| Reload Apps scans on a worker thread. Each scan has a number in the order it started, and a scan that ends after a newer one changes nothing. | Lead | With the first scan's file read held back, a second Reload Apps showed a new app, and then the first scan ended, hid it, and brought back an app that had been deleted (reproduced on 2026-10-17). |
| Every program the daemon starts goes through one `Runner` as an argv. `lyncast daemon --record <file>` swaps it for a recorder that appends each argv as a JSON array, one whole line at a time. The real runner fails until plan step 8 builds launching through niri. | Lead | Plan step 8 owns the niri mechanism. A recorder at the runner means a Tier 1 test never starts `systemctl reboot`. Two launches at once, each of an argv of 18,000 bytes, mixed their lines in a FIFO recorder in 10 of 10 tries (reproduced on 2026-10-17). |
| The commands run `loginctl lock-session`, `systemctl suspend`, `niri msg action quit --skip-confirmation`, `systemctl reboot`, and `systemctl poweroff`. Open Config creates `$XDG_CONFIG_HOME/lyncast/config.toml` if it is missing and runs `xdg-open` on it. Reload Apps scans again. | Lead default, pending Hongyi | Without `--skip-confirmation`, niri asks again after lyncast has asked (`niri-ipc` 26.4.0 `src/lib.rs:196-200`). |
| Quit removes the socket. A request that still arrives gets `failed` with `the daemon is quitting`, or finds the connection closed as the process exits, and runs nothing either way. Quit waits for the work in flight and the last write of the visits, 5 s at most, then gives up the lock and replies. It waits up to 1 s more for its replies to be written, and exits 0. So a client that runs after the reply finds no daemon, and a new daemon can start at once. After the 5 s the daemon takes no more requests or results, so one still queued gets `failed` with `the daemon is quitting`. Once Quit has begun, the daemon fails every request and says when no work or write is left. The host owns the order and the waits, and plan step 7's palette host follows the same steps. | Lead | A Quit that returned at once left programs running whose clients were told nothing ran: of 60 Lock Screens sent with a Quit, 27 reached the recorder and 17 clients printed `ran cmd:lock-screen` (reproduced on 2026-10-17). A daemon that takes the lock next loads `ranking.json` and writes it, so a write still running keeps the lock. While the headless host decided which requests fail during Quit, a request on an old connection got no answer for the 5 s that a slow last write took, and plan step 7's palette host would have needed its own copy. The process exit closes every connection, so without the last wait it would cut off a reply that is still being written. With the 64 MiB file of [design/daemon.md](design/daemon.md), each result took about 2 s to record. While the host still took the results queued after the 5 s, a Quit sent with 64 Lock Screens in flight had not replied after 40 s, and the daemon exited after 141 s (reproduced on 2026-10-17). |
| A successful `Launch`, `LaunchAction`, or `Run` records a visit and closes the summon, except Clipboard History, which opens it. Quit records none, so it never leads Suggestions. A failure records nothing and leaves the summon as it was. Calculator and clip rows record no visits. | Lead | Ranking reads visits only for apps and commands (`Index::learned`). TinyCast counts no contextual rows ([notes/06](research/notes/06-tinycast-behaviour-spec.md)). |
| Without GPUI the summon is a state, `open` or `closed`. `toggle` flips it, Clipboard History opens it, and every other successful activation closes it. | Lead | Lets Tier 1 tests check the close rules before the palette exists. |
| Until plan step 6, copy items fail with `not copied: no Wayland compositor`, and every clip key fails with `no clip <id>`. | Lead | The notes/07 Tier 1 spec uses both messages, and the first stays true in Tier 1 after step 6. |
| The root list's merge lives in `lyncast-core`. The calculator card is the Answer row at index 0, titled with the answer's display, and `Row` carries the `calc::Answer`. In `query --json` the card row alone has a `card` object after `title`, with `expression`, `source_badge`, `display`, `copy`, and `target_badge`. | Lead | [design/calculator.md](design/calculator.md#hyphen-queries-reach-the-engine) checks `lyncast query -(2+3)` for its Negation badge, which the six fields cannot show. The palette draws the card from the same answer. |
| `ranking.json` is one JSON object from key to `{"anchor","last_opened","terms"}`, in key order, with times in milliseconds from 1970 to the end of 9999. A member whose key no longer parses is dropped, and stored terms are folded, deduplicated, and cut to three again. The daemon reads only a regular file of at most 64 MiB, after symlinks, and writes no file larger. It checks the path before it opens the file, opens it without waiting on a FIFO, and checks the file it opened. A symlink to nothing counts as a file. A search term over 1,024 bytes is not kept. A file that cannot be read, does not parse, or holds a time out of range is renamed `ranking.json.bad`, with a warning on stderr that names it, and the daemon starts with no visits. If `ranking.json.bad` already exists or the rename fails, the visits stay in memory and the daemon never writes the file. Without an absolute data home the visits stay in memory. | Lead | Key order keeps the bytes stable between writes. A bad file is kept for the user instead of overwritten. A stored anchor of `i64::MAX` overflowed the next visit and killed a debug daemon (reproduced on 2026-10-17). Before these rules, a FIFO there kept the daemon from ever binding, and a symlink to `/dev/zero` read until 1 GiB of address space ran out. While the daemon checked the path and then opened it with an open that waits on a FIFO, a FIFO swapped in between kept 9 of 40 starts from binding (reproduced on 2026-10-17). While it opened the file before any check, a program that waited to write to a FIFO there went on (reproduced on 2026-10-17). A dangling symlink read as a fresh install and was written over, and a second bad file replaced the first (reproduced on 2026-10-17). After one visit, a valid file of 64 MiB held 67,108,947 bytes, so the next start moved it aside. A query of 1,000,000 bytes was kept whole as a term (reproduced on 2026-10-17). |
| One writer thread writes `ranking.json`. After each recorded visit the owner loop leaves a serialized copy of the visits in one slot, in place of any copy the writer has not taken. The writer writes each copy to a temporary file named with the process ID and renames it over `ranking.json`. Frecency's time arithmetic saturates, and a visit counts an infinite score as the largest `f64`, so the new anchor is 10,240 days ahead, and the entry leads Suggestions for about 28 years. | Lead | An fsync on the owner loop would stall plan step 7's foreground executor, and a `ranking.json.tmp` that blocked held up every request (reproduced on 2026-10-17). Two daemons with their own `XDG_RUNTIME_DIR` and one data home hold their own locks. While both wrote `ranking.json.tmp`, one daemon's copy followed by the tail of the other's longer copy reached `ranking.json`, and the next start moved it to `ranking.json.bad` after 4 of 5 rounds of 5 s (reproduced on 2026-10-17). While a write was blocked, a queue of copies grew the daemon from 6 to 45 MiB over 120 activations that each carried a 300 KiB term (reproduced on 2026-10-17). An anchor more than 10,240 days ahead, which a clock set back can leave, gives an infinite score. Its next visit overflowed, and a saturated anchor led Suggestions for good. |

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
| Fuzzy matching with `frizbee`, scoring with TinyCast's points, with TinyCast's ranking rules on top. | Measured | 100k items in 0.4 to 1.2 ms vs 2.9 to 5.2 ms for `nucleo-matcher`. [notes/03-wayland-niri-integration.md](research/notes/03-wayland-niri-integration.md). The weights: [App index and ranking](#app-index-and-ranking). |
| A hand-written calculator, no `fend-core` or `numbat`. | Measured | Neither matched TinyCast's fixtures. |
| Clipboard watching with `wl-clipboard-rs` 0.9.4 `Watcher`. Storage with `rusqlite`, blobs as files named by hash. | Lead | |
| Desktop entries with our own parser in `lyncast-core` and our own `Exec` tokenizer. `freedesktop-desktop-entry` was dropped on 2026-10-15. | Measured | 0.8.3 rejected the whole file for `Keywords=a\;b;` (spec-legal), a UTF-8 BOM, an indented comment, a localized key before or without its base key, and `Comment=C:\Users`. Its Exec parser split `sh -c "echo a b"` on every space. It does not dedupe IDs, compare `OnlyShowIn`, check `TryExec`, or read `LC_MESSAGES`. |
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
