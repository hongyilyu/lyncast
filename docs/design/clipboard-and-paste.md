# Clipboard history copies now and keeps one seam for paste

In the MVP, Enter on a clip or calculator row copies the entry and closes the palette. Paste must plug in later per platform without a redesign. The [decision log](../decisions.md#clipboard-and-paste) is the source of truth and overrides this page. Hongyi decided capture types, the clipboard screen, previews, and retention. A probe settled the marker. The rest are lead defaults.

The evidence is [notes/07](../research/notes/07-paste-seam-workflow.md), with three design candidates, a judge, and two reviews. Claims keep its evidence labels.

## A capture thread owns the Watcher

In `wl-clipboard-rs` 0.9.4, `Watcher::next_event` blocks in `poll` with no timeout, and the `Offer` it returns borrows the `Watcher` (`src/watch.rs:308-315`, read in source). So a separate capture thread owns the `Watcher`, applies the three filters below, and sends entries over a channel.

## History keeps text, images, and files

The thread captures text, images, and files. Files arrive as `text/uri-list` or `x-special/gnome-copied-files` and point at the original paths. lyncast derives links and emails from text on read, as TinyCast does. Unpinned entries expire after 3 months by default, and pinned entries never expire. Source is the app that niri IPC reports as focused when the selection changed. It is a best guess, because data-control names no source client.

## Three filters keep own writes, passwords, and large entries out

The watcher lists each offer's MIME types, so each filter is a string compare (`src/watch.rs:193-198`, read in source).

The first filter skips lyncast's own writes, which all carry `application/x-lyncast-own`. The marker holds the same bytes as the payload. A probe under Xvfb plus anvil read 12 copies of `image/png` with an empty marker through `MimeType::Any`, and 5 reads got the marker and 0 bytes (measured). `wl-clipboard-rs` keeps offered types in a `HashMap`, so a reader that takes the first type can pick the marker (`src/copy.rs:264`, read in source).

The second filter skips the password-manager hint `x-kde-passwordManagerHint` (`src/utils.rs:21`, read in source). The third skips any entry over 25 MB.

## The clipboard screen follows TinyCast's

Tab, or the Clipboard History command, opens the clipboard screen. Hongyi chose TinyCast's layout ([decision log](../decisions.md#clipboard-and-paste)):

- A filter input at the top. Its text filters entries, and the selection lands on the newest unpinned entry.
- A type filter with All Types, Text Only, Images Only, Files Only, Links Only, and Emails Only. Links and emails are kinds derived from text, so the filter reads them from the same rows.
- Rows grouped by day, newest first, with pins above the groups.
- A preview pane. Text shows its content. Images show a preview capped in size, decoded from the blob only for the selected row. Files show their icon, plus an image preview when the file is an image. The MVP renders no PDFs or documents. Reading the freedesktop thumbnail cache (`~/.cache/thumbnails`) is the first follow-up.
- An information panel with Source, Type, Path (files only), Size, and Copied (the capture time).
- A footer with the default action and Enter, plus Actions and Ctrl+K. The Actions menu offers Copy, Pin or Unpin, and Delete.

The preview decodes one image at a time and drops it when the selection moves. That keeps image memory bounded by one preview, not by the history size. Vicinae reached 1.2 to 2.7 GB while scrolling images on niri, as reported in its issue #998 ([notes/01](../research/notes/01-landscape.md)).

## Clips is the only owner of clip rows

`Clips` keeps the history in `rusqlite`, with blobs in files named by hash. Core keeps no copy of the rows and ranks a borrowed slice, because a second copy went out of sync in review.

## Delivery records each write before it returns

`lyncast-core` says what to hand over, never how:

```rust
pub enum Delivery { Clip(ClipId), Answer(CalcAnswer) }
```

`Clip` names a stored row, so core carries no bytes. `Clips::deliver` sets the selection through `copy_multi`, then records the write in the same call. A `Clip` promotes its row. An `Answer` inserts one, since a copied answer goes into history ([lead default](../decisions.md#lead-defaults-hongyi-can-override)). So the marker means "already recorded". Letting the watcher record answers would race the next `clip list` (inferred). If the write fails, the palette stays open, shows the reason, and keeps the row.

## The daemon owns the selection and must stay alive

smithay forwards each paste read to the client that set the selection. When that client destroys its source or exits, smithay clears the selection, and niri keeps no copy (read in source). A probe saw the selection empty when the copier exited (measured).

The palette cannot own it, because GPUI's Wayland clipboard needs a focused window and an input serial (read in source). The resident daemon owns it through `copy_multi`, which serves from a thread (`src/copy.rs:1043-1058`, read in source). `--headless` does the same.

So the GPUI app runs with `QuitMode::Explicit`, because by default GPUI on Linux quits with its last window ([process model](../decisions.md#process-model)).

## The palette reports chords and closes on key release

The palette reports a chord, Enter or Ctrl+Enter, and one pure table maps the row's subject and the chord to an action. Both chords copy in the MVP, and Ctrl+Enter is the slot paste will use. Paste then gives clip and calculator rows their own Ctrl+Enter item in the table, and the palette stays the same. Requests to the daemon are async from the start, because a paste result arrives after the window closes.

The palette closes when the activating keys are released. A held Shift or Alt would merge with an injected Ctrl+V, and the virtual keyboard cannot release the user's key. smithay drops a release while another source still holds the key (`src/input/keyboard/mod.rs:313-341`, read in source).

Three rules govern the close:

- After any successful action, the daemon schedules the close with `cx.defer`, because a nested close fails in GPUI (`gpui-pre` 0.3.9 `src/app.rs:2215`, read in source).
- `on_window_closed` clears the summon, so the toggle state follows the real window even when niri closes it.
- Losing keyboard focus cancels the summon, because under focus-follows-mouse the keyboard can move to another output (read in source).

## Paste v1 aims at summon and confirms focus from state

The decision log records only that paste backends live in the daemon's `paste/` module, starting with the niri virtual keyboard. The rest is the judge's spec with both reviews' fixes ([notes/07](../research/notes/07-paste-seam-workflow.md)), unrun because the dev box cannot build niri (measured).

Paste is copy plus a final step, so a failed paste leaves the entry on the clipboard. Like TinyCast, lyncast never restores the old clipboard (read in source).

`paste/mod.rs` holds `Paster`, a struct until a second backend makes it a trait, and an opaque `Aim`. `NoPaste` is `Sandboxed`, `NoProtocol`, `CompositorTooOld`, or `NeedsConsent`. `Missed` is `FocusMoved(Option<u64>)` or `Timeout`. A failed paste replies `CopiedNotPasted(NotPasted)`, which holds a `Missed` or a `NoPaste`.

A paste runs in this order:

1. Toggle calls `Paster::aim` before the palette maps. On niri, `aim` queries `Request::FocusedWindow` and opens the IPC `EventStream`. `FocusedWindow` returns nothing once the palette has focus (`niri-ipc/src/lib.rs:1355`, read in source). With no summon, `aim` runs at request time.
2. The daemon copies, and the window closes.
3. `fire` runs on a worker. It folds each event into `niri_ipc::state::EventStreamState` and waits until `windows[aim].is_focused` is true. A stream opened after the close could miss the refocus, because a new subscriber gets only the initial burst (`src/ipc/server.rs:247-255`, read in source).
4. Before it sends keys, `fire` checks that the capture thread has seen no selection change newer than this write. The daemon gets no other signal that its source was replaced (`src/copy.rs:361`, read in source).

Only one `fire` runs at a time, and any `Toggle` or `Deliver` cancels it. Otherwise a stale `fire` could paste a newer clip into the old target.

The virtual keyboard uploads lyncast's own keymap, with standard evdev keycodes for `Control_L`, `Shift_L`, and `v`, so Ctrl+V resolves on any layout (inferred). lyncast sends explicit presses and releases, never the `modifiers` request, whose state outlives the device (read in source). Each paste swaps the seat keymap, and anvil never swaps back, so each paste test starts a fresh anvil.

Terminals get Ctrl+Shift+V, because Ctrl+V there means literal-next. `paste/chord.rs` checks the `/proc/<pid>/exe` basename of the focused window's pid, then the app_id stem, then `Categories=TerminalEmulator`. The app_id alone misses `foot --app-id=scratch`.

`Request::Version` cannot gate paste, because v26.04 and a main build with the virtual-keyboard rework, PR #4548, both report `26.04` plus a commit (read in source). v26.04 shipped on 2026-04-25, and #4548 merged on 2026-10-13 (measured). So paste needs a release after 26.04, or a config opt-in on 26.04.

Paste v1 adds `paste/mod.rs`, `paste/niri_vk.rs`, and `paste/chord.rs`, and the reviews imply about 10 edits outside `paste/` (inferred). No code exists yet to check that count.

## Each platform needs its own injection route

Cells come from the mechanism matrix in [notes/07](../research/notes/07-paste-seam-workflow.md) and are read in source unless marked.

| Platform | Route | Known failure |
|---|---|---|
| niri | `zwp_virtual_keyboard_v1`, unsandboxed, needs PR #4548. The first backend | Released niri can break the keyboard (inferred). No RemoteDesktop portal (measured) |
| sway | `zwp_virtual_keyboard_v1`, unsandboxed | Hidden from security-context clients |
| Hyprland | `send_shortcut` on Hyprland main | Dispatchers are "not guaranteed to be stable" |
| GNOME | RemoteDesktop portal or a Shell extension | `Start` "typically" shows a dialog. Mutter has no layer shell or data-control (measured) |
| KDE | RemoteDesktop through KWin's EIS plugin (inferred) or `org_kde_kwin_fake_input` | Fake input may be locked down (inferred) |
| X11, for DCV | XTest, with a keysym lookup per key | Chrome's URL bar needs a hold delay. `Clips`, aim, and the palette need X11 ports |
| Any | uinput, with write access to `/dev/uinput` | Drops early keys and ignores the layout. Denied on the dev box (measured) |

Because the likely second backend is uinput on niri, `detect` walks an ordered list of mechanisms, each with `probe() -> Result<(), NoPaste>`.

## Eighteen review findings have fixes and four do not

The design-quality and platform reviews each raised 14 findings. A finding both raised appears once.

A decision log entry resolves these:

- Core missed own writes. `Clips` alone owns rows.
- A blocked watcher could freeze Enter. A capture thread owns the `Watcher`.
- GPUI rejects a nested close and quits with its last window. The fixes are `cx.defer` and `QuitMode::Explicit`.
- Launch and Run left the palette open. Any successful action closes it. The log has no `Held` token.
- Paste v1 would edit the palette. Chords and async requests prevent that.
- An empty marker gave 0-byte reads. The marker holds the payload.
- `Option<Summon>` went stale. `on_window_closed` clears it.
- Focus-follows-mouse moved the keyboard away. Focus loss cancels the summon.
- A held Shift or Alt merged with Ctrl+V. The palette closes on key release.
- Tests touched the real clipboard. Tier 1 clears `WAYLAND_DISPLAY`.

These fixes live in the paste v1 spec, not yet in the decision log:

- A late stream missed the refocus. `aim` opens it at toggle, and focus comes from state.
- A waiting `fire` could paste a newer clip. Single flight, cancel, and the sequence check stop it.
- CLI paste had no `Aim`. It aims at request time.
- `Missed` and `Reply` lacked two outcomes. `FocusMoved(Option<u64>)` and `NotPasted` add them.
- `Request::Version` cannot tell 26.04 from a fixed build. A release gate plus an opt-in replaces it.
- App ids miss custom terminals. The pid's executable comes first.
- One file per backend held only on niri. `detect` walks ordered probes, and the claim shrinks to key injection.
- Each paste swaps the seat keymap. The spec accepts that, with a fresh anvil per test.

These four have no fix in the log or the spec. The planned tests in [testing.md](testing.md#planned-tier-2-cases-for-copy-and-close) cover the first and the third:

- No test covers Enter, close, and still serving. Both reviews ask for a Tier 2 case.
- `deliver` returns before the compositor applies the selection. The review asks it to wait for its own `Changed` event.
- Back-to-back seeding races the watcher. The fix waits for each row before the next seed.
- A daemon restart empties the clipboard. The review suggests re-offering the newest row at startup.
