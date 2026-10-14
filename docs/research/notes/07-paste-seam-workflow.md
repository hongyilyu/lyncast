# Paste seam design workflow (9 agents, 2026-10-14)

Raw workflow outputs, kept verbatim for provenance. Paths under /tmp were scratch space and may no longer exist.


## Grounding: TinyCast paste and copy contract

Sources. C, P, K, T, A = /tmp/tinycast-docs/{clipboard,palette,calculator,testing,architecture}.md. S = /tmp/paste-seam-scratch/upstream/snippets.md (abue-ammar/tinycast@57e29b4). Local C and P match upstream byte for byte (measured, diff -q). W = ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wl-clipboard-rs-0.9.4. N = /tmp/paste-seam-scratch/niri/niri.rs (niri@4263c9a).

1. Delivery modes (read-in-source)
- Paste, Enter: write entry, activate previousApp, synthetic Cmd+V, promote row (C:11, C:367-368, P:620-623).
- Copy, Cmd+Enter: write and promote, no paste (C:11, C:367-368, T:439). Whether Copy closes the palette is undocumented.
- Paste as Plain Text, Ctrl+Cmd+Enter: writes plainText only. A file gives its path. An image falls back to Cmd+Enter (C:17-19, T:445).
- Paste and Keep Open, Opt+Enter: pasteInPlace posts Cmd+V to the target pid without activating. No promote. Ignores the default setting (C:14-15, C:366-368, P:624-625).
- clipboardDefaultAction moves one action to Enter; Paste takes the vacated chord (C:9-12, T:446-447).
- Pin slots Cmd+1 to Cmd+0 and double-click use the same activate(_:chord:) (C:13-14, T:468).
- Paste Sequentially, a bindable command: next older entry via pasteInPlace, no activate, no promote (C:475-478).
- Copy Text, Shift+Cmd+T: closes the palette, copies OCR text (C:211-226).

2. Ordering (read-in-source)
- Summon: show() records previousApp and resolves PasteTarget once per summon (P:39-40, P:52). Pop-to-root keeps the target (P:637-638).
- Activate: Paster.write writes the pasteboard, then paste and copy promote (C:367-368). Hide returns the keyboard to the other app (P:629-633). Paste activates previousApp, then posts Cmd+V (P:623). pasteInPlace posts Cmd+V 50ms after the write (C:491).
- The hide versus activate order inside Paster.paste is undocumented. Snippets read previousApp before hiding (S:236-237).
- No restore after a history paste. The pasteboard keeps the pasted entry (C:487-488). Snippet loans do restore, after a changeCount check (S:359-361, S:374-376).
- Vanished file: write returns false, HUD, row kept (C:395-397).
- Missing Accessibility: docs say only "requires" (P:23-24, P:627). The fallback is undocumented.
- Pinned rows never promote (C:362-364).

3. Own-write marker (read-in-source)
- Every Tinycast write stamps internalType. The poller skips it. Disagreement re-captures pastes in a loop (C:20-21, C:77-79).
- So re-delivery adds no row. History moves only by explicit promote, which keeps the UUID (C:137-140).
- New text is deliberately unmarked and recorded: Copy Text, Copy Path (C:224-225).
- A borrowed Cmd+C uses pauseCapture, not the marker (C:81-84). Writes drain the poller first (C:480-482).
- Snippet loans mark only the lent item, so the restored clipboard is unmarked (S:377-379).

4. Other deliveries (read-in-source)
- Calculator: Enter copies the answer and records history. Cmd+Enter records and rewrites the query (K:619-621, T:695). T:409 says Enter "pastes 3,8", a doc conflict. The calculator copy's marker status is undocumented.
- Snippets use their own funnel: AX write, keystrokes, or a leased paste. Not Paster (S:236-239, S:321-326).
- Emoji names the paste target (P:635-636). Its write path is undocumented.
- Paster lives in the effect layer (A:45).

5. lyncast translation (inferred unless marked)
Pure core:
- One function maps (source kind, chord) to a DeliveryMode. MVP has Copy only. Paste variants are reserved enum cases.
- Promotion policy: copy promotes; pinned rows never promote.
- Payload choice: text, file path, image.
- Activation returns effects in order: WriteClipboard { payload, own: true }, Promote(id), RecordCalc, Dismiss.
- Capture policy drops any offer carrying the own marker or a password hint.
- The summon target arrives at Toggle as a plain value.
Backend:
- Marker: offer a private MIME type, such as application/x-lyncast-own, beside the text via copy_multi (W/src/copy.rs:78-81, :491). Watcher reports mime_types in protocol order, so the check is a string compare (W/src/watch.rs:193-198, read-in-source). Also skip x-kde-passwordManagerHint (W/src/utils.rs:21, read-in-source).
- Serving: the resident process must keep the data source alive. Non-foreground copy serves from a thread (W/src/copy.rs:395-399, read-in-source). Headless mode must do the same.
- Focus: niri recomputes keyboard focus. With no exclusive Overlay surface it falls back to layout focus (N:1323-1330, N:1378-1394, read-in-source). So closing the window restores focus with no activate call.
- Target: query FocusedWindow over niri IPC at Toggle. Store it for future paste only.
- Future paste needs synthetic Ctrl+V. Unverified on niri. Out of MVP.
- Tests: Tier 2 asserts that a copy offers the marker MIME and adds no row. Tier 1 asserts the promote rules.

## Grounding: niri internals

niri on 4263c9a provides every primitive a paste feature needs. The catch is that it recomputes focus once per event-loop pass. After that pass, focus goes to whatever window the layout has active at dismiss time, which may not be the window that was focused at summon. Nothing here was run against a live niri: the libinput, libseat and gbm dev libraries are missing, so niri cannot be built locally. The only measured items are gh queries.

Sources: niri is /tmp/research-wl/niri. smithay is /tmp/paste-seam-scratch/smithay-pin, checked out at rev 9786228, which is the pin in niri Cargo.lock:3399.

**1. Injection protocols**
- Virtual keyboard, wlr virtual pointer and input-method-v2 are all registered behind `client_is_unrestricted` (niri src/niri.rs:2571-2578). [read-in-source]
- text-input-v3 and keyboard-shortcuts-inhibit are not filtered (src/niri.rs:2570, 2573-2574). [read-in-source]
- A client is "restricted" only when it connects through security-context (src/handlers/mod.rs:499-511). A host lyncast sees all of these protocols. [read-in-source]
- There is no config switch for any of them. The only clipboard setting is disable-primary (niri-config/src/misc.rs:105-117). [read-in-source]
- The RemoteDesktop/libei route does not exist. The ScreenCast code rejects remote-desktop sessions (src/dbus/mutter_screen_cast.rs:113-117), and issue #390 asking for the portal is still open (https://github.com/niri-wm/niri/issues/390). [read-in-source, measured]
- Only one input method is allowed per seat. A second one is told "unavailable" (smithay src/wayland/input_method/input_method_handle.rs:58-69). [read-in-source]

**2. Focus after dismiss**
- Focus is recomputed once per loop iteration, in `update_keyboard_focus` (src/niri.rs:860, 1272-1405). The code comment at src/niri.rs:1141-1144 says so. [read-in-source]
- An exclusive surface only counts while it is mapped (src/niri.rs:1341). Unmapping with a null buffer drops that state (src/handlers/layer_shell.rs:181-186), and so does destroying the surface (src/handlers/layer_shell.rs:47-69). [read-in-source]
- Without a fullscreen window, focus falls back in this order: Overlay, then Top, then on-demand surfaces on Bottom and Background, then the layout's focused window (src/niri.rs:1378, 1386-1394). Our surface never changes the layout's focused window. [read-in-source]
- The layout's focus can still move while the palette is open. niri binds keep working, and focus-follows-mouse changes the active window and output (src/niri.rs:6933-6965). Exclusive focus is only checked on the active output (src/niri.rs:1306-1308). [read-in-source]
- **Signals the client can observe:**
  - `wl_keyboard.leave` on our surface. smithay sends leave and then enter in one call (smithay src/input/keyboard/mod.rs:57-69, 1693-1698). This only proves anything if our wl_surface survives. If the wl_surface is destroyed, a destruction hook sends leave right away, before niri recomputes focus (smithay src/wayland/seat/keyboard.rs:221-233). [read-in-source]
  - A `wl_display.sync` roundtrip. niri flushes to clients only at src/niri.rs:826, after the focus refresh, so the reply should arrive after focus has moved. [inferred]
  - IPC events `Event::WindowFocusChanged { id: Option<u64> }` (niri-ipc/src/lib.rs:1673-1676) and `WindowFocusTimestampChanged` (niri-ipc/src/lib.rs:1682-1687). Both are emitted in the same refresh (src/ipc/server.rs:739-745, 782-784). [read-in-source]
  - The foreign-toplevel Activated state, which also tracks keyboard focus (src/protocols/foreign_toplevel.rs:129, 619-620). [read-in-source]

**3. Learning the target at summon**
- `Request::FocusedWindow` returns the id, app_id, pid and title (niri-ipc/src/lib.rs:1333-1372; src/ipc/server.rs:338-342). pid may be None (niri-ipc/src/lib.rs:1346-1350). [read-in-source]
- This is only reliable before our overlay maps. Once our surface has focus, the window is marked unfocused (src/niri.rs:1416-1422), FocusedWindow returns None, and the stream emits `WindowFocusChanged { id: None }`. niri-ipc/src/lib.rs:1355 documents this. [read-in-source]
- While the overlay is up, `Workspace.active_window_id` (niri-ipc/src/lib.rs:1479) still names the layout window (src/ipc/server.rs:627; src/layout/workspace.rs:488-494). [read-in-source]

