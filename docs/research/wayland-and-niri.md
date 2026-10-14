# Wayland and niri research

This page lists what research found about niri, the Wayland protocols, and the Rust crates that lyncast builds on. The [decision log](../decisions.md) is the source of truth and wins any disagreement. The evidence is in [notes/03](notes/03-wayland-niri-integration.md) and [notes/07](notes/07-paste-seam-workflow.md). Each niri `file:line` was read in source at commit 4263c9a (2026-10-13). The latest niri release is v26.04 (2026-04-25).

## niri offers every needed protocol to unsandboxed clients

| Protocol | niri source | Hidden from sandboxed clients |
|---|---|---|
| wlr-layer-shell | `src/niri.rs:2525` | Yes |
| wlr-data-control, ext-data-control-v1 | `src/niri.rs:2555, 2560` | Yes |
| xdg-activation | `src/niri.rs:2606` | Not recorded |
| zwlr-foreign-toplevel, ext-foreign-toplevel-list | `src/protocols/foreign_toplevel.rs:7-12,79` | Not recorded |
| `zwp_virtual_keyboard_v1`, wlr virtual pointer, input-method-v2 | `src/niri.rs:2571-2578` | Yes |
| text-input-v3, keyboard-shortcuts-inhibit | `src/niri.rs:2570, 2573-2574` | No |

The first four rows come from [notes/03](notes/03-wayland-niri-integration.md) and the last two from [notes/07](notes/07-paste-seam-workflow.md). The filter is `client_is_unrestricted`. A client is restricted only when it connects through security-context (`src/handlers/mod.rs:499-511`), so lyncast on the host sees every protocol in the table.

