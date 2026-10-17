# lyncast runs as one resident process around a pure core

This page explains how lyncast fits together, for a new maintainer. The [decision log](../decisions.md) is the source of truth. Where this page disagrees with it, the log wins.

Evidence labels follow the research notes. Measured means a probe produced the number. Read in source means someone read the cited code. Inferred means a reasoned guess.

## One resident process owns the window and the clipboard

lyncast is one long-running GPUI process, called the daemon. Each summon opens a new wlr-layer-shell window, and each dismiss closes it. GPUI on Linux cannot hide a window, so the daemon cannot reuse one.

The probe in [notes/05](../research/notes/05-probe-summon-latency.md) compared this design with a fresh process per summon, on the lavapipe software renderer. A warm summon in the resident process reached its first present in a median of 44.9 ms. A fresh process took 137.0 ms. The resident process pays in memory instead. Its anonymous memory goes from 3.0 MiB before the first summon to 57.3 MiB after 20 summons and never comes back down. It levels off by about cycle 10, and idle CPU stays under 0.1%. All of these numbers are measured.

Most of the 45 ms is pipeline creation, which every new window repeats (measured by sampling). Each window compiles its own shaders and gets its own glyph atlas, and only the GPU device is shared (read in source, gpui-wgpu 0.3.9). A real GPU with a warm driver cache should make the summon faster (inferred). For a launcher, the delay on every summon matters more than memory held while idle, so the trade is worth it.

The clipboard also needs a resident process. Wayland clipboard data disappears when the app that offered it exits ([notes/03](../research/notes/03-wayland-niri-integration.md)). The daemon owns the selection, not the palette window, because GPUI's Wayland clipboard needs a focused window and an input serial ([notes/07](../research/notes/07-paste-seam-workflow.md), read in source).