**4. Virtual keyboard**
- There is no per-device keyboard state. On the first key or modifier event, niri swaps the single seat keymap to the virtual keyboard's keymap (src/handlers/virtual_keyboard.rs:153-199, comment at 110-112). smithay then sends that keymap to every client's keyboard (smithay src/input/keyboard/mod.rs:855-871). [read-in-source]
- The user's keymap comes back on the next physical key press (src/input/mod.rs:429-432) or when the virtual device is removed (src/handlers/virtual_keyboard.rs:69-82). [read-in-source]
- If we upload our own keymap, Ctrl+V should resolve through that keymap whatever the user's layout, including Dvorak. [inferred]
- If we send `no_keymap` or echo back the user's own keymap, niri skips the swap (src/handlers/virtual_keyboard.rs:154-158, 171-177). Then lyncast has to find the keycode for "v" in the user's layout itself. [read-in-source]
- Virtual keys go through niri's keybindings (src/handlers/virtual_keyboard.rs:3, 47-49). Modifiers set with the `modifiers` request stay set after the device is removed (src/handlers/virtual_keyboard.rs:84-108). [read-in-source]
- **GitHub state** [measured with gh]:
  - The virtual-keyboard rework, PR #4548, merged on 2026-10-13. `gh api compare` shows 4263c9a is one commit after it. It fixed #403, #3274, #3394 and #3595.
  - #2314 ("wtype makes input unusable") was closed on 2026-10-13.
  - #2280 is still open: wtype types gibberish on Dvorak when focus changes. It was filed against 25.05.1.
  - niri main has one newer commit, fc9e8c4 (2026-10-14). It updates smithay to 8a27023, which fixes garbled text when fcitx holds a keyboard grab. 4263c9a does not have this fix.
  - Terminals need Ctrl+Shift+V (https://github.com/niri-wm/niri/discussions/3648, https://github.com/niri-wm/niri/discussions/2961).

**5. Clipboard ownership**
- lyncast must stay alive. When an app reads a paste, smithay forwards the request to the client's source (smithay src/wayland/selection/offer.rs:177-183). [read-in-source]
- If lyncast destroys its source or exits, the selection is cleared (smithay src/wayland/selection/ext_data_control/source.rs:67-108). niri keeps no copy; it only owns selections for its own screenshots (src/niri.rs:6345-6356). [read-in-source]
- When another client replaces the selection, our source gets `cancelled` and the focused app is offered the new selection (smithay src/wayland/selection/seat_data.rs:59-74). [read-in-source]
- A stale offer still points at our old source. If we already destroyed it, the app reads EOF. [inferred]
- Selection offers follow keyboard focus (src/handlers/mod.rs:108-113). [read-in-source]

**Constraints a paste design on niri must honor**
1. Record the target with `FocusedWindow` before the overlay maps.
2. Before injecting, confirm `WindowFocusChanged { id: Some(target) }`. Otherwise do a sync roundtrip and query again. Abort if focus is somewhere else.
3. If `wl_keyboard.leave` is the signal, unmap with a null commit rather than destroying the wl_surface.
4. Never send keys in the same request batch as the unmap.
5. Set the selection before dismissing. Keep the daemon serving it until it is cancelled.
6. Tag lyncast's own writes with a private MIME type so its watcher skips them.
7. Run lyncast unsandboxed. Under security-context the protocols disappear.
8. Do not depend on input-method-v2 while another input method is running.
9. Choose the paste chord per app (Ctrl+V or Ctrl+Shift+V).
10. Send explicit key presses and releases. Avoid the `modifiers` request. Destroy the virtual keyboard when done.
11. Expect a seat-wide keymap swap if lyncast uploads its own keymap. Require niri with #4548. Track fc9e8c4 for fcitx users.

## Grounding: prior art and platform matrix

Labels: [M] measured, [S] read-in-source (code or doc), [I] inferred. Path prefixes: V=/tmp/research-wl/vicinae/src, LK=/tmp/paste-seam-scratch/src/look/apps/linows, NI=/tmp/research-wl/niri, SM=/tmp/research-wl/smithay, CQ=/tmp/paste-seam-scratch/src/copyq, CS=/tmp/research-wl/clipse, EL=/tmp/paste-seam-scratch/src/elephant/internal/providers, SC=/tmp/paste-seam-scratch.

## (a) Mechanism matrix

| Mechanism | Works on | Requires | Failure modes |
|---|---|---|---|
| zwp_virtual_keyboard_v1 (wtype, look) | sway (SC/sway/server.c:127), niri (NI/src/niri.rs:2575), anvil (SM/anvil/src/state.rs:718) [S]. Not in Mutter main src/wayland (gh tree) or KWin's protocol list (SC/kwin/wayland_CMakeLists.txt). This contradicts LK/backend/src/platform/linux/virtual_keyboard.rs:6 [S] | Unsandboxed client. sway and niri hide it from security-context clients (server.c:99-149, niri.rs:2498) [S]. look uploads its own keymap (virtual_keyboard.rs:30-66) [S] | On niri, after the VK exits the keyboard stops working or has the wrong keymap (niri-wm/niri#2314, vicinae#994) [S]. Fixed by niri PR #4548, merged 2026-10-13, after release v26.04 [S]. Released niri is still affected [I]. XWayland/Electron targets get nothing or wrong characters (atx/wtype#62, #38) [S] |
| uinput (ydotool, dotool, Vicinae, clipse) | Any compositor and X11. It is a kernel device (SC/ydotool-README.md:83) [S] | Write access to /dev/uinput: a udev rule plus the input group (SC/dotool-1.md:18-19), or setcap cap_dac_override (V/../scripts/install.sh:402-410) [S]. On this host /dev/uinput is root 0600 and the user is not in the input group [M] | A new device registers late and drops keys, so tools keep a daemon (ydotool-README.md:85-91; dotool-1.md:110-112) [S]. Evdev keycodes ignore the keyboard layout (dotool-1.md:32-35) [S]. Vicinae sends raw KEY_V (V/snippet/src/server.cpp:182), which is likely Ctrl+K on Dvorak [I] |
| RemoteDesktop portal | GNOME, through org.gnome.Mutter.RemoteDesktop (xdg-desktop-portal-gnome src/remotedesktop.c:61,127) [S]. KDE probably through KWin's EIS plugin (KDE/kwin src/plugins/eis) [I]. niri has Mutter ScreenCast only (NI/src/dbus/) [S], so no RemoteDesktop [I] | Start() "typically" shows a dialog. persist_mode and restore_token exist since v2. Keyboard calls need the KEYBOARD device (flatpak.github.io/xdg-desktop-portal RemoteDesktop doc) [S] | Prompts on first use or when the token is lost [I] |
| org_kde_kwin_fake_input | KWin | Unsandboxed client. authenticate always succeeds, "TODO: make secure" (SC/kwin/fakeinputbackend.cpp:118-124; wayland_server.cpp:131-163) [S] | May be locked down later [I] |
| Hyprland send_shortcut | Hyprland main | The compositor sends mods+key to a chosen window, with no device (hyprwm/Hyprland src/config/shared/actions/ConfigActions.cpp:1571-1640) [S] | Dispatchers are "not guaranteed to be stable", and config moved to Lua (hyprland-wiki content/configuring/core/dispatchers.md:9,78) [S] |
| GNOME Shell extension | GNOME | Installed extension that drives a Clutter virtual device over D-Bus (LK/backend/src/gnome-shell-extension/extension.js:55-75) [S] | Extension may not answer until re-login (LK/backend/src/paste.rs:43-48) [S] |
| X11 XTest | X11 | XTEST extension (LK/backend/src/platform/linux/keysynth.rs:36-44; CQ/src/platform/x11/x11platformwindow.cpp:48) [S] | Keycode must come from a keysym lookup (keysynth.rs:105-121). Chrome's URL bar needs a hold delay (x11platformwindow.cpp:132) [S] |
| input-method-v2 commit_string | niri, sway, anvil (niri.rs:2571; server.c:463; state.rs:717) [S] | No keystroke. Text only | One IM per seat. A second one gets `unavailable` (SM/src/wayland/input_method/input_method_handle.rs:59-62) [S], so it conflicts with fcitx5/IBus [I]. Reaches only an active text-input (:256-259) [S] |
| niri IPC | none | No key-injection Action (grep of NI/niri-ipc/src/lib.rs) [S] | n/a |

## (b) How each tool structures paste

- **Vicinae:** One interface, `AbstractPasteService::pasteToApp(window, app)` (V/server/src/services/paste/abstract-paste-service.hpp:5-10), picked per OS at compile time (V/server/src/server.cpp:285-307). On Linux it sends Ctrl+V or Ctrl+Shift+V through a setcap'd uinput helper over stdio RPC (V/server/src/services/paste/linux-paste-service.cpp:10-16). A shared orchestrator copies, then polls for focus on another window (5 ms, 5 s cap) or sleeps 150 ms. It pastes and restores the old clipboard after 800 ms (paste-service.cpp:4-78). A config option picks Paste or Copy, and Paste is offered only when supported (V/server/src/builtins/clipboard/clipboard-preferences.hpp:108-125). Its own writes carry `vicinae/concealed` (V/lib/common/include/common/clipboard-formats.hpp:5). [S]
- **look (has a GPUI frontend):** `paste_into_focused_app` hides the window, waits until it is hidden, waits for named focus, settles 80 ms, then sends the chord (paste.rs:18-73). Backends are tried in order: VK, GNOME extension, XTest (keysynth.rs:21-34). `clipboard_paste_blocker()` returns a reason while the window is still up (paste.rs:28-55; LK/gpui/src/launcher.rs:1481-1499). Paste is bound to Ctrl+I, separate from copy. [S]
- **clipse:** A `DS` interface per display server with `SendPasteKey` (CS/display/ds.go:13-22). After the TUI exits it spawns a detached `clipse --auto-paste` (CS/cmd/root.go:172-173). One global keybind, default ctrl+v, off by default (CS/config/constants.go:15-17). It creates a fresh uinput device per paste (CS/handlers/uinput.go:104). The buffer value is used as microseconds in one place (root.go:256) and milliseconds in another (uinput.go:140). [S]
- **CopyQ:** `PlatformWindow::pasteFromClipboard()` (CQ/src/platform/platformwindow.h:34). On X11 it sends Shift+Insert unless the window title matches a Ctrl+V regex option (x11platformwindow.cpp:327-334; platformcommon.cpp:20-45). On Wayland it needs a user script that overrides `paste()` with `ydotool key` Shift+Insert after 100 ms (SC/wayland-support.ini:91-117). [S]
- **Elephant/Walker:** Clipboard is copy-only, through a configurable shell command that defaults to `wl-copy` (EL/clipboard/types.go:56). Snippets type text with `wtype %CONTENT%` after 100 ms (EL/snippets/setup.go:35-63,84-90). [S]
- **zlaunch, cliphist, GPaste:** Copy only. zlaunch flattens rich text to plain (/tmp/research-gpui/zlaunch/src/ui/modes/clipboard_mode.rs:36-69). cliphist pipes to wl-copy (/tmp/research-wl/cliphist/readme.md:37). GPaste has no key synthesis (grep of SC/src/gpaste/src). [S]

## (c) Recurring failure modes

1. **Focus race.** Every tool sleeps or polls: clipse 10 ms, Elephant and CopyQ 100 ms, Vicinae 150 ms or a focus poll, look 80 ms after focus. On niri the focus signal is EventStream (V/server/src/services/window-manager/niri/niri.cpp:53,372) [S].
2. **Terminal chord.** Ctrl+V is literal-next in terminals (paste.rs:145). Vicinae detects terminals by the .desktop `Categories=TerminalEmulator` or `Terminal=true` (V/server/src/services/app-service/xdg/xdg-app.hpp:19-20). look uses an app-id table and matches the stem of reverse-DNS Flatpak ids (paste.rs:146-158) [S].
3. **PRIMARY.** Shift+Insert pastes PRIMARY in foot (SC/foot.ini:209) and kitty (SC/kitty-conf.py:3879) [S]. So CopyQ's chord pastes the wrong selection there [I].
4. **Broken keyboard after VK use** (niri#2314, vicinae#994, vicinae#869). Vicinae disabled paste on niri in 0.19.1 (#994 comment, 2026-01-22) [S].
5. **Layout and permissions.** Raw keycodes break on non-QWERTY layouts. look avoids this by uploading a keymap or looking up keysyms [S]. Setup happens outside the app (udev, setcap, extension, portal dialog). So tools ship a capability probe plus a copy fallback (Vicinae `supportsPaste`, look's blocker) [S].
6. **Sandboxing.** A Flatpak'd launcher loses VK and IM on sway and niri (server.c:148; niri.rs:2571-2578) [S]. Sandboxed target apps still receive keys [I].
7. **Clipboard restore** can race slow readers (Vicinae 800 ms) [I].

Seam hint [I]: keep Copy in core. Later, add a `Deliver` port with `probe() -> Result<(), Reason>` and a target app_id captured at summon (niri FocusedWindow). Choosing the chord then stays a pure core rule.

## Design candidate A

# Delivery seam: copy now, paste later

Labels: [M] measured, [S] read-in-source, [I] inferred. Prefixes: NI=/tmp/research-wl/niri, SM=/tmp/paste-seam-scratch/smithay-pin, W=~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wl-clipboard-rs-0.9.4, C=/tmp/tinycast-docs/clipboard.md, V=/tmp/research-wl/vicinae/src.

## Problem

MVP stops at the clipboard. Paste adds strictly ordered steps. Read the target before our surface maps, since a focused layer surface leaves no focused window (NI/niri-ipc/src/lib.rs:1355) [S]. Own the selection before close and keep it alive, or it clears (SM/src/wayland/selection/ext_data_control/source.rs:67-108) [S]. Keys wait for confirmed focus, since niri recomputes focus per loop pass (NI/src/niri.rs:1378-1394) [S]. Own writes carry a marker (C:20-21) [S]. I honor constraints 1, 2, 4, 5 and 6, and none reach lyncast-core.

## Usage

UI: Enter sends the CLI's request and never closes the window.

```rust
fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
    if let Reply::Failed(why) = self.daemon.send(Request::Activate { key: self.selected.clone() }) {
        self.status = Some(why); // window stays up, row kept
        cx.notify();
    }
}
```

The daemon (`lyncast/src/session.rs`) writes the order once.

```rust
Request::Activate { key } => match self.core.activate(&key)? {
    Action::Launch { app, action } => self.niri.spawn(app, action),
    Action::Deliver(d) => self.deliver(d),
    Action::Run(cmd) => self.run(cmd),
},
Request::ClipCopy { id } => self.deliver(self.core.redeliver(id)?),

fn deliver(&mut self, d: Delivery) -> Result<Reply, Failure> {
    let held = self.clips.hand_over(&d)?; // fails while the palette is up
    if let Some(s) = self.summon.take() { s.end(&held) }
    Ok(Reply::Copied(d.to_string()))
}
```

CLI: `lyncast clip copy 2` prints `copied 2`. Later, `lyncast clip paste 2` prints `pasted 2`.

A later backend is one file plus one arm in `paste::pick`'s exhaustive `match` on `Compositor`.

```rust
// lyncast/src/paste/hyprland.rs
impl Paster for Hyprland {
    fn probe(&self) -> Result<(), Unavailable> { todo!() }
    fn aim(&self) -> Option<Aim> { todo!() } // before our surface maps
    fn finish(&self, after: Closed, aim: Aim) -> Result<Pasted, Missed> { todo!() }
}
```

## Shape

`lyncast-core` (pure) decides what to hand over. It never says copy or paste.

```rust
pub struct ClipId(i64);
pub struct BlobHash([u8; 32]);
pub enum Payload { Text(String), Blob { mime: Mime, hash: BlobHash } }
pub enum Delivery {
    Redeliver { clip: ClipId, payload: Payload }, // marked, promotes
    Fresh(String),                                // calc answer: unmarked, recorded
}
pub enum Action { Launch { app: AppId, action: Option<ActionId> }, Deliver(Delivery), Run(Command) }
impl Core {
    pub fn activate(&self, key: &Key) -> Result<Action, NoSuchRow> { todo!() }
    pub fn redeliver(&self, id: ClipId) -> Result<Delivery, NoSuchClip> { todo!() }
}
```

`lyncast-clip` (wl-clipboard-rs, rusqlite, blobs) is the only crate that knows the marker, the password hint and serving.

```rust
pub struct Held(()); // proof: the selection is ours and served
impl Clips {
    pub fn hand_over(&mut self, d: &Delivery) -> Result<Held, HandOverError> { todo!() }
    /// Own writes and password hints never come out.
    pub fn next_capture(&mut self) -> Result<Capture, WatchError> { todo!() }
}
pub enum HandOverError { BlobMissing(ClipId), Compositor(String) }
```

`hand_over` offers `application/x-lyncast-own` beside the payload via `copy_multi` (W/src/copy.rs:491) and serves from a thread (W/src/copy.rs:395-399) [S]. The watcher lists MIME types (W/src/watch.rs:193-198), so skipping the marker and `x-kde-passwordManagerHint` (W/src/utils.rs:21) is a string compare [S].

The daemon owns one summon's life.

```rust
struct Summon { palette: ui::Palette } // `Option<Summon>` is the toggle state
impl Summon {
    fn end(self, _: &Held) { self.palette.close() } // needs a held selection
    fn cancel(self) { self.palette.close() }        // Escape, second toggle
}
```

`hand_over` promotes only after the write, and `end` needs its `Held`. A vanished file shows in the palette and the row survives (C:395-397) [S]. The UI renders the Enter hint the session sends. MVP sends "Copy".

At summon, MVP captures nothing for delivery. Nothing would read it.

Paste adds, in the daemon crate only:
- `paste/` and `Paster`, a trait once a second backend exists.
- `Summon.aim`, filled in `toggle()` before the window opens. After the overlay maps, niri reports no focused window (NI/niri-ipc/src/lib.rs:1355) [S]. CLI paste aims at request time [I].
- `end` returns `Closed`, whose constructor is private, so `finish` cannot run early.
- `Focused`, private to each backend. niri builds it only from `WindowFocusChanged { id: Some(aim) }` (NI/niri-ipc/src/lib.rs:1673-1676) [S].
- `Request::ClipPaste { id }`.

Core and UI do not change.

## Testing

Tier 2 runs `lyncast --headless` on Xvfb plus anvil, with wl-copy and wl-paste as other apps.

```
$ wl-copy hello; wl-copy world; lyncast clip list
2	world
1	hello
$ lyncast clip copy 1; wl-paste; wl-paste --list-types | grep lyncast
copied 1
hello
application/x-lyncast-own
$ lyncast clip list
1	hello
2	world
```

The last list shows promotion, no new row. A later `wl-paste` printing `hello` shows serving. `lyncast calc copy '2*3'` prints `copied 6` with no marker, and the list gains `3	6`. With clip 4's blob deleted, `lyncast clip copy 4` exits 1 with `lyncast: clip 4: blob missing` and `wl-paste` still prints `hello`.

Tier 1: `lyncast clip copy abc` exits 2 with `lyncast: "abc" is not a clip id`.

Later, a fake niri socket replays `WindowFocusChanged { id: Some(7) }` and an anvil client prints `hello`, typed by anvil's virtual keyboard (/tmp/research-wl/smithay/anvil/src/state.rs:718) [S]. Replaying `Some(8)` prints `copied 1, not pasted: focus on window 8`.

## Tradeoffs

- We accept that the first paste change edits daemon, protocol and CLI, in exchange for no paste code in MVP.
- We accept Enter meaning "deliver" in exchange for no UI change when paste lands. A copy-only chord is later UI work.
- We accept that GPUI destroys the window, so `wl_keyboard.leave` fires before niri refocuses (SM/src/wayland/seat/keyboard.rs:221-233) [S], in exchange for an unpatched GPUI. Paste waits on niri IPC.
- We accept no restore after paste, as TinyCast does (C:487-488) [S], in exchange for no race with slow readers [I].
- We accept the calc answer entering history, following TinyCast's rule for new text (C:224-225) [S].

## Alternatives considered

- A core mode enum with reserved `Paste` variants and a `Deliver` port with a `CopyOnly` adapter, like Vicinae's `supportsPaste` (V/server/src/services/paste/abstract-paste-service.hpp:5-10) [S]. Reserved variants are dead code, one adapter is a guess, and core learns chords.
- Core returns an ordered `Vec<Effect>`. Order becomes data, a reorder compiles, and `Inject` edits core.

## Open questions and risks

- Does `WindowFocusChanged { id: Some(_) }` follow our destroyed surface? Exclusivity needs a mapped surface (NI/src/niri.rs:1341) [S]. The event should come from the next refresh (NI/src/ipc/server.rs:739-741) [I]. Unrun, since libinput, libseat and gbm are missing here [M].
- Latest release v26.04 (2026-04-25) predates PR #4548, merged 2026-10-13 [M]. The niri probe must gate on `Request::Version` (NI/niri-ipc/src/lib.rs:69) [S].
- Terminals need Ctrl+Shift+V (/tmp/paste-seam-scratch/src/look/apps/linows/backend/src/paste.rs:145) [S]. Vicinae reads `Terminal=` and `TerminalEmulator` (V/server/src/services/app-service/xdg/xdg-app.hpp:19-20) [S]. If MVP drops those keys, paste edits core once. niri app_id may differ from the .desktop id [I].
- Paste and keep open (C:14-15) [S] skips promote and close, so it edits core and UI.

## Design candidate B

# Paste seam for lyncast: copy now, paste later

Labels: [M] measured, [S] read-in-source, [I] inferred. NI=/tmp/research-wl/niri. W=~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wl-clipboard-rs-0.9.4. C=/tmp/tinycast-docs/clipboard.md. V=/tmp/research-wl/vicinae/src/server/src/services/paste. LK=/tmp/paste-seam-scratch/src/look/apps/linows/backend/src. P=/tmp/paste-seam-scratch/probe/watch.log.

## Problem

Enter must copy the chosen clip or calc answer and close the palette. Paste backends must plug in later without edits to lyncast-core or the palette. The MVP ships no paste code. Vicinae copies, then sends the chord (V/paste-service.cpp:19,76) [S]. look reports a failed chord as "the clip is still on the clipboard" (LK/paste.rs:70) [S]. So paste is copy plus a tail step, and copy is the whole MVP. The MVP honors constraints 5 (selection set before dismiss, then served) and 6 (own-write marker). Constraint 1 (target captured before the overlay maps) and the injection rules belong to the future backend.

## Usage

All MVP call sites reach one daemon function.

```rust
// lyncast/src/ui/palette.rs: Enter never says copy or paste.
fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
    let Some(row) = self.selected() else { return };
    self.daemon.send(Request::Activate { key: row.key.clone(), action: None }, cx);
}
// lyncast/src/cli.rs: `lyncast activate clip:42` prints "copied".
Cmd::Activate { key } => client.call(Request::Activate { key: key.parse()?, action: None }),
// lyncast/src/daemon.rs
Request::Activate { key, action } => match self.core.resolve(&key, action) {
    Ok(Action::Deliver(d)) => self.deliver(d),
    Ok(other) => self.run(other),
    Err(e) => Reply::Error(e.to_string()),
},
```

The MVP has no `clip copy` verb. With one verb it would equal `activate`.

Paste v1, the first backend, edits the daemon and protocol once:

```rust
// Toggle handler, before the window opens:
self.session.aim = self.paster.as_ref().and_then(Paster::aim);
// deliver(), after self.palette.close():
if let (Enter::Paste, Some(p), Some(aim)) = (self.config.enter, &self.paster, self.session.aim.take()) {
    return p.fire(aim).map_or_else(Reply::CopiedNotPasted, |()| Reply::Pasted);
}
// `lyncast clip copy 42`, `lyncast clip paste 42` (paste aims at request time):
Request::Deliver { clip: ClipId, verb: Verb } // enum Verb { Copy, Paste }
```

Every later backend touches only `paste/`:

```rust
// lyncast/src/paste/mod.rs: add a variant, fill the arms the compiler flags.
enum Backend { NiriVk(niri_vk::NiriVk), XTest(xtest::XTest) }
```

## Shape

In pure lyncast-core, `Deliver(Delivery)` replaces `Copy(Payload)`, because the store holds the bytes.

```rust
pub struct ClipId(i64);
pub struct CalcAnswer { text: String }   // built only by calc::eval
pub enum Action {
    Launch { app: AppId, action: Option<DesktopAction> },
    Deliver(Delivery),                    // what to hand over, never how
    Run(Command),
}
pub enum Delivery {
    Clip(ClipId),       // in history: write marked, then promote
    Answer(CalcAnswer), // new text: write unmarked, the watcher records it
}
```

No bool picks the marker, so an answer cannot be marked and a clip cannot skip promotion.

The daemon crate holds platform code:

```rust
// clipboard.rs: the only module that knows MIME strings. Watcher and writer share it.
const OWN: &str = "application/x-lyncast-own";
impl Clipboard {
    pub fn put_clip(&self, c: &StoredClip) -> Result<(), Error> { todo!() } // reps + OWN
    pub fn put_text(&self, t: &str) -> Result<(), Error> { todo!() }        // unmarked
}
// daemon.rs
fn deliver(&mut self, d: Delivery) -> Reply {
    let put = match &d {
        Delivery::Clip(id) => self.store.get(*id).and_then(|c| self.clipboard.put_clip(&c)),
        Delivery::Answer(a) => self.clipboard.put_text(a.text()),
    };
    if let Err(e) = put { return Reply::Error(e.to_string()) } // palette stays open
    if let Delivery::Clip(id) = d { self.store.promote(id, self.clock.now()) }
    self.palette.close(); // selection already set (constraint 5)
    Reply::Copied
}
```

Under anvil, copy_multi with text plus OWN reached a Watcher with OWN in mime_types, and a plain copy lacked it (P:2,4) [M]. The selection cleared when the copier exited (P:3,5) [M], so the daemon, headless or not, keeps serving from W's copy thread (W/src/copy.rs:1043-1050) [S].

The MVP captures nothing at summon. Copy needs no target, because niri falls back from Overlay to the layout's focused window once our surface is gone (NI/src/niri.rs:1378,1394) [S].

Paste v1 adds `lyncast/src/paste/`:

```rust
pub struct Paster(Backend);
pub struct Aim(Target); // opaque: niri window id and app_id, X11 XID, or Blind
pub enum NoPaste { Sandboxed, NoProtocol, CompositorTooOld }
pub enum Missed { FocusMoved, Timeout }
impl Paster {
    pub fn detect() -> Result<Paster, NoPaste> { todo!() }
    pub fn aim(&self) -> Option<Aim> { todo!() }                 // at summon
    pub fn fire(&self, aim: Aim) -> Result<(), Missed> { todo!() } // after close
}
```

`paste/chord.rs` picks Ctrl+V or Ctrl+Shift+V from the app_id, as look does (LK/paste.rs:146-158) [S]. Aim runs at summon because niri reports no focused window while a layer-shell surface has focus (NI/niri-ipc/src/lib.rs:1355) [S]. Target types differ per platform, so they stay out of core and the wire [I].

Red-flag screen: one call per caller, MIME strings only in clipboard.rs, targets only in paste/.

## Tests

Tier 1, no display: `lyncast activate clip:999` prints `error: no clip 999`, exit 1.

Tier 2, Xvfb plus anvil, with a test-only helper on W's paste API (W/src/paste.rs:277):
- Helper copies `hello`, then `world`. `lyncast clip list` prints `2\tworld\n1\thello`.
- `lyncast activate clip:1` prints `copied`. Helper `types` includes `application/x-lyncast-own`, `text` prints `hello`, and `clip list` prints `1\thello\n2\tworld`.
- After `lyncast query 2+3`, `lyncast activate calc` prints `copied` and `clip list` starts with `3\t5`.
- After the daemon stops, helper `text` prints `(empty)`.

Paste v1 tests use anvil's virtual keyboard (/tmp/research-wl/smithay/anvil/src/state.rs:718) [S]. A fixture client prints `hello`. With the fake niri socket naming another window, the CLI prints `copied; not pasted: focus moved`.

## Tradeoffs

- We accept one protocol edit and two daemon edits at paste v1, in exchange for zero paste code now.
- We accept calc answers in history, in exchange for no RecordCalc effect. TinyCast records new text unmarked too (C:224-225) [S].
- We accept a closed backend enum, in exchange for exhaustive matches.
- We accept that `activate` will follow the `enter` setting. Scripts use `clip copy`.

## Alternatives considered

- A `Deliver` port now, with a CopyOnly implementation and `Toggle { target }`. It lost: one pass-through implementation, a target captured and never read, a niri window id on the wire.
- A core `DeliveryMode` with reserved Paste variants and a chord table, like ClipboardDefaultAction (C:9-10) [S]. It lost because reserved variants are dead code and core cannot see the runtime probe.
- Core returns effects `[Write { own }, Promote, Dismiss]`. It lost because paste would edit core, and `own: bool` leaks the marker.

## Open questions and risks

- niri v26.04 (2026-04-25) predates VK rework PR #4548, merged 2026-10-13 (gh) [M]. `detect` should check `Request::Version` (NI/niri-ipc/src/lib.rs:69) [S]. The VK also swaps the seat keymap for every client (NI/src/handlers/virtual_keyboard.rs:153-160) [S].
- niri has no RemoteDesktop portal; #390 is open (gh) [M]. On niri only VK and uinput remain [I].
- Does `aim` slow the summon? Unmeasured.
- If Hongyi wants `clip copy` now, it can alias `activate clip:<id>` until paste v1.
- If calc answers belong outside history, `put_text` gains OWN.

## Design candidate C

# lyncast delivery seam: copy now, paste later

Labels: [M] measured, [S] read-in-source, [I] inferred. NI=/tmp/research-wl/niri, SM=/tmp/research-wl/smithay, W=~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wl-clipboard-rs-0.9.4, PR=/tmp/paste-seam-scratch/probe, TC=/tmp/tinycast-docs.

## Problem

Enter on a clip or calc row copies and closes the palette. Later backends must add paste without touching lyncast-core or the palette. The MVP ships no paste code. I honor grounding constraints 5 to 7: the resident process sets the selection before dismiss and serves it until cancelled, recalled writes carry a private MIME, and lyncast runs unsandboxed. Constraints 2 and 4 become the backend contract. The palette cannot own the selection. GPUI's Wayland clipboard needs a focused window and an input serial (/tmp/research-wl/gpui_wl_client.rs:1465-1471) [S].

## Usage

The palette never says "copy", now or later.
```rust
let reply = self.daemon.handle(Request::Activate { key: self.selected_key(), action });
self.toast(reply);
window.remove_window();
```
The MVP daemon handler matches exhaustively.
```rust
Request::Activate { key, action } => match (self.core.activate(&key)?, action) {
    (Activation::Launch(app), action) => self.niri.spawn(&app, action),
    (Activation::Deliver(d), None) => {
        let copied = self.clipboard.copy(&d)?;
        if let Origin::History(id) = d.origin { self.history.promote(id, self.clock.now())?; }
        Ok(Reply::Copied(copied))
    }
    (Activation::Deliver(_), Some(a)) => Err(NoSuchAction(a)),
},
```
The CLI sends the same request, parsed in `ipc`.
```
$ lyncast clip copy 1    # Activate { key: clip:1, action: None }
copied clip 1 (text/plain;charset=utf-8, 5 bytes)
$ lyncast clip paste 1   # later: action: Some("paste")
```
After the first paste PR, a backend author adds one file and one line.
```rust
// lyncast/src/paste/hyprland.rs
pub fn detect() -> Option<Box<dyn Backend>> { todo!() }
impl Backend for Hyprland {
    fn paste(&mut self, apps: &Catalog) -> Result<Pasted, NotPasted> { todo!() }
}
// lyncast/src/paste/mod.rs
const DETECT: &[fn() -> Option<Box<dyn Backend>>] = &[niri_vk::detect, hyprland::detect];
```

## Shape

```rust
// lyncast-core (pure)
pub struct ClipId(u64);
pub enum Payload { Text(String), Image { mime: ImageMime, blob: BlobHash }, Files(Vec<FileUri>) }
pub enum Origin { History(ClipId), Fresh } // calc answers are Fresh
pub struct Deliverable { pub payload: Payload, pub origin: Origin }
pub enum Activation { Launch(AppId), Deliver(Deliverable), Run(Command) }
impl Core { pub fn activate(&self, key: &Key) -> Result<Activation, NoSuchRow> { todo!() } }

// lyncast::clipboard (wl-clipboard-rs; writer and watcher together)
const OWN_MIME: &str = "application/x-lyncast-own";
pub enum NotCopied { NoCompositor, BlobMissing(BlobHash), Wayland(String) }
impl Clipboard { pub fn copy(&self, d: &Deliverable) -> Result<Copied, NotCopied> { todo!() } }

// lyncast::ipc (wire types, never in core)
pub struct ActionId(String);
pub struct RowView { pub key: Key, pub title: String, pub actions: Vec<ActionView> }
```
`Deliverable` replaces the sketch's `Action::Copy(Payload)`. Core says what reaches the user, never how. `copy` offers `OWN_MIME` only for `Origin::History`, through `copy_multi` on a serving thread (W/src/copy.rs:1034-1060) [S]. The watcher in the same module skips offers carrying `OWN_MIME` or `x-kde-passwordManagerHint` (W/src/utils.rs:21) [S]. Deliver rows send `actions: vec![]`. App rows fill them from .desktop Actions, which the agreed `Activate { key, action }` already implies [I].

The MVP is these types, `Clipboard::copy` and one handler arm. No trait, no reserved variant, no summon capture.

The first paste PR adds `lyncast::paste`: `trait Backend`, `DETECT`, `Pasted`, `NotPasted`, and a pure `chord(&DesktopEntry) -> Chord`. It picks Ctrl+Shift+V for `Terminal=true` or `Categories=TerminalEmulator`, as Vicinae does (/tmp/research-wl/vicinae/src/server/src/services/app-service/xdg/xdg-app.hpp:19-20) [S]. The Deliver arm copies, returns so the palette closes, and runs `backend.paste()` on a worker whose result answers the socket caller. Deliver rows then list `paste`, `copy`. The niri backend reuses the MVP `niri` module, so niri wire types stay there.

Nothing is captured at summon. While our overlay holds focus, niri still reports the layout window in `Workspace.active_window_id` (NI/niri-ipc/src/lib.rs:1479; NI/src/ipc/server.rs:627) [S]. After unmap, focus falls back to that window (NI/src/niri.rs:1378-1394) [S]. So the niri backend reads its target inside `paste()`, before the palette closes [I]. XTest, uinput and the portal need no window id [I].

## Testing

Tests run `lyncast --headless` on temp paths and drive the CLI.

Tier 1, no display: `lyncast clip copy 1` exits 1 with stderr `not copied: no Wayland compositor`.

Tier 2 seeds and reads the clipboard in-process with wl-clipboard-rs. Under Xvfb plus anvil, my marked `copy_multi` read back as "hello lyncast" and the Watcher listed the marker (PR/run.log) [M].
```rust
seed("alpha"); seed("beta");
assert_eq!(cli("clip copy 1"), "copied clip 1 (text/plain;charset=utf-8, 5 bytes)\n");
assert_eq!(read_text(), "alpha");
assert!(read_mimes().contains("application/x-lyncast-own"));
assert_eq!(cli("clip list"), "1\talpha\n2\tbeta\n"); // promoted, no new row
sleep(SECOND);
assert_eq!(read_text(), "alpha"); // still served
assert_eq!(cli("calc copy 2+3*4"), "copied 14\n");
assert_eq!(cli("clip list"), "3\t14\n1\talpha\n2\tbeta\n"); // Fresh is recorded
```
"Still served" has teeth. After my probe's copier exited, reads failed with `ClipboardEmpty` (PR/run.log) [M]. MIME order differed between two runs (PR/watch.log, PR/run.log) [M], so tests check membership.

Paste tests arrive with the first backend. The fake niri socket serves `Workspaces`, `Windows` and an EventStream emitting `WindowFocusChanged { id: Some(7) }`. A test-only sink toplevel on anvil prints each chord and the selection it reads. anvil exposes the virtual keyboard (SM/anvil/src/state.rs:718) and forwards its keys to the focused client (SM/anvil/src/input_handler.rs:1414-1432, 176) [S].
```
lyncast clip paste 1 -> "pasted clip 1 into window 7 (foot) with ctrl+shift+v\n"
sink stdout          -> "ctrl+shift+v alpha\n"
fake niri log        -> Workspaces, Windows, EventStream
```
If the stream emits `Some(9)`, the CLI prints `copied clip 1; not pasted: focus went to window 9` and the sink stays silent.

## Tradeoffs

- We accept that the first paste PR edits one daemon arm and the CLI, in exchange for no speculative MVP code.
- We accept that paste copies first and never restores, in exchange for a free fallback and no restore race. TinyCast keeps the pasted entry too (TC/clipboard.md:487-488) [S].
- We accept calc answers entering history, in exchange for one rule: marked means "already a row". TinyCast leaves Copy Text unmarked likewise (TC/clipboard.md:224-225) [S].
- We accept Xvfb and anvil for delivery tests, in exchange for no fake clipboard in the product.

## Alternatives considered

- A core `enum DeliveryMode { Copy, Paste, PastePlain }` with paste reserved. Two variants would have no MVP path, and core would name mechanisms.
- An MVP `trait Deliver` with a `CopyOnly` impl. One adapter is a hypothetical seam, and `deliver` would pass through to `Clipboard::copy`.
- A post-dismiss hook process, like clipse's auto-paste (/tmp/research-wl/clipse/cmd/root.go:172-173) [S]. Its outcome never returns over the socket, so tests cannot assert it.

## Open questions and risks

- If focus-follows-mouse moves layout focus during the palette (NI/src/niri.rs:6933-6965) [S], the niri backend pastes into the new window. TinyCast targets the summon-time app (TC/palette.md:39-40) [S]. Hongyi should pick one.
- Platforms that cannot read their target under the palette need a summon hook in Toggle [I].
- Released niri may break the keyboard after VK use (niri#2314) [I]. The fix, PR #4548, merged 2026-10-13 [M]. The backend should refuse older niri via `Request::Version` (NI/niri-ipc/src/lib.rs:69) [S].
- Paste outputs are a spec, unrun on niri.

## Judge and synthesis

```json
{
  "scorecard_md": "| Criterion | minimal | domain | testable |\n|---|---|---|---|\n| (a) MVP footprint | 5. Two-variant `Delivery`, one `deliver` fn, no trait, no target, no extra verb. | 3. Adds a lyncast-clip crate, `Held`, `Summon`, a duplicate `ClipCopy` request, `redeliver()` and a payload in core. | 4. No trait and no capture. But `Payload` plus `Origin` plus empty `actions` lists ship now. |\n| (b) Plug-in cost later | 3. Each backend adds a variant to a closed enum and fills arms in mod.rs. Paste v1 also adds `Request::Deliver` and changes what `activate` does. | 4. One file plus one `pick` arm. `Summon` already exists to hold `aim`. Paste v1 still adds `ClipPaste` and a CLI verb. | 4. One file plus one `DETECT` line. But a backend that needs a summon-time target must edit Toggle (its own open question). |\n| (c) Correctness vs grounding | 4. Aims before map, sets the selection before close, gates on Version, notes the keymap swap. Misses that GPUI destroys the wl_surface on close (gpui-pre-linux-0.3.9/src/linux/wayland/window.rs:806-807) [S]. | 4. Covers constraints 1 to 6, including the GPUI leave problem. Omits the sandbox and `modifiers` rules. Cites TinyCast clipboard.md:224-225 for calc, but TinyCast records calc answers in CalculatorHistoryStore (calculator.md:619-620) [S]. | 3. Reads the target at paste time, which focus-follows-mouse can move (niri src/niri.rs:6916-6940) [S]. Closes the window even when the copy fails. |\n| (d) Headless testability | 4. Literal outputs. The daemon-stop check proves ownership. The calc test depends on query state across CLI calls. | 4. Literal shell transcript plus a blob-missing case. Depends on wl-copy and wl-paste, which this host lacks [M]. | 5. In-process seeding. Membership checks, because MIME order varied between probe runs (probe/watch.log:4, probe/run.log:2) [M]. Tier 1 no-compositor literal. Paste spec with a fake niri log. |\n| (e) Interface depth | 4. clipboard.rs hides the MIME strings. The daemon calls store, clipboard and promote separately. | 5. `hand_over` hides the write, marker, serving and promote. `next_capture` hides filtering. `Summon` owns the window's life. | 3. Promote leaks into the handler arm. `paste(&Catalog)` passes the catalog through. `ActionId` is a raw string. |\n| (f) Type discipline | 4. No bool picks the marker. `CalcAnswer` has a private constructor. `Aim` is opaque. Nothing stops `fire` from running before close. | 5. `Held`, `Closed` and `Focused` proof tokens, and `Option<Summon>`. `Fresh(String)` drops the `CalcAnswer` newtype. | 3. `Origin::Fresh` with `Payload::Image` can be built. Actions are checked as strings at runtime. `Box<dyn Backend>`. |\n| **Total (of 30)** | **24** | **25** | **22** |\n\nShared flaw: all three write calc answers unmarked and let the watcher record them. The next `clip list` then races the watcher [I].",
  "base": "domain",
  "grafts": [
    "minimal: `Delivery { Clip(ClipId), Answer(CalcAnswer) }`, with `CalcAnswer` built only by calc::eval. It replaces domain's `Redeliver { clip, payload }` and `Fresh(String)`.",
    "minimal: one `lyncast activate <key>` verb in the MVP, instead of domain's duplicate `ClipCopy` request.",
    "minimal: `NoPaste { Sandboxed, NoProtocol, CompositorTooOld }` and `Missed` for paste v1, gated on `Request::Version` (/tmp/research-wl/niri/niri-ipc/src/lib.rs:69) [S].",
    "minimal: a daemon-stop check that the selection is empty once the daemon exits. It proves the daemon owns the selection. The probe measured clearing on copier exit (/tmp/paste-seam-scratch/probe/run.log:4-8) [M].",
    "testable: `match (core.resolve(key)?, action)` with a `NoSuchAction` arm. Paste verbs then live in the daemon and core never changes.",
    "testable: paste v1 adds `paste` and `copy` actions to deliver rows instead of a new request type.",
    "testable: an in-process wl-clipboard-rs helper for seeding and reading, plus MIME membership checks. MIME order varied between probe runs (/tmp/paste-seam-scratch/probe/watch.log:4, run.log:2) [M].",
    "testable: `fire` runs on a worker, and its result answers the socket caller.",
    "testable: the reason the palette cannot own the selection. GPUI's clipboard needs a focused window and a serial (/tmp/research-wl/gpui_wl_client.rs:1465-1471) [S].",
    "testable: the paste test spec shape, a fake niri socket plus a sink client with literal outputs."
  ],
  "rejected": [
    "testable: capture nothing at summon and read `Workspace.active_window_id` at paste time. Focus-follows-mouse moves layout focus while the palette is open (/tmp/research-wl/niri/src/niri.rs:6916-6940) [S], and TinyCast targets the app that was active at summon (/tmp/tinycast-docs/palette.md:39) [S]. Kept only as a latency fallback.",
    "All three: write calc answers unmarked and let the watcher record them. The next `clip list` races the async watcher [I]. The TinyCast rule they cite (clipboard.md:224-225) covers Copy Text. TinyCast records calc answers in a separate CalculatorHistoryStore (calculator.md:619-620) [S]. Replaced by marked writes plus a synchronous insert.",
    "domain: `Delivery::Redeliver { clip, payload }`. Core would carry bytes the store already owns.",
    "domain: `Request::ClipCopy` and `Core::redeliver` in the MVP. Both duplicate activate.",
    "domain: `Request::Activate { key }` with no action. It drops the .desktop action parameter from the agreed sketch.",
    "domain: a separate lyncast-clip crate. A module gives the same depth without a crate boundary in the MVP [I].",
    "domain: wl-copy and wl-paste as test tools. This host lacks both [M]. The in-process helper uses the crate lyncast already links.",
    "testable: `Deliverable { payload, origin }`. It lets a calc answer carry an image payload.",
    "testable: the palette calls `remove_window` after every reply. A failed copy would close the window.",
    "testable: the `sleep(SECOND)` check that the clip is still served. Reading after the reply, plus the daemon-stop check, proves serving without a sleep.",
    "minimal: `Request::Deliver { clip, verb }` at paste v1. Row actions on Activate need no new request type.",
    "minimal: a closed Backend enum with an arm per method in mod.rs. That spreads each backend over several arms. One detect arm, plus a trait once a second backend exists, costs less.",
    "minimal: calc through `lyncast query 2+3` and then `activate calc`. That depends on daemon state across CLI calls. A `calc:<expr>` key needs no state."
  ],
  "design_md": "# lyncast delivery seam: copy now, paste later\n\nLabels: [M] measured, [S] read-in-source, [I] inferred. NI=/tmp/research-wl/niri, SM=/tmp/paste-seam-scratch/smithay-pin, W=~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wl-clipboard-rs-0.9.4/src, GP=~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-pre-linux-0.3.9/src/linux/wayland, PR=/tmp/paste-seam-scratch/probe, TC=/tmp/tinycast-docs.\n\n## Problem\n\nEnter on a clip or calc row copies and closes the palette. Paste lands later without edits to lyncast-core or the palette. The MVP ships no paste code. It honors constraints 5 (selection set before close, then served) and 6 (own writes marked). The daemon owns the selection. The palette cannot: GPUI's Wayland clipboard needs a focused window and an input serial (/tmp/research-wl/gpui_wl_client.rs:1465-1471) [S]. A copier's exit clears the selection (PR/run.log:4-8) [M].\n\n## Usage\n\nThe palette forwards Enter. The daemon closes the window.\n```rust\nfn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {\n    let req = Request::Activate { key: self.selected_key(), action: None };\n    if let Reply::Failed(why) = self.daemon.handle(req) {\n        self.status = Some(why); // window stays, row kept\n        cx.notify();\n    }\n}\n```\nThe CLI sends the same request. Keys are `app:<id>`, `clip:<id>` and `calc:<expr>`.\n```\n$ lyncast activate clip:1\ncopied clip 1\n$ lyncast activate 'calc:2+3*4'\ncopied 14\n```\n\n## Shape\n\nlyncast-core (pure) says what to hand over, never how.\n```rust\npub struct ClipId(i64);\npub struct CalcAnswer(String); // built only by calc::eval\npub enum Delivery { Clip(ClipId), Answer(CalcAnswer) }\npub enum Action { Launch(AppId), Deliver(Delivery), Run(Command) }\nimpl Core { pub fn resolve(&self, key: &Key) -> Result<Action, NoSuchRow> { todo!() } }\n```\n`Deliver(Delivery)` replaces the sketch's `Copy(Payload)`. The store holds the bytes, so core names only the row. Row actions stay in the daemon, so paste verbs never reach core.\n\n`lyncast::clip` owns wl-clipboard-rs, rusqlite and the blob files. It is the only module that knows MIME strings.\n```rust\nconst OWN: &str = \"application/x-lyncast-own\";\npub struct Held(()); // the selection is ours and served\npub enum NotCopied { NoCompositor, BlobMissing(ClipId), Wayland(String) }\nimpl Clips {\n    pub fn deliver(&mut self, d: &Delivery, at: Timestamp) -> Result<Held, NotCopied> { todo!() }\n    /// Skips OWN and x-kde-passwordManagerHint.\n    pub fn next_capture(&mut self) -> Result<Capture, WatchError> { todo!() }\n}\n```\nEvery lyncast write carries OWN. `deliver` calls `copy_multi` (W/copy.rs:491), which serves from a thread (W/copy.rs:1043-1058) [S]. Then it records the write itself: a Clip promotes, an Answer inserts a row. Marked means \"already recorded\". The Watcher lists MIME types (W/watch.rs:196-197), so the skip is a string compare (W/utils.rs:21) [S]. Under anvil, a marked copy reached the Watcher with OWN and a plain copy did not (PR/watch.log:2,4) [M]. Without a compositor, `Clips` opens unwatched and `deliver` returns `NoCompositor`.\n\nThe daemon owns one summon.\n```rust\nstruct Summon { palette: ui::Palette } // Option<Summon> is the toggle state\nimpl Summon {\n    fn end(self, _: Held) { self.palette.close() } // needs a held selection\n    fn cancel(self) { self.palette.close() }       // Escape, second toggle\n}\nRequest::Activate { key, action } => match (self.core.resolve(&key)?, action) {\n    (Action::Launch(app), action) => self.niri.spawn(&app, action),\n    (Action::Deliver(d), None) => {\n        let held = self.clips.deliver(&d, self.clock.now())?; // Err keeps the summon\n        if let Some(s) = self.summon.take() { s.end(held) }\n        Ok(Reply::Copied(d.to_string()))\n    }\n    (Action::Run(cmd), None) => self.run(cmd),\n    (_, Some(a)) => Err(NoSuchAction(a)),\n},\n```\n`RowView.enter` labels Enter. The MVP sends \"Copy\" on clip and calc rows.\n\nCopy needs no target. Without an exclusive Overlay surface, niri refocuses the layout's window (NI/src/niri.rs:1378-1394) [S].\n\n**The MVP code, complete:** `Delivery` and `CalcAnswer` in core. `Clips::deliver`, `Held`, `NotCopied` and the OWN skip. `Summon` with `end` and `cancel`. One Activate arm. `RowView.enter`. No trait, no reserved variant, no summon capture.\n\n**Paste v1 adds, in the daemon crate only:**\n- `paste/mod.rs`: `Paster`, a struct until a second backend makes it a trait. Opaque `Aim`. `NoPaste { Sandboxed, NoProtocol, CompositorTooOld }`. `Missed { FocusMoved(u64), Timeout }`.\n- `paste/niri_vk.rs` and a pure `paste/chord.rs`.\n- `Summon.aim`, set in toggle before the window opens, because FocusedWindow reports nothing once a layer surface has focus (NI/niri-ipc/src/lib.rs:1355) [S]. `end` returns a `Closed` that only `end` can build.\n- Deliver rows accept `paste` and `copy` actions. Enter follows an `enter = \"copy\" | \"paste\"` setting. No new request type.\n- `Reply::Pasted` and `Reply::CopiedNotPasted(Missed)`. `fire(Closed, Aim)` runs on a worker, and its result answers the socket caller.\n\nniri_vk rules:\n- `detect` gates on `Request::Version` (NI/niri-ipc/src/lib.rs:69) [S]. No release has VK rework PR #4548 (merged 2026-10-13; v26.04 is 2026-04-25) [M].\n- On niri, a missing VK global means sandboxed (NI/src/niri.rs:2575-2576) [S].\n- `fire` waits for `WindowFocusChanged { id: Some(aim) }` (NI/niri-ipc/src/lib.rs:1673-1676) [S]. `wl_keyboard.leave` is no signal. GPUI destroys the wl_surface on close (GP/window.rs:806-807), and smithay's destroy hook sends leave before niri refocuses (SM/src/wayland/seat/keyboard.rs:221-233) [S].\n- It uploads its own keymap so Ctrl+V resolves on any layout [I]. That swaps the seat keymap (NI/src/handlers/virtual_keyboard.rs:187-188) [S].\n- It sends explicit presses and releases, never `modifiers`, whose state outlives the device (NI/src/handlers/virtual_keyboard.rs:98-108) [S].\n- `chord.rs` picks Ctrl+Shift+V for terminals by app_id stem, as look does (/tmp/paste-seam-scratch/src/look/apps/linows/backend/src/paste.rs:145-158) [S].\n\nA later backend adds one file in `paste/` and one arm in `paste::detect`'s exhaustive match on `Compositor`.\n\n## Testing\n\nTier 1, no display, `lyncast --headless` on temp paths:\n- `lyncast activate clip:9` exits 1 with stderr `lyncast: no clip 9`.\n- `lyncast activate calc:2+3` exits 1 with stderr `lyncast: not copied: no Wayland compositor`.\n\nTier 2, Xvfb plus anvil. An in-process wl-clipboard-rs helper seeds and reads, since wl-copy is absent here [M]. MIME order differed between probe runs (PR/watch.log:4; PR/run.log:2) [M], so tests check membership.\n```rust\nseed(\"alpha\"); seed(\"beta\"); wait_rows(2);\nassert_eq!(cli(\"activate clip:1\"), \"copied clip 1\\n\");\nassert_eq!(read_text(), \"alpha\");\nassert!(read_mimes().contains(OWN));\nassert_eq!(cli(\"clip list\"), \"1\\talpha\\n2\\tbeta\\n\"); // promoted, no new row\nassert_eq!(cli(\"activate calc:2+3*4\"), \"copied 14\\n\");\nassert_eq!(cli(\"clip list\"), \"3\\t14\\n1\\talpha\\n2\\tbeta\\n\"); // one row, no race\nstop_daemon();\nassert_eq!(read_text_or_empty(), \"\"); // the daemon was the owner\n```\nA second test deletes a seeded image's blob. `lyncast activate clip:1` exits 1 with stderr `lyncast: clip 1: blob missing`, and the selection still lacks OWN. Tier 3 has no compositor, so Enter keeps the window open and shows `not copied: no Wayland compositor`.\n\nPaste v1 spec, unrun. anvil has the virtual keyboard (/tmp/research-wl/smithay/anvil/src/state.rs:718) [S]. A fake niri socket answers FocusedWindow with window 7 (foot) and streams `WindowFocusChanged { id: Some(7) }`. `lyncast activate clip:1 --action paste` prints `pasted clip 1 into window 7 with ctrl+shift+v`, and a sink client prints `ctrl+shift+v alpha`. Streaming `Some(9)` prints `copied clip 1; not pasted: focus went to window 9`, and the sink stays silent.\n\n## Tradeoffs\n\n- We accept that paste v1 edits toggle and the Activate arm, in exchange for no paste code now.\n- We accept calc answers in clip history. TinyCast records answers too, in a separate CalculatorHistoryStore (TC/calculator.md:619-620) [S].\n- We accept the `Held` token, in exchange for close-before-write failing to compile.\n- We accept no restore after paste, as TinyCast does (TC/clipboard.md:487-488) [S].\n\n## Alternatives considered\n\n- A core `DeliveryMode` with reserved Paste variants: dead code, and core cannot see the probe.\n- An MVP `Deliver` trait with one CopyOnly pass-through.\n- Unmarked calc answers recorded by the watcher. The next `clip list` races the watcher [I].\n- Reading the target at paste time from `Workspace.active_window_id` (NI/niri-ipc/src/lib.rs:1479) [S]. Focus-follows-mouse moves layout focus under the palette (NI/src/niri.rs:6916-6940) [S], so paste could hit a window the user never chose.\n\n## Open questions and risks\n\n- Does summon-time FocusedWindow slow the summon? Unmeasured. Fallback: read `active_window_id` after mapping (NI/src/ipc/server.rs:627) [S].\n- Does `WindowFocusChanged` follow a destroyed layer surface on live niri? Unrun: libinput, libseat and gbm are missing here [M].\n- Should calc answers stay out of clip history? Then Answer skips the insert.\n- If the chord rule needs `Categories=TerminalEmulator` (/tmp/research-wl/vicinae/src/server/src/services/app-service/xdg/xdg-app.hpp:20) [S], paste edits core's DesktopEntry once.\n- Hongyi needs a niri main build. fcitx users also need fc9e8c4 (2026-10-14) [M].\n\n## Synthesis decision\n\nBase: domain (Summon, Held, deep Clips, aim before map, IPC focus wait). Grafts: minimal's Delivery enum, one verb and daemon-stop test; testable's (resolve, action) match, helper and worker reply. New: marked writes recorded synchronously, and calc keys carry the expression.\n"
}
```

## Review: design quality

```json
{
  "verdict": "The MVP is not over-built. It has no trait, no reserved variant and no summon capture. The only speculative pieces are the Held token and a Summon wrapper whose two methods both just call palette.close(). The real problems are in how the code is split, and in the seam promising more than it delivers. First, the design records history in two places that never sync: sqlite is updated on every own write, but lyncast-core, which resolves keys and ranks rows, never sees those writes. Second, one Clips object holds both a watcher that blocks forever and deliver, behind the same &mut self. Third, the palette closes itself by calling into the daemon, and GPUI's re-entrancy rules reject that (read-in-source). On the future seam: paste v1 does edit the palette (the reply becomes deferred, and copy needs a second key binding), and it may edit core. The X11 XTest backend is not one file. Aim has to be an event stream opened before the window maps, not just a window id, or fire can miss the focus event it waits for. The tests check only error paths around closing, so every tier would still pass if Summon::end did nothing. Fixes that keep the MVP small: split Capture out of Clips. Have deliver return what it recorded so the daemon can apply it to core. Let the palette close its own window and clear summon from on_window_closed. Drop Held. Either forward chords (Enter, Mod+Enter) to the daemon or admit that paste v1 edits the palette, and give handle a reply sink from day one.",
  "issues": [
    {
      "title": "Own writes bypass core, so core and the store disagree after every Enter",
      "severity": "major",
      "scenario": "This uses the design's own Tier 2 sequence. Seed alpha and beta, run `activate clip:1`, then `activate calc:2+3*4`. `clip list` shows `3\\t14` first, which the test asserts. `lyncast activate clip:3` then prints `lyncast: no clip 3`, because Core::resolve never learned about row 3. The palette also keeps alpha at its old rank, because the promote went only to sqlite.",
      "evidence": "Core must hold the set of clip ids. Tier 1 expects `no clip 9` while deliver would fail first with NoCompositor, and NotCopied has no missing-row variant, so only Core::resolve can produce that message (inferred from design text). The Activate arm calls clips.deliver and returns without updating core (design text). Every lyncast write carries OWN and next_capture skips OWN (design text), so the watcher never feeds these rows to core either. The watcher lists MIME types, which is what the skip checks (W/watch.rs:193-198, read-in-source).",
      "fix": "Make deliver return what it recorded, for example `enum Recorded { Promoted(ClipId, Timestamp), Inserted(ClipRow) }` carried in Held, and apply it to core in the same arm. In the Tier 2 test, add `activate clip:3` after the calc line."
    },
    {
      "title": "Clips puts a forever-blocking watcher and deliver behind one &mut self",
      "severity": "major",
      "scenario": "next_capture blocks until some selection changes, so it needs its own thread. If that thread holds Mutex<Clips> while blocked in next_capture, pressing Enter blocks the GPUI main thread on the lock inside confirm. The UI freezes until some other app copies something. If Clips lives on the GPUI thread instead, next_capture freezes the UI directly.",
      "evidence": "Watcher::next_event(&mut self) blocks in poll with no timeout (W/watch.rs:308-315, 349-373, read-in-source). The Offer it returns borrows the Watcher (W/watch.rs:331-340, 390, read-in-source), so reading the data also needs the &mut. CancelHandle writes a byte that nothing ever drains (W/watch.rs:148-150, 283-291, 358, read-in-source), so cancel is one-shot and cannot pause the watcher for a deliver (inferred). The design puts both `deliver(&mut self)` and `next_capture(&mut self)` on one Clips (design text).",
      "fix": "Split Clips in two. A Capture type owns the Watcher on its own thread, applies the OWN and x-kde-passwordManagerHint skips, and sends Capture values over a channel. Clips keeps the store, the blobs and deliver on the daemon thread. The daemon loop selects on that channel and the socket."
    },
    {
      "title": "The palette closes itself by calling into the daemon, and GPUI rejects that",
      "severity": "major",
      "scenario": "Hongyi presses Enter on a clip. confirm runs inside the palette window's update and calls daemon.handle. deliver succeeds, summon.take() runs, and end calls palette.close(). If close goes through the window handle, GPUI returns `window not found` and the window stays open while summon is now None, so the next Mod+Space opens a second overlay. If close goes through the Palette entity, GPUI panics with `cannot update Palette while it is already being updated`. Escape hits the same path through cancel.",
      "evidence": "While a window is updating, GPUI takes it out of cx.windows, and a nested update of the same id gets None (gpui-pre-0.3.9/src/app.rs:3140-3149, 2201-2215, read-in-source). An entity under update is leased, and a second update panics (gpui-pre-0.3.9/src/app/entity_map.rs:160-165, 230-233, read-in-source). The design's confirm calls self.daemon.handle, and the arm calls s.end(held), which calls self.palette.close() (design text). end and cancel take no App context, and closing a GPUI window needs one (inferred).",
      "fix": "Have the palette close its own window with window.remove_window() (gpui-pre-0.3.9/src/window.rs:2384) on a Copied reply or on Escape. The daemon clears summon from cx.on_window_closed (gpui-pre-0.3.9/src/app.rs:2751). The same hook also covers closes that start in the compositor."
    },
    {
      "title": "No test checks that Enter copies and closes",
      "severity": "major",
      "scenario": "Every test in the design passes with the re-entrant close bug above still present. Tier 1 asserts two error strings. Tier 3 asserts only the no-compositor failure. Tier 2 drives the CLI with no palette open, so summon is None and end never runs. Replace the body of Summon::end with nothing and all three tiers still pass.",
      "evidence": "Testing section (design text). Tier 1's calc test passes for any calc result, because the failure message never prints the answer (design text). The MVP's main user-visible behavior is 'Enter re-copies the entry and closes the palette' (task brief).",
      "fix": "Add a Tier 2 case: `lyncast toggle`, then `lyncast activate clip:1`, then a status query prints `closed`, and a second toggle opens exactly one window. Add a Tier 3 case using an in-memory selection constructor on Clips (a constructor, not a trait) and assert that Enter closes the window."
    },
    {
      "title": "Paste v1 edits the palette, though the design says it will not",
      "severity": "major",
      "scenario": "Hongyi sets enter = \"paste\". The palette has one binding, Confirm, which sends action None, so Copy can no longer be reached from the palette. TinyCast's swap, with Copy on Cmd+Enter when Paste owns Enter, needs a second binding that sends Some(\"copy\"). Separately, fire answers the socket caller later from a worker, so daemon.handle can no longer return a Reply synchronously. The palette line `if let Reply::Failed(why) = self.daemon.handle(req)` then stops compiling.",
      "evidence": "The Problem section says 'Paste lands later without edits to lyncast-core or the palette' (design text). The design's own paste v1 list touches toggle, Summon::end, the Activate arm, Reply, the enter setting and RowView (design text). Not listed: the socket server's deferred reply, the palette call site, a palette binding for the second verb, the CLI printer for Pasted and CopiedNotPasted, and Cargo.toml for the virtual-keyboard protocol and keymap code (all inferred). Core's DesktopEntry may also change, as the Open questions admit. TinyCast keeps one function that maps a chord to an action (/tmp/tinycast-docs/clipboard.md:9-12, read-in-source). Count: 3 new files and about 10 edited places, 2 of them in the palette (inferred).",
      "fix": "Pick one. Either state plainly that paste v1 edits the palette twice and maybe core once. Or have the MVP forward a chord (Enter, Mod+Enter) instead of an action string, with one daemon function mapping (row kind, chord, setting) to a verb, and give handle a reply sink from day one so a deferred answer needs no change at the call site."
    },
    {
      "title": "fire subscribes too late to see the focus event it waits for",
      "severity": "major",
      "scenario": "Enter with paste. end closes the window, and a worker starts fire, which opens the niri EventStream. If niri refocuses window 7 before the worker's stream connects, the new stream gets only the initial WindowsChanged burst with is_focused set on 7. No WindowFocusChanged follows. fire hits Timeout and reports `not pasted` even though focus is correct.",
      "evidence": "niri sends a new stream its initial state from replicate() (/tmp/research-wl/niri/src/ipc/server.rs:228-255, read-in-source). For windows, replicate() yields only WindowsChanged (/tmp/research-wl/niri/niri-ipc/src/state.rs:167-171, read-in-source). fire takes Closed, which only end builds, so it starts after the close (design text). GPUI sends the destroy requests when it drops the platform window (GP/window.rs:768-807, read-in-source). They are flushed on a later loop pass, so their timing relative to the worker is outside lyncast's control (inferred). Which side wins on live niri is unmeasured.",
      "fix": "Open the EventStream in toggle, before the window maps, and make that open stream part of Aim. Treat either WindowFocusChanged { id: Some(aim) } or a WindowsChanged entry with is_focused for aim as the signal."
    },
    {
      "title": "Held guards one close path while cancel and Launch go around it",
      "severity": "minor",
      "scenario": "Hongyi types `fire` and presses Enter. The Launch arm calls niri.spawn and never touches summon. The overlay stays mapped with exclusive keyboard, so Firefox gets no keys until Escape. The only way that arm could close the palette is cancel, which needs no Held. `s.cancel(); self.clips.deliver(..)` also compiles, so the claim that closing before writing fails to compile is false.",
      "evidence": "cancel(self) closes with no token, and the Launch and Run arms close nothing (design text). For copy-only, the order does not affect the selection: ext-data-control SetSelection checks no focus and no serial (/tmp/paste-seam-scratch/smithay-pin/src/wayland/selection/ext_data_control/device.rs:47-72, read-in-source). The one ordering the MVP needs, keeping the window open on Err, already comes from the `?` before summon.take() (design text).",
      "fix": "Drop Held from the MVP. Give Summon a single close(self) -> Closed, used by Launch, Run, Deliver and Escape. If paste v1 wants proof of ordering, make fire take both Held and Closed."
    },
    {
      "title": "The paste v1 types do not allow paste through the CLI",
      "severity": "minor",
      "scenario": "The paste v1 spec test runs `lyncast activate clip:1 --action paste` without a toggle. No Summon exists, so there is no Summon.aim and nothing can build Closed, and `pasted clip 1 into window 7 with ctrl+shift+v` cannot be produced. If the test toggles first, Tier 2 has to run GPUI under anvil, which the MVP Tier 2 never does. The sink also needs anvil keyboard focus, and anvil gives that only on a pointer click, or to an exclusive layer surface when a key event arrives.",
      "evidence": "Only end builds Closed, and Aim is set in toggle (design text). The agreed MVP says every UI action also exists as a CLI command (task brief). anvil's new_toplevel never sets keyboard focus (/tmp/research-wl/smithay/anvil/src/shell/xdg.rs:46-56). Exclusive layers get focus only inside keyboard_key_to_action (/tmp/research-wl/smithay/anvil/src/input_handler.rs:136-160). layer_destroyed unmaps the layer without refocusing anything (/tmp/research-wl/smithay/anvil/src/shell/mod.rs:265-276). All three are read-in-source.",
      "fix": "When no summon is open, take Aim from FocusedWindow at activate time, and give Closed a constructor for 'nothing was open'. Write the spec test with an explicit toggle step or with the no-summon path, and say how the sink gets anvil focus, for example a layer-shell sink with exclusive keyboard."
    },
    {
      "title": "Missed and Reply cannot represent two outcomes paste v1 will hit",
      "severity": "minor",
      "scenario": "After dismiss, focus goes to another layer surface or to an empty workspace, and niri emits WindowFocusChanged { id: None }. Missed::FocusMoved(u64) cannot hold that, so fire waits for Timeout and the user waits for nothing. Separately, a sandboxed `lyncast activate clip:1 --action paste` gets NoPaste::Sandboxed, but Reply has only Pasted and CopiedNotPasted(Missed). The copy succeeded, and no variant says why the paste did not run.",
      "evidence": "In niri-ipc, id is Option<u64> and is None when no window is focused (/tmp/research-wl/niri/niri-ipc/src/lib.rs:1673-1676, read-in-source). Missed { FocusMoved(u64), Timeout } and Reply::CopiedNotPasted(Missed) (design text).",
      "fix": "Use FocusMoved(Option<u64>). Make the reply CopiedNotPasted(NotPasted), where NotPasted is either Missed(Missed) or Unsupported(NoPaste)."
    },
    {
      "title": "CompositorTooOld cannot be computed from Request::Version",
      "severity": "minor",
      "scenario": "Hongyi runs niri main at 4263c9a, which includes #4548. The broken v26.04 release and this build both answer Version with `26.04 (...)`. detect must either refuse Hongyi's build or accept the broken release.",
      "evidence": "niri formats its version as MAJOR.MINOR plus a commit string (/tmp/research-wl/niri/src/utils/mod.rs:144-160, read-in-source). 4263c9a is 26.4.0 (/tmp/research-wl/niri/Cargo.toml:9, read-in-source), and main is still 26.4.0 (gh api repos/niri-wm/niri/contents/Cargo.toml, measured). v26.04 from 2026-04-25 is the latest release, and #4548 merged on 2026-10-13 (gh release list; gh pr view 4548, measured). A client cannot check commit ancestry from that string (inferred).",
      "fix": "Gate niri_vk on an explicit config opt-in until niri ships a release after 26.04. After that, gate on version >= that release."
    },
    {
      "title": "'One file per backend' holds only for backends on niri",
      "severity": "minor",
      "scenario": "Adding XTest on X11 changes five places outside paste/, including the UI. Clips uses only wl-clipboard-rs. toggle gets Aim from niri FocusedWindow. The Activate arm calls self.niri.spawn. chord.rs keys on the Wayland app_id. The palette uses WindowKind::LayerShell. The likely second backend is uinput on niri, because released niri has the broken virtual keyboard. match Compositor has a single niri arm, so that arm gets edited to 'VK if the probe passes, else uinput' rather than a new arm being added.",
      "evidence": "WindowKind::LayerShell exists only under cfg(wayland) (gpui-pre-0.3.9/src/platform.rs:2725-2728). The X11 window code handles only PopUp, Floating and Dialog (gpui-pre-linux-0.3.9/src/linux/x11/window.rs:514, 616-658). look tries mechanisms in order within one platform: the virtual keyboard, then the GNOME extension (/tmp/paste-seam-scratch/src/look/apps/linows/backend/src/platform/linux/keysynth.rs:21-34). All three are read-in-source. The Activate arm and the Clips shape come from the design text.",
      "fix": "Have detect walk an ordered list of mechanisms, each with probe() -> Result<(), NoPaste>. Rewrite the claim to say that an XTest backend depends on porting clip, toggle, launch and the palette window to X11."
    },
    {
      "title": "No-compositor tests depend on the developer's environment",
      "severity": "minor",
      "scenario": "Hongyi runs cargo test in a terminal on niri. WAYLAND_DISPLAY is inherited, so Clips connects to the real niri. `lyncast activate calc:2+3` prints `copied 5` and exits 0, and the Tier 1 test fails. The run also overwrites Hongyi's real clipboard with 5, and the watcher copies their live clipboard into the temp store. Tier 3 has the same problem.",
      "evidence": "wayland-client reads WAYLAND_DISPLAY and XDG_RUNTIME_DIR and has no default socket (~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wayland-client-0.31.11/src/conn.rs:69-87, read-in-source). wl-clipboard-rs calls connect_to_env (W/common.rs:87, read-in-source). The design says only 'on temp paths' (design text).",
      "fix": "Have the Tier 1 and Tier 3 harness remove WAYLAND_DISPLAY and WAYLAND_SOCKET and point XDG_RUNTIME_DIR at a temp directory. Add one guard test that asserts this."
    },
    {
      "title": "Tier 2 seeding races the watcher's read",
      "severity": "minor",
      "scenario": "seed(\"alpha\") and seed(\"beta\") run back to back. Beta's set_selection cancels alpha's source, and wl-clipboard-rs destroys it immediately. If the watcher has not read alpha's offer yet, its receive gets EOF. Row 1 is then empty or missing, and either wait_rows(2) hangs or `clip list` differs from the assertion.",
      "evidence": "wl-clipboard-rs destroys the source on Cancelled (W/copy.rs:361, read-in-source). smithay forwards receive to the client's source (/tmp/paste-seam-scratch/smithay-pin/src/wayland/selection/offer.rs:177-183, read-in-source). Each probe copier slept 1.5 s before exiting (/tmp/paste-seam-scratch/probe/src/main.rs:29, read-in-source), and the probe's watch.log shows CLEARED between copies, so the probe never seeded back to back (measured). That the read ends in EOF is inferred.",
      "fix": "Wait for each row before seeding the next: seed(\"alpha\"); wait_rows(1); seed(\"beta\"); wait_rows(2)."
    },
    {
      "title": "Option<Summon> goes stale when niri closes the overlay",
      "severity": "minor",
      "scenario": "The palette is open on a laptop panel, and Hongyi unplugs or disables that output. niri sends closed to the layer surface, and GPUI closes the window by itself. summon stays Some. The next Mod+Space runs cancel on a dead window and shows nothing, so Hongyi has to press it twice.",
      "evidence": "niri sends closed to every layer surface on an output being removed (/tmp/research-wl/niri/src/niri.rs:3175-3177), and to a new layer surface that has no output (/tmp/research-wl/niri/src/handlers/layer_shell.rs:27-35). GPUI turns that event into a close (gpui-pre-linux-0.3.9/src/linux/wayland/window.rs:1359-1362; client.rs:1886-1890). All read-in-source. The design makes Option<Summon> the toggle state (design text).",
      "fix": "Use the same fix as for the re-entrant close: clear summon from cx.on_window_closed, so the toggle state follows the real window."
    }
  ]
}
```

## Review: platform reality

```json
{
  "verdict": "The copy MVP holds up on niri. The daemon owning the selection, recording marked writes synchronously, and relying on niri's layout refocus all match the source: niri.rs:1378-1394 falls back to the layout window, GPUI destroys the wl_surface on close (gpui-pre-linux window.rs:807), and the probe logs confirm the claimed watcher behavior. Three things still break the MVP in practice. First, the OWN marker is an empty payload offered in random order. In 5 of 12 measured runs, a first-type reader of a re-copied image got 0 bytes. Second, the Launch and Run arms never end the summon, so the overlay keeps exclusive keyboard focus after an app launch. Third, no test tier covers the success path (Enter, close, still serving). Two GPUI facts make that path fragile: a window is leased while its own handler runs, and QuitMode::Default quits on Linux when the last window closes, which would clear the selection. The paste v1 spec has three race holes. The focus wait listens for a WindowFocusChanged event that a late subscriber never gets. Chord modifiers the user is still holding merge with the virtual keyboard's Ctrl+V, and the virtual keyboard cannot release them. A fire that is still waiting is not cancelled by a newer summon and does not check that the selection is still its own. The remaining items are minor: version gating, keymap churn, terminal detection, test environment hygiene, and an overstated claim that other platforms need only a new paste/ file. All have small fixes. Scratch evidence is in /tmp/paste-seam-scratch/order-probe/ (main.rs, order.log, wl-paste.c).",
  "issues": [
    {
      "title": "Empty OWN marker can be the type a reader picks, so re-copied images paste as 0 bytes",
      "severity": "major",
      "scenario": "Hongyi re-copies an image row. lyncast offers image/png plus an empty application/x-lyncast-own. A reader that picks the first offered type gets 0 bytes. That includes cliphist's `wl-paste --watch cliphist store`, `wl-paste > shot.png`, and wl-clipboard-rs MimeType::Any. Text rows are safe because every picker prefers text types.",
      "evidence": "Measured: /tmp/paste-seam-scratch/order-probe/order.log. 12 fresh copies of image/png+OWN under Xvfb+anvil, each read with MimeType::Any. 5 returned mime=application/x-lyncast-own bytes=0; 7 returned image/png bytes=19. Read-in-source: wl-clipboard-rs-0.9.4 copy.rs:264 stores the sources in a HashMap, and copy.rs:938-947 forces only the password hint to the end. paste.rs:435-440: Any takes the first non-hint type when no text type is offered. wl-paste.c:160-166 (bugaevc/wl-clipboard, fetched with gh api) falls through to try_any, the first offered type. Measured: OWN was first in both probe logs (PR/run.log:2, PR/watch.log:4).",
      "fix": "Give OWN the same bytes as the primary payload, so a first-type reader still gets real data. The watcher only checks membership, so nothing else changes. Add a Tier 2 case: re-copy an image row, read it with MimeType::Any, assert a non-empty PNG."
    },
    {
      "title": "Launch and Run arms never end the summon, so the overlay keeps keyboard focus after a launch",
      "severity": "major",
      "scenario": "Hongyi types 'fire' and presses Enter. The Activate arm runs self.niri.spawn and returns, but the summon stays. Firefox maps behind the overlay and becomes the layout focus. niri still gives the keyboard to the mapped exclusive Overlay surface. Hongyi's typing goes into the palette until they press Escape. `lyncast activate app:...` from the CLI with a palette open does the same.",
      "evidence": "Read-in-source (design): `(Action::Launch(app), action) => self.niri.spawn(&app, action)` and the Run arm never call self.summon.take(). Only the Deliver arm calls end. Read-in-source: niri src/niri.rs:1378 checks Overlay exclusive focus before layout_focus at :1394, and excl_focus_on_layer (niri.rs:1336-1351) counts any mapped Exclusive surface.",
      "fix": "After a successful spawn or run, take the summon and close it. Either give end a non-Held variant, or make cancel public to those arms. Add a Tier 2 or Tier 3 assertion that the window is gone after activating an app row."
    },
    {
      "title": "No tier tests Enter, close, still serving, and two GPUI facts can break that path",
      "severity": "major",
      "scenario": "Case A: confirm runs inside the palette window's dispatch. It calls daemon.handle, which calls Summon::end, which calls palette.close(). If close() goes through a WindowHandle update, GPUI returns 'window not found' because the window is leased, and the window stays open. Case B: close works but the app was built without QuitMode::Explicit. GPUI quits when the last window closes, the process exits, and the serving thread dies, so the copy is cleared at once. Tier 2 never summons, and Tier 3 only covers the no-compositor failure, so neither case shows up in CI.",
      "evidence": "Read-in-source: gpui-pre-0.3.9 app.rs:3146 takes the window out of its slot during an update, and app.rs:2215 returns 'window not found' for a nested update. app.rs:3180-3187: QuitMode::Default quits when windows.is_empty() on non-macOS. examples/toggle_headless.rs:256 and zlaunch daemon/mod.rs:61 both set QuitMode::Explicit. The design never mentions it. Measured: PR/run.log:4-8 shows the selection empties when the copier exits. Inferred: the close-through-a-handle path, since Summon::end takes no cx.",
      "fix": "Set QuitMode::Explicit when building the app. Have the daemon defer the close (cx.defer, or have the palette call window.remove_window() after a successful reply) instead of updating a leased window. Add a Tier 2 test under anvil: summon, press Enter on a clip row, assert the window is closed, then assert read_text() still returns the entry."
    },
    {
      "title": "Paste v1 focus wait can miss focus returning, so every paste times out",
      "severity": "major",
      "scenario": "fire(Closed, Aim) runs on a worker after close and opens the EventStream. niri may have already refocused window 7 by then. That takes one loop pass, while connecting and subscribing takes about as long. A new subscriber gets a WindowsChanged burst with is_focused=true on window 7, never a WindowFocusChanged. fire waits until Timeout and reports CopiedNotPasted(Timeout) even though focus is correct. The fake niri socket in the spec streams WindowFocusChanged{Some(7)} directly, so the test passes while real niri fails.",
      "evidence": "Read-in-source: niri src/ipc/server.rs:247-255 sends state.replicate() as the first burst. niri-ipc/src/state.rs:97-106 shows the burst is WorkspacesChanged, WindowsChanged, KeyboardLayoutsChanged and so on, with no WindowFocusChanged. src/ipc/server.rs:739-741 emits WindowFocusChanged{Some} only when a window goes from unfocused to focused. Inferred: the subscribe-after-close timing, from the design's 'fire(Closed, Aim) runs on a worker'.",
      "fix": "Open the EventStream in toggle, next to the FocusedWindow query and before the overlay maps. Fold every event into niri_ipc::state::EventStreamState. After close, decide from state: windows[aim].is_focused. Make the fake niri send the real initial burst before any delta events."
    },
    {
      "title": "Chord modifiers the user is still holding merge with the virtual Ctrl+V, and the virtual keyboard cannot release them",
      "severity": "major",
      "scenario": "Enter is set to copy and paste is on Shift+Enter or Alt+Enter, or the reverse. The worker fires a few ms after the keydown, while Shift or Alt is still held. The target sees Ctrl+Shift+V, or Ctrl+Alt+V. In LibreOffice Ctrl+Shift+V opens Paste Special, and in many apps Ctrl+Alt+V does nothing. Sending a Shift release from the virtual keyboard does not help: smithay drops it because the physical source still holds the key.",
      "evidence": "Read-in-source: smithay-pin src/input/keyboard/mod.rs:313-341 tracks each keycode per source. A press of a key another source holds is dropped (:321), and so is a release while another source still holds it (:332). :884-888 re-applies every held keycode, physical included, to the new xkb state after a keymap swap. niri src/input/mod.rs:433-441 says shortcuts can be pressed across keyboards on purpose. Inferred: how LibreOffice handles Ctrl+Shift+V and Ctrl+Alt+V.",
      "fix": "Trigger paste on release. The palette waits until GPUI shows no modifiers and Enter released, and only then sends Activate with action paste and closes. Use standard evdev keycodes for Control_L, Shift_L and v in lyncast's keymap so they never land on a held physical key."
    },
    {
      "title": "A waiting fire is never cancelled and never checks the selection is still its own",
      "severity": "major",
      "scenario": "Paste of clip A hits the Timeout path, for example because focus went to a layer surface or an empty workspace. While fire waits, Hongyi re-summons and copies clip B, then focuses window 7. Worker A sees Some(7), sends Ctrl+V, pastes B into window 7, and reports 'pasted clip 1'. The previous issue makes the Timeout path common.",
      "evidence": "Read-in-source (design): fire runs on a worker. No cancellation or single-flight rule is stated. Read-in-source: wl-clipboard-rs copy.rs:361 handles Cancelled inside the serving thread, and copy.rs:1054-1057 drops serve() errors. The daemon gets no signal when its source is replaced. Inferred: the interleaving.",
      "fix": "Allow one fire at a time. Cancel it on any Toggle or Deliver. Before sending keys, check that the daemon's Watcher has seen no selection change newer than this write's sequence number."
    },
    {
      "title": "Held is returned before the compositor has the selection",
      "severity": "minor",
      "scenario": "In the Tier 2 test, `cli(\"activate clip:1\")` returns 'copied clip 1' and read_text() runs at once. The serving thread may not have flushed set_selection yet. A loaded CI host can delay that thread, so the reader sees 'beta' and the test flakes. Held also proves nothing if the thread later fails, because that error is dropped.",
      "evidence": "Read-in-source: wl-clipboard-rs copy.rs:956 queues set_selection without flushing. copy.rs:1054 signals success before serve() at :1057. wayland-client-0.31.15 event_queue.rs:398-404: blocking_dispatch flushes only after dispatch_pending returns 0.",
      "fix": "Have deliver wait, with a short timeout, until the daemon's own Watcher reports a Changed event that carries OWN. Only then return Held. That proves the compositor applied the selection."
    },
    {
      "title": "The detect version gate cannot tell v26.04 from a main build that has #4548",
      "severity": "minor",
      "scenario": "Hongyi runs niri main at 4263c9a. Request::Version returns '26.04 (<commit>)', the same major.minor as the v26.04 release, which lacks the virtual-keyboard rework. A COPR or Nix build may put any string in the commit slot, or override the whole string. A numeric gate either refuses paste on a fixed niri or allows it on 26.04.",
      "evidence": "Read-in-source: niri src/utils/mod.rs:144-161 formats MAJOR.MINOR plus the commit, and NIRI_BUILD_VERSION_STRING overrides it entirely (:145-146). niri.spec.rpkg:127 sets the commit to the package version. Measured: gh api compare v26.04...7da6ae4 reports ahead_by=191. The latest release is v26.04 (2026-04-25).",
      "fix": "Allow paste only when the parsed release is greater than 26.04. On a 26.04 build, require an explicit config opt-in instead of parsing commit strings."
    },
    {
      "title": "Uploading lyncast's own keymap swaps the keymap for every client on every paste, and anvil cannot test the swap",
      "severity": "minor",
      "scenario": "Each paste sends a new keymap to every client's wl_keyboard, and again on swap-back. It also emits KeyboardLayoutsChanged, so a status bar's layout name flickers. X11 apps under xwayland-satellite must re-sync the X keymap mid-chord, which is where wtype reports garbled input. The Tier 2 paste spec runs on anvil, which keeps the virtual keyboard's keymap and never restores it. That test can neither catch restore bugs nor stay isolated from later tests.",
      "evidence": "Read-in-source: niri src/handlers/virtual_keyboard.rs:179-198 swaps the keymap, resets state, and emits a layouts IPC event. :205-231 restores it. smithay-pin src/input/keyboard/mod.rs:855 updates the keymap for every wl_keyboard. virtual_keyboard.rs:171-177 skips the swap when the keymap matches the configured one. smithay-pin anvil/src/input_handler.rs:1418-1421: 'Anvil never switches back'. Inferred: satellite behavior, from atx/wtype#62 cited in the grounding.",
      "fix": "Consider sending back the keymap lyncast received on its own wl_keyboard, and finding the v, Control_L and Shift_L keycodes in it with xkbcommon, so niri never swaps. Whichever keymap is used, start a fresh anvil per paste test."
    },
    {
      "title": "Overlay loses the keyboard while still visible under focus-follows-mouse on two monitors",
      "severity": "minor",
      "scenario": "Hongyi has two outputs and focus-follows-mouse on. The palette opens on output A. The pointer drifts to B, which becomes the active output. The overlay on A no longer counts, and the keyboard goes to B's window while the palette is still drawn. Hongyi types a query and presses Enter into a terminal on B.",
      "evidence": "Read-in-source: niri src/niri.rs:6933-6936 calls focus_output when the pointer moves to another output. src/niri.rs:1306-1307 checks exclusive layers only on the active output.",
      "fix": "Cancel the summon when the palette window loses keyboard focus (GPUI activation change). Treat it as cancel, not end."
    },
    {
      "title": "Choosing the terminal chord by app_id misses terminals with a custom app-id",
      "severity": "minor",
      "scenario": "Hongyi's dropdown terminal is `foot --app-id=scratch`. chord.rs sees 'scratch' and sends Ctrl+V. foot treats that as literal-next and inserts nothing.",
      "evidence": "Read-in-source: /tmp/paste-seam-scratch/foot.ini:7 makes app-id configurable. look's paste.rs:145-158 matches app_id and its reverse-DNS stem only. niri-ipc/src/lib.rs:1346-1350: pid is available but optional.",
      "fix": "Resolve the terminal from FocusedWindow.pid: read the /proc/<pid>/exe basename first, then fall back to the app_id stem, then to Categories=TerminalEmulator."
    },
    {
      "title": "Tier 1 inherits WAYLAND_DISPLAY and copies into the real clipboard",
      "severity": "minor",
      "scenario": "Hongyi runs cargo test from a terminal inside niri. The headless daemon inherits WAYLAND_DISPLAY=wayland-1. `lyncast activate calc:2+3` copies '5' into the real clipboard and prints 'copied 5', so the literal stderr check fails. The watcher also pulls the real clipboard into the temp store.",
      "evidence": "Read-in-source: wayland-client-0.31.15 conn.rs:69-71 returns NoCompositor only when WAYLAND_DISPLAY is unset. Otherwise it connects.",
      "fix": "In the Tier 1 harness, env_remove WAYLAND_DISPLAY and WAYLAND_SOCKET, and point XDG_RUNTIME_DIR at a temp dir."
    },
    {
      "title": "A daemon restart leaves the clipboard empty",
      "severity": "minor",
      "scenario": "Hongyi re-copies a clip, then rebuilds or upgrades lyncast, or GPUI panics. The selection is cleared. The next Ctrl+V pastes nothing, and whatever was on the clipboard before lyncast is gone too. The design's own daemon-stop test asserts this, but it is not listed under Tradeoffs.",
      "evidence": "Measured: PR/run.log:4-8 shows 'after copier exit' then ClipboardEmpty. Read-in-source: smithay ext_data_control source.rs:67-108, cited in the grounding, clears the selection when the source is destroyed.",
      "fix": "At startup, if the Watcher reports Cleared, re-offer the newest row marked with OWN. Or list this under Tradeoffs explicitly."
    },
    {
      "title": "Claim that a new backend is one paste/ file plus one match arm is overstated for GNOME and X11",
      "severity": "minor",
      "scenario": "GNOME: Mutter has no layer-shell and no data-control, so the summon window and Clips both fail before paste is reached. GNOME has no FocusedWindow API, so toggle's niri-based aim capture needs editing. The portal's Start can show a dialog, which cannot appear inside fire after close. X11/DCV: Clips must use X11 selections and the aim comes from _NET_ACTIVE_WINDOW, so lyncast::clip and toggle change too.",
      "evidence": "Measured: gh api tree of GNOME/mutter main (2337 entries, truncated=false) has RemoteDesktop files and no layer-shell, data-control or virtual-keyboard files. Read-in-source (design): Summon.aim is set in toggle from niri FocusedWindow. Clips is 'the only module that knows MIME strings' and owns wl-clipboard-rs. Read-in-source: portal Start 'typically' shows a dialog (flatpak RemoteDesktop doc, cited in the grounding).",
      "fix": "Let the backend own aim capture through a Paster::aim() that toggle calls. Add NoPaste::NeedsConsent and a `lyncast paste setup` command for the portal. Limit the one-file claim to key injection."
    }
  ]
}
```