niri hides the Top layer behind fullscreen windows, so lyncast uses the Overlay layer with exclusive keyboard focus ([Process model](../decisions.md#process-model)). Activation tokens expire after 10 s (`src/handlers/mod.rs:86`), and a token without a serial only marks the window urgent. [notes/03](notes/03-wayland-niri-integration.md)

## A niri bind is the global hotkey

Wayland has no global hotkeys, so a niri bind runs `lyncast toggle` ([Process model](../decisions.md#process-model)). The syntax comes from the niri wiki page on key bindings, cited in [notes/03](notes/03-wayland-niri-integration.md):

```kdl
binds {
	Mod+Space repeat=false hotkey-overlay-title="Launcher" { spawn "lyncast" "toggle"; }
}
```

## niri IPC takes one JSON request per line on `$NIRI_SOCKET`

- Requests include `Outputs`, `Windows`, `FocusedOutput`, `FocusedWindow`, `Action`, and `EventStream`. Each `Window` has `id`, `app_id`, `title`, `pid`, `workspace_id`, and `focus_timestamp`. [notes/03](notes/03-wayland-niri-integration.md)
- `Action Spawn` runs the same code as a niri bind. It mints a token, double-forks, and puts the app in a systemd scope (`src/utils/spawning.rs:239-256`). [notes/03](notes/03-wayland-niri-integration.md)
- No IPC action injects keys. A new `EventStream` subscriber gets a state burst with no `WindowFocusChanged` (`src/ipc/server.rs:247-255`). [notes/07](notes/07-paste-seam-workflow.md)
- The `niri-ipc` crate 26.4.0 is GPL-3.0-or-later. notes/03 advised against linking it, but lyncast is GPL-3.0-or-later too and uses the crate ([Project](../decisions.md#project)).

## Focus goes back to the layout window when the palette closes

niri recomputes keyboard focus once per event-loop pass, in `update_keyboard_focus` (`src/niri.rs:1272-1405`). An exclusive layer surface counts only while it is mapped, so a null-buffer unmap or a destroy drops that state. With no fullscreen window, focus falls back to Overlay, then Top, then on-demand surfaces on Bottom and Background, then the layout's focused window (`src/niri.rs:1378, 1386-1394`). [notes/07](notes/07-paste-seam-workflow.md)

The palette never changes the layout's focused window, so focus normally returns to the window that had it at summon. Layout focus can still move while the palette is open, through niri binds or focus-follows-mouse (`src/niri.rs:6933-6965`). niri checks exclusive focus only on the active output, so losing keyboard focus cancels the summon ([Clipboard and paste](../decisions.md#clipboard-and-paste)). None of this ran against a live niri, which could not be built on the dev box. [notes/07](notes/07-paste-seam-workflow.md)

A client can watch for the change through these signals, read in source unless marked ([notes/07](notes/07-paste-seam-workflow.md)):

| Signal | Behaviour |
|---|---|
| `wl_keyboard.leave` | Arrives before niri recomputes focus if the `wl_surface` is destroyed, and GPUI destroys it on close. |
| `wl_display.sync` reply | Arrives after focus moves, because niri flushes to clients after the focus refresh. Inferred. |
| `Event::WindowFocusChanged { id: Option<u64> }` | Emitted in the same refresh as the focus change. |
| foreign-toplevel Activated state | Tracks keyboard focus. |

`Request::FocusedWindow` names the target only before the palette maps. After the palette takes focus, `FocusedWindow` returns `None`, and the stream emits `WindowFocusChanged { id: None }`. `Workspace.active_window_id` still names the layout window. [notes/07](notes/07-paste-seam-workflow.md)

## Clipboard history needs a resident data-control client

A watcher binds ext-data-control, falls back to zwlr, and reads the selection and offer events plus `receive(mime, fd)`. It needs no keyboard focus. Only connected clients get events, and the data disappears when the source app exits, so the watcher must live in a resident process. [notes/03](notes/03-wayland-niri-integration.md)

The same holds for lyncast's own copies. If lyncast exits, smithay clears the selection, and niri keeps no copy. A probe measured the selection going empty when the copier exited ([notes/07](notes/07-paste-seam-workflow.md)). For that reason the GPUI app runs with `QuitMode::Explicit` ([Process model](../decisions.md#process-model)).

| Crate | Finding ([notes/03](notes/03-wayland-niri-integration.md)) |
|---|---|
| `wl-clipboard-rs` 0.9.4, MIT or Apache | Has a blocking `watch::Watcher`. Prefers ext over wlr. Chosen ([Stack](../decisions.md#stack)). |
| `smithay-client-toolkit` 0.21.1 | No data-control module. |
| `clipboard-rs` 0.3.5 | Polls on Wayland. |

- **Threading.** `Watcher::next_event` blocks with no timeout, so a separate capture thread owns it ([Clipboard and paste](../decisions.md#clipboard-and-paste)). notes/03 also asks for a password-hint filter, item size caps, and read timeouts from day one.
- **MIME types.** Handle `text/plain;charset=utf-8`, `image/png`, `text/uri-list`, and `x-special/gnome-copied-files`. Skip any offer that includes `x-kde-passwordManagerHint`. [notes/03](notes/03-wayland-niri-integration.md)
- **Own writes.** lyncast tags them `application/x-lyncast-own`, with the same bytes as the payload. An empty marker gave first-type readers 0 bytes in 5 of 12 measured runs ([Clipboard and paste](../decisions.md#clipboard-and-paste)).
- **Storage.** `rusqlite` 0.40.2 bundles SQLite with FTS5. Blobs go in files named by hash. [notes/03](notes/03-wayland-niri-integration.md)

## App launching goes through desktop entries and niri `Spawn`

`freedesktop-desktop-entry` 0.8.3 (MPL-2.0) handles XDG directory scanning, locale, `NoDisplay`, `OnlyShowIn`, `NotShowIn`, `Terminal`, and actions. Its `Exec` parser splits on plain whitespace. Tested on `sh -c "echo a b"`, it returned `["sh","-c","\"echo","a","b\"",…]`. lyncast's own tokenizer must handle double quotes, escapes, `%f`, `%F`, `%u`, `%U`, `%i`, `%c`, and `%k`. [notes/03](notes/03-wayland-niri-integration.md)

`freedesktop-icons` 0.4.0 parses every `index.theme` on first use. Each later lookup probes files with `exists()` per directory and extension, and ignores `icon-theme.cache`. notes/03 suggests resolving icons for visible rows on a background thread, with a cache. The cost is unmeasured. [notes/03](notes/03-wayland-niri-integration.md)

GPUI creates activation tokens only inside `open_uri` and `reveal_path`, from a mouse-press serial. It has no public API for a launch from the keyboard. niri `Action Spawn` supplies the token and the systemd scope, so lyncast launches through it ([Stack](../decisions.md#stack)). Outside niri, the options are `systemd-run --user --scope` or a double-fork plus `setsid`. [notes/03](notes/03-wayland-niri-integration.md)

## frizbee beats nucleo-matcher at every measured list size

Measured on an AMD EPYC 9R14, one thread, with paths from `/usr` ([notes/03](notes/03-wayland-niri-integration.md)):

| Items | `frizbee` 0.13.0 | `nucleo-matcher` 0.3.1 |
|---|---|---|
| 1k | 5 to 8 µs | 20 to 30 µs |
| 10k | 34 to 73 µs | 250 to 440 µs |
| 100k | 0.4 to 1.2 ms | 2.9 to 5.2 ms |

The nucleo numbers include a UTF-32 conversion that nucleo's high-level crate caches. frizbee is MIT, has no dependencies, tolerates typos, and returns highlight positions from `match_list_indices`. lyncast uses frizbee with TinyCast's ranking rules on top ([Stack](../decisions.md#stack)).

## The surveyed calculator crates lost to a hand-written engine

| Crate | Finding ([notes/03](notes/03-wayland-niri-integration.md)) |
|---|---|
| `fend-core` 1.5.8, MIT | No dependencies. Has units and `evaluate_preview_with_interrupt`. Measured 8 µs init and 18 to 175 µs per query. |
| `numbat` 1.24.0, MIT or Apache | Richer units and ECB exchange rates. Measured 41 ms to load its standard library, about 0.5 ms per query, and 79 crates. |
| `evalexpr` 13.1.0, AGPL-3.0-only | Avoid. |

notes/03 recommended `fend-core`. The decision log overrides it with a hand-written calculator, because neither `fend-core` nor `numbat` matched TinyCast's fixtures (measured, [Stack](../decisions.md#stack), [Calculator engine](../decisions.md#calculator-engine)).

## Xvfb plus anvil is the tested headless compositor

| Compositor | Status | Finding ([notes/03](notes/03-wayland-niri-integration.md)) |
|---|---|---|
| Weston 13.0.3 headless | Tested | No layer-shell, data-control, xdg-activation, or `wl_seat`. |
| Xvfb 21.1.13 | Tested | XFixes 5.0 change events work, for the X11 path. |
| smithay anvil, HEAD 8a27023 | Tested | Runs inside Xvfb on llvmpipe. Offers `wl_seat` v9, layer-shell v5, zwlr-data-control v2, and xdg-activation, but no ext-data-control. The `wl-clipboard-rs` Watcher saw a copy end to end. |
| niri | Not run | `main.rs` hardcodes `headless=false`. Open PR #3953 adds `NIRI_BACKEND=headless`. niri inside Xvfb is unverified. |
| sway | Not available | Not in the AL2023 repos. |

GPUI panics without a `wl_seat`, so Tier 2 tests run under Xvfb plus anvil ([Testing](../decisions.md#testing)). anvil never restores the keymap after a virtual keyboard swap, so it cannot test keymap restore. [notes/07](notes/07-paste-seam-workflow.md)

## Every paste mechanism fails somewhere

The MVP delivers by copy only. Paste plugs in later through the daemon's `paste/` module, and the first backend is the niri virtual keyboard ([Clipboard and paste](../decisions.md#clipboard-and-paste)). The matrix comes from [notes/07](notes/07-paste-seam-workflow.md), read in source unless marked.

| Mechanism | Works on | Requires | Failure modes |
|---|---|---|---|
| `zwp_virtual_keyboard_v1` | sway, niri, anvil. Not Mutter or KWin. | Unsandboxed client | The keyboard breaks after use (niri-wm/niri#2314). PR #4548 fixes it but landed after v26.04, so released niri is still affected (inferred). Vicinae disabled paste on niri in 0.19.1. XWayland and Electron targets get nothing or wrong characters. |
| uinput | Any compositor and X11 | Write access to `/dev/uinput`, which the dev box user lacks (measured) | A new device drops keys. Evdev keycodes ignore the layout. |
| RemoteDesktop portal | GNOME. KDE probably (inferred). Not niri. | User consent. `Start()` typically shows a dialog. | Prompts again when the token is lost (inferred). |
| `org_kde_kwin_fake_input` | KWin | Unsandboxed client | May be locked down later (inferred). |
| Hyprland `send_shortcut` | Hyprland main | No device. The compositor sends the chord. | Dispatchers are "not guaranteed to be stable". |
| GNOME Shell extension | GNOME | An installed extension | May not answer until re-login. |
| X11 XTest | X11 | The XTEST extension | The keycode needs a keysym lookup. |
| input-method-v2 `commit_string` | niri, sway, anvil | Nothing. Sends text, not keys. | One input method per seat, so it conflicts with fcitx5 and IBus (inferred). |

### The niri virtual keyboard swaps the keymap for the whole seat

Read in source ([notes/07](notes/07-paste-seam-workflow.md)):

- On the first key event, niri swaps the seat keymap to the virtual keyboard's keymap, and smithay sends it to every client (`src/handlers/virtual_keyboard.rs:153-199`). If lyncast echoes the user's keymap, niri skips the swap, and lyncast must find the keycode for "v" itself.
- The virtual keyboard cannot release a key that a physical keyboard holds. A held Shift merges with an injected Ctrl+V, so the palette closes when the activating keys are released ([Clipboard and paste](../decisions.md#clipboard-and-paste)).

### Failure modes recur across tools

notes/07 found these patterns in the tools that paste:

- **Focus race.** Every tool sleeps or polls. clipse waits 10 ms and Vicinae 150 ms.
- **Terminal chord.** Terminals read Ctrl+V as literal-next and need Ctrl+Shift+V.
- **PRIMARY.** Shift+Insert pastes PRIMARY in foot and kitty.
- **Setup outside the app.** udev rules and portal dialogs push tools to ship a capability probe plus a copy fallback.