The window sits on the Overlay layer with exclusive keyboard focus, because niri hides the Top layer behind fullscreen windows. Three more pieces complete the [process model](../decisions.md#process-model):

- `lyncast toggle` is a thin client. A niri bind such as `Mod+Space` runs it, and it sends one request over a Unix socket. Wayland has no global hotkeys, so the compositor has to start the client.
- `lyncast daemon --headless` runs the core, the clipboard watcher, and the socket without GPUI. Tests use it. [daemon.md](daemon.md) describes the daemon and its socket.
- The GPUI app runs with `QuitMode::Explicit`. By default GPUI quits on Linux when the last window closes (`gpui-pre` 0.3.9 `src/app.rs:3180-3187`). Every dismiss closes the last window, so the process would exit and drop its selection.

Inside the daemon, a separate capture thread owns the `wl-clipboard-rs` `Watcher`, because `Watcher::next_event` blocks with no timeout. The thread sends each new entry to the daemon over a channel.

## The core is pure, and the binary does the IO

`lyncast-core` holds ranking, the calculator, and delivery policy. Its crate doc says it does no IO, has no UI or Wayland dependencies, and takes the clock and every external input as arguments. This follows TinyCast's PURE layer, which TinyCast checks mechanically ([notes/06](../research/notes/06-tinycast-behaviour-spec.md)). As a result, every ranking and calculator rule runs in a plain unit test.

The `lyncast` binary holds the daemon, the GPUI palette, the CLI, and all platform code. The `clip` module owns `wl-clipboard-rs`, `rusqlite`, and the blob files. niri IPC goes through the `niri-ipc` crate.

`Clips` is the only owner of clip rows, and the core ranks a borrowed slice. In the [notes/07](../research/notes/07-paste-seam-workflow.md) review, a second copy of the rows in the core went out of sync after every Enter.

## A few shapes carry a row from feature to action

This sketch shows the core data shapes. The names come from the decision log and notes 07 and 08. The code settles the details. The shapes down to `Actions` are in `lyncast-core` now, and later steps add the rest.

```rust
// lyncast-core
pub enum Key { App(AppId), Command(Command), Clip(ClipId), Calc(String) } // "app:<id>", "cmd:<id>", "clip:<digits>", "calc:<raw query>"
pub enum Kind { App, Command, Clip, Calculator } // key.kind()
pub struct AppId(pub String); // a desktop entry id
pub struct ClipId(pub i64); // the sqlite rowid type

pub struct Row {
	pub subject: Subject, // the key, kind, and actions all come from it
	pub section: Section,
	pub title: String,
	pub card: Option<calc::Answer>, // the calculator card's answer, on the Answer row only
	// the palette derives the kind label from the kind; clip badges come with clips
}

pub enum Section { Answer, Results, Suggestions, Commands } // declaration order is the merge order

pub enum Command {
	ClipboardHistory, OpenConfig, ReloadApps, Quit,
	LockScreen, Suspend, LogOut, Reboot, PowerOff,
}

pub enum Subject { // the facts the chord table reads
	App { id: AppId, actions: Vec<DesktopAction> },
	Command(Command),
	Clip { id: ClipId, pinned: bool },
	Calc { query: String },
}
pub struct DesktopAction { pub id: String, pub name: String } // one [Desktop Action <id>] group
impl Subject {
	pub fn key(&self) -> Key; // ignores the facts, so a clip has one key pinned or not
	pub fn actions(&self) -> Actions; // the chord table
}

pub enum Chord { Enter, CtrlEnter }
pub enum Verb { Launch, LaunchAction(String), CopyPath, Copy, Pin, Unpin, Delete, Run } // paste adds a variant
pub enum ActionItem { // bound to the row's ids
	Launch(AppId), LaunchAction(AppId, DesktopAction), CopyPath(AppId),
	CopyClip(ClipId), CopyAnswer(String), Pin(ClipId), Unpin(ClipId), Delete(ClipId),
	Run(Command),
}
impl ActionItem {
	pub fn verb(&self) -> Verb;
	pub fn label(&self) -> &str;
}
pub struct Actions { /* the default item, then the menu-only items */ }
impl Actions {
	pub fn on(&self, chord: Chord) -> &ActionItem; // both chords run the default in the MVP
	pub fn pick(&self, verb: &Verb) -> Option<&ActionItem>; // None if the row does not offer the verb
	pub fn iter(&self) -> impl Iterator<Item = (Option<Chord>, &ActionItem)>; // menu order, default first
}

pub enum Delivery { Clip(ClipId), Answer(CalcAnswer), Text(String) } // Text is a path that Copy Path hands over
pub struct CalcAnswer(String); // only the calculator can build one

pub enum Outcome { Card(Answer), Error(String), Silent } // calc::evaluate

// lyncast binary
pub enum Via { Chord(Chord), Menu(Verb) }
pub enum Request {
	Toggle,
	Status,
	Query { text: String },
	Clips,
	Activate { key: Key, via: Via, confirmed: bool, query: String },
}
```

Each shape has one reason to exist:

- `Row` is one shape for every feature, so the palette and `lyncast query --json` show the same rows. It stores a `Subject`, and its key, kind, and actions come from that, so they cannot disagree.
- `Section` makes order a table. A new feature adds a section, not a rule in the merge.
- `Key` names a row without daemon state. It is a typed enum that the CLI and the socket parse once, so no resolver splits the prefix again. The calculator key carries the raw query, not the echo, so activation evaluates the same text again ([notes/08](../research/notes/08-calc-engine-workflow.md)).
- `Subject` holds the facts the chord table reads, such as an app's Desktop Actions and a clip's pin state. A table keyed only on kind could not list Desktop Actions or choose Pin or Unpin. The daemon builds the subject again from current state at every activation.
- `Chord` and `Verb` keep meaning out of the palette. The palette reports which keys were pressed, or which Actions menu item was picked, and one pure table maps the row's subject to its actions. A menu pick sends only the verb. Paste then adds a verb to the table and leaves the palette alone.
- `ActionItem` binds each action to its row's ids, such as `Pin(ClipId)` and `Run(Command)`. The daemon's dispatch is then an exhaustive match, with no unreachable arm for a pair such as an app and Pin.
- `Actions` makes the chord table the only list of what a row can do. Only `Subject::actions` builds one, so it is never empty and Enter always has an item. The footer, the Actions menu, and `lyncast query --json` all read it.
- `Command` is a closed enum of built-in commands. A new command is one variant plus its `Command::ALL` entry. The compiler finds every match to update, but not a missing `ALL` entry.
- `Delivery` follows the notes/07 rule that the core "says what to hand over, never how". It names the row, not the bytes, because the store already owns the bytes.
- `Outcome` is all the calculator returns. The daemon builds the row around it, so the engine knows no verbs.
- `Request` is one protocol for the palette and the CLI. Requests are async from the start, because a paste result will arrive after the window closes.

## The input box routes by scope, then fans out and merges

The [input box](../decisions.md#input-box) works in two steps:

1. An explicit scope routes the query to one feature. Tab is a scope, and so is a registered prefix.
2. Without a scope, every root feature evaluates the query. Apps and built-in commands rank together in Results. A merge step orders the rows by the section table, Answer then Results, so the calculator card sits at index 0. A Fallback section (for example web search) can follow later. The MVP has no fallback rows.

Fan-out stays under the 0.1 ms budget per keystroke on a 300-app desktop. The calculator takes 0.45 µs on its 26 shared inputs ([calculator](calculator.md#the-ports-cost-per-keystroke), measured). On 300 apps the worst bench query for ranking apps and commands, with frizbee and the comparator, took 71 µs, and 79 µs in the slowest of four bench runs ([ranking](ranking.md#the-worst-bench-keystroke-takes-71-µs-on-300-apps-and-254-µs-on-1000), measured). The budget breaks in three places. On 1,000 apps the worst query of every typed group takes 139 to 254 µs. When some field is as long as a pasted query, a paste of 109 characters took 228 µs in the worst of six runs, and one of 896 takes about 2.1 ms. The calculator's worst capped input takes 602 µs. zlaunch and look both work this way ([notes/09](../research/notes/09-input-box-prior-art.md), read in source).

No step sniffs keywords to pick a feature. Each feature's own parser decides whether the query is its input. A rule like "contains x" fires on `firefox`, while the calculator parser keeps `xterm`, `7zip`, `0x`, and `12 x` silent.

The answer outranks a matching app, so `8x8` shows 64 first and the app second. That is a lead default Hongyi can override. A code comment in look gives the reason, "an expression is a question, and the answer outranks a file that fuzzy-matched some of its digits" ([notes/09](../research/notes/09-input-box-prior-art.md)).

Hongyi decided that any `NxN` shape is math, so `1920x1080` shows 2,073,600 ([calculator engine](../decisions.md#calculator-engine)).

The MVP registers no text prefixes. Tab is the only scope, and it opens the clipboard screen, as the Clipboard History command does.

## The empty input shows suggestions, then commands

Before the first keystroke the palette shows two sections, as Raycast does ([decision log](../decisions.md#input-box)):

- **Suggestions** holds the top 5 apps and commands by frecency. On a fresh install, with no usage yet, it shows Clipboard History and then the first apps alphabetically.
- **Commands** lists every built-in command that Suggestions does not hold, alphabetically.

Each row shows its kind, Command or Application, on the right. The commands are Clipboard History, Open Config, Reload Apps, Quit lyncast, Lock Screen (`loginctl lock-session`), Suspend, Log Out (`niri msg action quit --skip-confirmation`), Reboot, and Power Off. Typing searches commands like apps, so `clip` finds Clipboard History.

Log Out, Reboot, and Power Off ask for confirmation. The daemon answers the first `Activate` with a confirmation reply, the palette asks, and the second `Activate` carries `confirmed: true`. The CLI spells it `lyncast activate cmd:reboot --yes`.

## Ctrl+K opens the Actions menu

Ctrl+K opens a small, searchable menu for the selected row. The menu lists the row's `ActionItem`s, default first with its key. Apps offer Open, the entry's Desktop Actions, then Copy Path to .desktop. Clipboard rows offer Copy, Pin or Unpin, and Delete. The calculator card offers Copy Answer. Commands offer Run. Picking an item sends `Activate` with `Via::Menu(verb)`. The daemon rebuilds the row's subject from current state, and `pick` returns the item only if the rebuilt list still offers the verb.


## A summon is one window, from toggle to close

A summon goes through these steps:

1. A niri bind runs `lyncast toggle`, which sends `Request::Toggle`.
2. If no summon is open, the daemon's host opens a new layer-shell window. If one is open, the toggle closes it ([palette](palette.md#a-summon-opens-one-window)).
3. Each keystroke runs the input box and redraws the rows.
4. The activating keys, or a pick in the Actions menu, send `Request::Activate { key, via, confirmed, query }`. The daemon rebuilds the row's `Subject` from the key and current state, takes the item for the chord or the picked verb from its `Actions`, and runs it. If the action fails, the window stays open.
5. After a CLI client's successful action, the host schedules the close with `cx.defer`, so it runs outside every entity update. `remove_window` only marks the window (`src/window.rs:2384`), so a window's own handler can close it too. What fails is `update_window` on a window that is already being updated, because GPUI takes a window out of its slot while it updates (`src/app.rs:2201-2215`, read in source). [notes/07](../research/notes/07-paste-seam-workflow.md) read that failure as a failed close from inside the window's handler.
6. The palette closes when the activating keys are released. Once paste exists, a held Shift or Alt would otherwise merge with the injected Ctrl+V.
7. `on_window_closed` clears the summon. The hook also sees a close that niri starts when an output goes away, so the toggle state follows the real window (notes/07, read in source).

A successful launch closes the palette too. While an exclusive Overlay surface stays mapped, niri gives it the keyboard, so the new app would get no keys ([notes/07](../research/notes/07-paste-seam-workflow.md), read in source, niri `src/niri.rs:1378-1394`).

Losing keyboard focus cancels the summon, as TinyCast hides on focus loss. On two monitors with focus-follows-mouse, the palette can lose the keyboard while it is still drawn (notes/07, read in source). Whether the palette sends the request on key press or on key release is undecided.

## The palette and the CLI speak one protocol

Every palette action also exists as a CLI command over the same socket protocol. The dev box is headless, so tests are CLI integration tests that assert literal output. The shared protocol means those tests exercise the real path ([testing](../decisions.md#testing)). Tier 1 tests clear `WAYLAND_DISPLAY`. Without that, a test run inside niri would copy into the developer's real clipboard (notes/07, read in source).

The CLI maps onto the requests:

- `lyncast toggle` sends `Toggle`.
- `lyncast status` sends `Status` and prints `open` or `closed`.
- `lyncast query <text> --json` sends `Query` and prints one JSON object per row, with its key, index, kind, section, title, and actions ([ranking](ranking.md#lyncast-query-prints-one-json-object-per-row)). The calculator card's row also has its `card`. The query accepts a leading hyphen (`allow_hyphen_values`), because clap rejected `lyncast query -5+3`. A query that is an option's name, `--json` or `--help`, or `--` itself needs `lyncast query --json -- <text>` ([notes/08](../research/notes/08-calc-engine-workflow.md), measured).
- `lyncast clip list --json` sends `Clips` and prints one JSON object per clip ([clipboard and paste](clipboard-and-paste.md#lyncast-clip-list-shows-the-history)).
- `lyncast activate <key>` runs a row's action, chosen with `--chord` or `--verb`. `--yes` confirms, and `--query` passes the search text. In the notes/07 design, `lyncast activate 'calc:2+3*4'` prints `copied 14`.

The exit codes are part of the contract:

- `0` means the request succeeded.
- `1` means the action failed. The notes/07 design specifies `lyncast: no clip 9` on stderr for a missing clip.
- `2` means a usage error, because clap uses 2 for every usage error (notes/08, read in source).
- `3` will mean a stale rate snapshot, once currency exists. It has its own code so a test cannot pass on a usage error.

[daemon.md](daemon.md) gives the socket path, the wire format, the CLI's options and output, and the command that starts the daemon.

## New features plug into tables and modules

**Paste backends.** They live later in the daemon's `paste/` module, and the first one is the niri virtual keyboard. Paste adds a verb to the chord table, and Ctrl+Enter is the slot it will use. The rest of the paste design is undecided. The notes/07 reviews found two constraints in niri source. A backend has to capture the target window at toggle, because `FocusedWindow` reports nothing once a layer surface has focus. It has to open the niri event stream then too, because a late subscriber never gets `WindowFocusChanged`. No niri release has the virtual-keyboard rework, PR #4548, merged on 2026-10-13 (measured). See [clipboard-and-paste.md](clipboard-and-paste.md).

**Files.** File search is out of the MVP. A files feature would bring its own parser, a row kind for the chord table, and a section (inferred). look scopes files with an `f"` prefix ([notes/09](../research/notes/09-input-box-prior-art.md)). The section position and the prefix are undecided.

**Units and currency.** They come later in the calculator engine as new kinds of quantity, not as separate features. Values already carry a dimension, which is empty in the MVP. Parsing never reads `Env`, and atoms get their price during evaluation. That keeps "No exchange rate" and "no snapshot" error cards possible, as the first use of `Outcome::Error`. The base prototype took units and currency with 24 added and 5 removed lines in old files ([notes/08](../research/notes/08-calc-engine-workflow.md), measured). See [calculator.md](calculator.md).

**X11.** X11, for a DCV session, touches more than `paste/`. In GPUI, `WindowKind::LayerShell` exists only under `cfg(wayland)`, and `Clips` uses only `wl-clipboard-rs` (notes/07, read in source). X11 also has no niri binds, so the hotkey needs `XGrabKey` or a desktop keybinding, which is unverified on Xdcv ([notes/03](../research/notes/03-wayland-niri-integration.md)). notes/03 suggests one interface with separate backends for the clipboard and for launching. The X11 design is undecided.
