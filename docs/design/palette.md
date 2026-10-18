# The palette is one window per summon

This page describes the palette, lyncast's GPUI window, for a maintainer and for the tests. The [decision log](../decisions.md#palette) is the source of truth. Where this page disagrees with it, the log wins.

## lyncast daemon runs the palette

`lyncast daemon` runs the daemon with the palette, and `lyncast daemon --headless` runs it without, as [daemon.md](daemon.md#start-the-daemon-with-lyncast-daemon) says. The palette needs a Wayland compositor. Without `WAYLAND_DISPLAY`, `lyncast daemon` prints `lyncast: not started: the palette needs a Wayland compositor; run lyncast daemon --headless without one` and exits 1 before it takes the lock. GPUI's platform, which lyncast allows only Wayland, would otherwise panic once the daemon held the lock (`gpui-pre-linux` 0.3.9 `src/linux/platform.rs:369-373`, read in source).

The daemon starts as the headless one does: the lock, the scan, the visits, the clips, the capture thread, and the socket, in daemon.md's order. Then GPUI's app runs, with `QuitMode::Explicit`, so the process stays when the last window closes and keeps the clipboard selection it serves. A client that connects before GPUI runs waits in the daemon's channel until GPUI takes it.

The daemon lives on GPUI's main thread. One task there, the owner task, hands it every event that the connection threads, the workers, the copier, the capture thread, and the writers queue, and each of them rings a bell that wakes the task once its event is queued. The palette's own requests go to the daemon through one `Host::ask`, on the same thread, and get the daemon's typed answer back. Quit runs through the same `Owner` as the headless host's, in `Owner::stop`, `Stopping::wait`, `Owner::drain`, and `Owner::finish_blocking`: the socket goes, the work in flight and the last writes get 5 s, then the reply.

## A summon opens one window

The palette host holds the summon as a window. A socket client's answer changes it by daemon.md's rules: `toggle` opens a window when the summon is closed and closes it when it is open, a success closes it, and Clipboard History opens it. A failure leaves it.

The host opens and closes windows only from `cx.defer` callbacks, which GPUI runs at the end of its effect cycle, when no entity is being updated. `open_window` builds the window's view and draws it before it returns (`gpui-pre` 0.3.9 `src/app.rs:1500-1525`), and the view asks the host for its rows, so an open inside one of the host's updates would update the host twice, which panics (`src/app/entity_map.rs:230-233`). A close calls `window.remove_window()`, which only marks the window (`src/window.rs:2384`). GPUI removes a marked window once the window's update returns, and then calls `on_window_closed` (`src/app.rs:3155-3187`). All of these were read in source.

`on_window_closed` sees every window that closes while GPUI's loop runs, and traces it. When the summon's own window goes without the host closing it, the summon closes too. A compositor closes a window through the same `remove_window` (`src/window.rs:1750-1758`, read in source), but no test here closes one that way yet.

The window is a layer-shell surface of 750 by 480 logical pixels on the Overlay layer, with the keyboard to itself. It is anchored to the top edge, so the compositor centers it, and it never resizes. Its top margin is a third of the space the window leaves on the display GPUI lists first, in whole pixels. GPUI on Wayland has no primary display (`gpui-pre-linux` 0.3.9 `src/linux/wayland/client.rs:1256-1258`), and it knows the displays once it has connected, before the first window (`src/linux/wayland/client.rs:1197-1203`), both read in source. The `geometry` block below gives the size and the margins. A test in `crates/lyncast/src/palette/theme.rs` checks the margins the code computes against it, and one in `crates/lyncast/src/palette/tests.rs` checks the window's options for the display of GPUI's test platform.

## The field searches the root list

The window holds the field at the top, the list under it, and the footer at the bottom. The theme is dark, and the text is DejaVu Sans. Each summon starts with an empty field and the root list of an empty query. The empty field shows the placeholder `Search for apps and commands`. Each change to the field's text asks the daemon for the root list of the new text, through `Host::ask` on GPUI's main thread, and the list shows the answer. The list draws each section's header, then the section's rows, each with its kind on the right. The calculator's card comes first and has no header. It shows the expression and its badge, an arrow, and the answer and its words.

The field is one line of at most 1,024 bytes, as many as ranking and the calculator read. A line break in the text the field takes, such as an input method's commit, becomes a space, as TinyCast turns the line breaks of a pasted query into spaces ([notes/06](../research/notes/06-tinycast-behaviour-spec.md)). Text past the cap is dropped, cut on a char boundary. An input method's composing text sits in the field, underlined, and the query holds it. The placeholder stays hidden while an input method composes, also before the composition shows any text, as when a compose key has begun a sequence. Text wider than the field moves left only as far as the caret needs to stay inside the field, so a caret past the field's right edge sits at that edge, and the input method's place for the caret moves with it. The field draws nothing outside itself. `the_caret_stays_in_the_field` in `crates/lyncast/src/palette/tests.rs` puts 1,024 bytes in the field, moves the caret through them, and composes 1,024 bytes. After each step the window draws the caret once, wholly inside the field, at the field's left edge for a caret at the start and at its right edge for one past it, the input method's place for the caret is where the caret is drawn, and the composing underline shows nowhere outside the field.

One row is selected. A new list selects its first row, so each change to the text selects the first row again. Up and Down move the selection, and so do Ctrl+P and Ctrl+N. The selection stops at both ends. The list scrolls to keep the selected row in view, and the row's section header too when the row is the section's first. The footer names the selected row's default action, such as `Open` or `Run`.

Escape clears a field that holds text, and closes the palette when the field is empty. Backspace removes the char before the caret and never closes the palette, also in an empty field. Left, Right, Home, End, and Delete move the caret and edit as in a text field. Escape and Backspace follow TinyCast's palette ([notes/06](../research/notes/06-tinycast-behaviour-spec.md)).

lyncast binds no GPUI keys. GPUI runs the action of a key binding before any key listener, and the action's handler does not learn whether its key repeats (`gpui-pre` 0.3.9 `src/window.rs:6150-6174`, `src/interactive.rs:25-31`). The view reads every key press in the capture phase through the one table in `crates/lyncast/src/palette/keys.rs` (`src/window.rs:6260-6276`). A key the table names stops there. Any other key goes on to the input handler, which types the key's text (`gpui-pre-linux` 0.3.9 `src/linux/wayland/window.rs:1576-1598`). All of these were read in source.

## Enter runs the selected row as it goes down

Enter and Ctrl+Enter send `activate` for the selected row as the key goes down. The field's text goes with it as the query, which the visit keeps as a search term. A held key repeats as key presses that GPUI marks `is_held`. Only Enter's first press sends, and only Escape's first press takes a step. One activation is out at a time. Until its answer comes, and while the window closes after it, every key press but Escape does nothing, and Escape closes the window. A job's answer comes once its worker thread ends, so the view waits for it on GPUI's main thread without blocking it.

A success closes the window once the activating key is up and no modifier is held, in whichever order the release and the modifiers' change come. Under Wayland they come as separate events (`gpui-pre-linux` 0.3.9 `src/linux/wayland/client.rs:2115-2140, 2258-2270`, read in source). The release of another key, or of an Enter the window never saw go down, closes nothing. niri's bind leaves such a release as the window maps. A success sent by a socket client closes the window at once, as daemon.md's rules say. Clipboard History's success leaves the window as it is.

A failure leaves the window open and shows its reason in the footer until the next key that edits the field or moves the selection. Log Out, Reboot, and Power Off ask first: the daemon answers with a confirmation, and the footer asks `Reboot?` with Enter to run and Escape to cancel. Enter there sends the same activation again with `confirmed`. A key that moves the selection or edits the field, and typed text, cancel the question.

Losing the keyboard focus closes the window, as TinyCast hides on focus loss ([notes/06](../research/notes/06-tinycast-behaviour-spec.md)), but only once the window has been active. The view tells the model whether the window is active as it opens and at each change. Under anvil the window turns active only with the first key that reaches it (a GPUI probe under anvil, 2026-10-17), and GPUI's test platform opens a window inactive (`gpui-pre` 0.3.9 `src/platform/test/window.rs:414-425`, read in source).

## The compositor's end stops the daemon

When the compositor goes, GPUI's event loop ends and calls GPUI's quit hooks (`gpui-pre-linux` 0.3.9 `src/linux/platform.rs:608-624`). GPUI then drops its windows without calling `on_window_closed` (`gpui-pre` 0.3.9 `src/app.rs:1168-1184`), and runs no task on the main thread again. These were read in source. lyncast's quit hook takes the daemon from the host. Once GPUI's run returns, the daemon stops as the headless host does, with the summon closed, because its windows went with the loop. The capture thread sees the compositor go too, and that stops the daemon as Quit does. It prints `lyncast: the Wayland connection closed` and exits 1.

## --trace prints the bench's PROBE lines

`lyncast daemon --trace` prints `PROBE seq=<n> ev=<name> mono_ns=<CLOCK_MONOTONIC ns>` lines on stdout, one per event, as `bench/probe` does. The flag is hidden, and `--headless` refuses it. Each summon has a number, from 1, and the app's own line has 0. The events are:

| Event | When |
|---|---|
| `app_ready` | the owner task first runs, inside GPUI's event loop |
| `toggle_handled_open` | the host opens the summon |
| `first_render` | the window's view first renders, inside `open_window` |
| `window_opened` | `open_window` returns |
| `frame1_pre_present` | GPUI is about to draw and present the first frame |
| `frame1_presented` | the first present has returned |
| `frame1_done` | GPUI is about to draw the next frame |
| `window_closed` | the window has gone while GPUI's loop runs, however it went |

Each line's time is read as its event happens, and one thread writes the lines, so a stdout that stops draining, such as a pipe that no one reads, holds up that thread and nothing else: the daemon still answers, as it does when stderr stops draining ([daemon.md](daemon.md#one-thread-writes-the-warnings)). Up to 64 lines wait for the thread. A line that does not fit is dropped, and once stdout drains, `PROBE dropped=<n>` follows the lines that waited. The daemon waits up to 1 s for its lines before it exits, so a stdout that does not drain keeps no daemon from exiting.

## The fixtures

Four tests read the blocks below. The Tier 3 runner in `crates/lyncast/src/palette/tests.rs` reads the `desktop` and `palette` blocks, and `the_window_is_placed_as_the_geometry_block_says` there and `the_geometry_is_as_the_doc_says` in `crates/lyncast/src/palette/theme.rs` read the `geometry` block. The test in `crates/lyncast/tests/palette.rs` reads the `env`, `desktop`, `executable`, and `summon` blocks, and runs each `summon` block as [clipboard-and-paste.md's](clipboard-and-paste.md#the-fixtures) tests run a `clipboard` block: in a fresh `$TREE`, inside its own `scripts/test-env.sh start` environment, whose `XDG_RUNTIME_DIR` is `$RUNTIME`, with only the `env` block's variables and the compositor's `WAYLAND_DISPLAY`. `VK_DRIVER_FILES` selects lavapipe, as `scripts/bench.sh` does.

```env
HOME=$TREE/home
XDG_DATA_HOME=$TREE/data
XDG_CONFIG_HOME=$TREE/config
XDG_DATA_DIRS=$TREE/share
XDG_RUNTIME_DIR=$RUNTIME
XDG_CURRENT_DESKTOP=niri
LANG=C
PATH=$TREE/bin
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
```

The tree holds daemon.md's two apps, Firefox with one Desktop Action, and 8x8:

```desktop share/applications/firefox.desktop
[Desktop Entry]
Type=Application
Name=Firefox
Exec=firefox %u
Actions=new-window;

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
```

```executable bin/firefox
#!/bin/sh
```

```desktop share/applications/8x8.desktop
[Desktop Entry]
Type=Application
Name=8x8
Exec=8x8
```

```executable bin/8x8
#!/bin/sh
```

### The palette blocks run on GPUI's test platform

`every_palette_block_runs_as_the_doc_says` in `crates/lyncast/src/palette/tests.rs` runs each `palette` block against a daemon and a palette host of its own, in a GPUI `TestAppContext` app. The daemon has the apps of the `desktop` blocks above, no visits, no data home, and no compositor, so a copy fails with `not copied: no Wayland compositor`. Its runner records each program to a scratch file. A block is a list of steps. Each step is a `$ ` line, and the lines after it say what the palette shows once the step has run:

| Step | Does |
|---|---|
| `$ open`, `$ toggle` | sends `toggle` as a socket client sends it |
| `$ type <text>` | GPUI's `simulate_input`, one key press for each char |
| `$ press <keys>` | GPUI's `simulate_keystrokes`, one key press for each keystroke, in GPUI's syntax |
| `$ insert <text>` | the input handler's `replace_text_in_range`, as an input method's commit calls it. `\n` and `\r` are line breaks |
| `$ mark [<text>]` | the input handler's `replace_and_mark_text_in_range`, as an input method calls it while it composes |
| `$ unmark` | the input handler's `unmark_text` |
| `$ hold <key>` | GPUI's `simulate_event` with a key press marked `is_held`, as a held key repeats |
| `$ release <key>` | `simulate_event` with the key's release |
| `$ mods <none\|ctrl\|shift\|alt>` | `simulate_modifiers_change`. `$ press` changes no modifier, so a block holds Ctrl with `$ mods ctrl` before it presses `ctrl-enter` |
| `$ focus`, `$ blur` | the window turns active, or inactive. A blur acts only on an active window, as GPUI's test platform does |
| `$ stall-runner` | makes the runner's file a FIFO, so the next program waits in the runner and its activation stays out |
| `$ resume-runner` | lets the waiting program write its line, and makes the runner's file a file again |

After each step, the runner runs GPUI's tasks until none is ready and the daemon has no job out. While the runner is stalled, it waits for no job. Then it compares the palette's state with the lines after the step, and the programs the step started with the step's `record:` lines. A step with no `record:` line started none. The lines spell that state through the functions that give the view its labels and their order, but a key reads as the steps name it. So the lines do not check the key caps' text, or how the view lays the labels out. They show:

- `root "<text>"`, or `root (<placeholder>)` while the placeholder shows.
- `== <header>` before each section that has one.
- Each row, after `> ` when it is selected and after two spaces when it is not. An app or a command reads `<title>  (<kind>)`. The calculator's card reads `= <answer>  (<expression> · <badge>[ · <words>])`.
- `No results` when the list is empty.
- `confirm <title>?` while a command asks to be confirmed.
- The footer: `footer: <default action> (enter)`, or `footer: <title> (enter) · Cancel (escape)` while a command asks, or `footer: pending` while an activation is out, or `footer: closing` after a success while the key is still down, or `footer: failed: <reason>`.
- `closed` once the window has gone, or `stopped` once the daemon's stop has finished too.

### The field searches as it changes

An empty field shows Suggestions, then Commands. `8x8` puts the calculator's card first, selected, and the app second. Ctrl+N, Ctrl+P, Down, and Up move the selection, which stops at both ends. Escape clears the field, and the full list selects its first row again, not the second. A second Escape closes the window.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type 8x8
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter)
$ press ctrl-n
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter)
$ press down
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter)
$ press ctrl-p
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter)
$ press down up up
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter)
$ press down
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter)
$ press escape
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ press escape
closed
```

### Backspace edits the field and never closes the window

Backspace in the empty field leaves the window open. The caret keys edit where the caret is: `fox`, then Home and `fire`, then Space, give `fire fox`, and `fox`, then two Lefts and `i`, then Right and `z`, give `fiozx`.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ press backspace
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type fox
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press home
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ type fire
root "firefox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press space
root "fire fox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press end backspace backspace backspace backspace
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press home delete delete delete delete backspace
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type fox
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press left left
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ type i
root "fiox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press right
root "fiox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ type z
root "fiozx"
No results
```

### A line break in inserted text becomes a space, and composing text hides the placeholder

An input method's empty composition already hides the placeholder, and its commit takes the place of the composing text.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ insert fire\r\nfox
root "fire fox"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press escape
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ mark
root ""
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ unmark
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ mark f
root "f"
== Results
> Firefox  (Application)
footer: Open (enter)
$ insert fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
```

### Each summon starts on an empty field

A toggle from a socket client closes the window, and the next summon opens on the empty field again.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type 8x8
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter)
$ toggle
closed
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
```

### Enter runs the row on the press and closes the window on the release

Enter sends the activation as it goes down. The launch succeeds, so the window closes once Enter is up, and the release of another key does not close it. A second press while the window closes sends nothing, and neither does a held Enter's repeat.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press enter
root "fire"
== Results
> Firefox  (Application)
footer: closing
record: ["firefox"]
$ press enter
root "fire"
== Results
> Firefox  (Application)
footer: closing
$ hold enter
root "fire"
== Results
> Firefox  (Application)
footer: closing
$ release space
root "fire"
== Results
> Firefox  (Application)
footer: closing
$ release enter
closed
```

### One activation is out at a time

With the runner stalled, the launch's job stays out, and the footer shows it pending. A second Enter and a Down do nothing then, and Enter goes up before the answer comes. Once the program starts, the success closes the window at once, because Enter is up already. The recorder holds one launch.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ stall-runner
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press enter
root "fire"
== Results
> Firefox  (Application)
footer: pending
$ press enter down
root "fire"
== Results
> Firefox  (Application)
footer: pending
$ release enter
root "fire"
== Results
> Firefox  (Application)
footer: pending
$ resume-runner
closed
record: ["firefox"]
```

### Escape closes the window while an activation is out

The program still starts, and its answer goes nowhere.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ stall-runner
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press enter
root "fire"
== Results
> Firefox  (Application)
footer: pending
$ press escape
closed
$ resume-runner
closed
record: ["firefox"]
```

### A failure stays in the footer, and a repeat activates nothing

The card's Copy Answer fails without a compositor, and the window stays open with the reason in the footer. A repeat of Enter neither sends again nor clears the reason, and an input method's unmark does not clear it either. The next key that moves the selection clears it. A held Enter that the palette never saw go down sends nothing, and a repeat of Escape leaves the field's text. Copy Answer fails again, and typed text clears the reason too.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type 8x8
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter)
$ press enter
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ hold enter
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ release enter
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ unmark
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ press down
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter)
$ hold enter
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter)
$ hold escape
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter)
$ press up enter
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ type 0
root "8x80"
> = 640  (8 × 80 · Product · Six Hundred Forty)
footer: Copy Answer (enter)
```

### Ctrl+Enter closes the window once Ctrl is up too

Ctrl+Enter runs the row's Ctrl+Enter item, which is Open for an app. The window closes once both keys are up, in either order. The launch is a visit, so the next summon suggests Firefox first.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ mods ctrl
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press ctrl-enter
root "fire"
== Results
> Firefox  (Application)
footer: closing
record: ["firefox"]
$ release enter
root "fire"
== Results
> Firefox  (Application)
footer: closing
$ mods none
closed
$ open
root (Search for apps and commands)
== Suggestions
> Firefox  (Application)
  Clipboard History  (Command)
  8x8  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Open (enter)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ mods ctrl
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter)
$ press ctrl-enter
root "fire"
== Results
> Firefox  (Application)
footer: closing
record: ["firefox"]
$ mods none
root "fire"
== Results
> Firefox  (Application)
footer: closing
$ release enter
closed
```

### Reboot asks first

The daemon answers Reboot's first activation with a confirmation, and the footer asks. Escape cancels the question, and the field keeps its text. Down cancels it too, and an input method's unmark does not. Enter at the question sends the activation again, confirmed. Escape while the window closes closes it at once. The run is a visit, so the next summon suggests Reboot first, and text typed at its question cancels the question.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type reboot
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter)
$ press enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ release enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ unmark
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ press escape
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter)
$ press enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ release enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ press down
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter)
$ press enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ release enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ press enter
root "reboot"
== Results
> Reboot  (Command)
footer: closing
record: ["systemctl","reboot"]
$ press escape
closed
$ open
root (Search for apps and commands)
== Suggestions
> Reboot  (Command)
  Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ press enter
root (Search for apps and commands)
== Suggestions
> Reboot  (Command)
  Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ type reb
root "reb"
== Results
> Reboot  (Command)
footer: Run (enter)
$ press escape escape
closed
```

### Keys the window did not see go down do nothing

niri's bind leaves the release of Space and a change of the modifiers as the window maps. Neither closes the window, and neither does the release of an Enter the window never saw go down. Clipboard History's success leaves the window as it is.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ release space
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ mods none
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ release enter
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ press enter
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ release enter
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
```

### Focus loss closes only a window that has been active

The window starts inactive, as GPUI's test platform opens it. A blur then changes nothing. Once the window has been active, turning inactive closes it.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ blur
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ focus
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ blur
closed
```

### Quit lyncast stops the daemon

Quit lyncast closes the window, and the daemon's stop finishes. This daemon has no capture thread and nothing to write, so no other event wakes the owner task, and the palette's own Quit wakes it.

```palette
$ open
root (Search for apps and commands)
== Suggestions
> Clipboard History  (Command)
  8x8  (Application)
  Firefox  (Application)
== Commands
  Lock Screen  (Command)
  Log Out  (Command)
  Open Config  (Command)
  Power Off  (Command)
  Quit lyncast  (Command)
  Reboot  (Command)
  Reload Apps  (Command)
  Suspend  (Command)
footer: Run (enter)
$ type quit
root "quit"
== Results
> Quit lyncast  (Command)
footer: Run (enter)
$ press enter
stopped
```

### The summon blocks run under anvil

A `summon` block's `lyncast daemon` line has no `--headless`. Its daemon's stdout is `$RUNTIME/daemon.stdout`, as daemon.md's harness appends it. A block may use clipboard-and-paste.md's lines, such as `seed`, `paste`, `wait-exit`, `freeze-thread`, and `stop-compositor`, and these:

- `$ wait-trace <event> <seq>` waits until `$RUNTIME/daemon.stdout` holds that summon's line for the event.
- `$ count-trace <event>` prints how many lines for the event it holds.
- `$ keys <key>...` types keys into the compositor with `python3 -I scripts/x11.py <display> focus keys <key>...`, on the X display that `$RUNTIME/display` names. Each key is an X keysym name, such as `f` or `Return`, and `ctrl+v` holds Ctrl while it taps `v`. The script gives the X input focus to anvil's window, sends the keys through XTEST, and returns once the X server has handled them.
- `$ wait-summon <open|closed>` runs `lyncast status` until it prints that state.
- `$ stall-stdout` gives the next daemon a stdout that does not drain: a pipe that the test fills before the daemon starts, in place of `$RUNTIME/daemon.stdout`. `$ drain-stdout` reads what fills it, and from then on what the daemon writes to stdout goes on to `$RUNTIME/daemon.stdout`.

anvil reads the keys after `keys` returns, so a `keys` line holds no `record:` lines, and the programs its keys start count on the next line's. That line waits for what the keys do, as `wait-summon closed` waits for the window that a launch closes. Without that focus, anvil gave a GPUI window no key at all, and with it the window turned active at its first key (a GPUI probe under anvil, 2026-10-17). The keys summon block below fails when `x11.py` skips the focus (a deliberate break, 2026-10-17).

### A summon opens around activate and closes after it

`toggle` opens a window, and an app's launch closes it, so `status` reads `closed` and the window goes. A second toggle opens a second window, and a third closes it. A fourth opens a third window, and Quit closes that one and stops through the palette host's steps: the socket goes, the visit is written, and the daemon exits 0.

```summon
$ lyncast daemon --trace
$ lyncast status
closed
$ lyncast toggle
open
$ wait-trace frame1_presented 1
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
record: ["firefox"]
$ lyncast status
closed
$ wait-trace window_closed 1
$ lyncast toggle
open
$ wait-trace frame1_presented 2
$ lyncast toggle
closed
$ wait-trace window_closed 2
$ lyncast toggle
open
$ wait-trace frame1_presented 3
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
$ count-trace window_opened
3
$ count-trace window_closed
3
$ test -e "$RUNTIME/lyncast.sock" || echo gone
gone
$ grep -o '"app:firefox.desktop"' data/lyncast/ranking.json
"app:firefox.desktop"
```

### Quit waits for the last write with the window open

Here the test holds the writer still, as [daemon.md's](daemon.md#a-slow-write-holds-up-nothing-but-quit) slow write does, so the write of Lock Screen's visit waits. A shell line makes `ranking.json` a directory, which the rename cannot replace, and the test lets the writer go on a second later, while Quit waits for the write. The write's warning is on stderr as soon as Quit has replied, so the palette host's stop waited for the writer before the reply, and the daemon exits 0. Quit closes the open window too.

```summon
$ mkdir -p data/lyncast
$ lyncast daemon --trace
$ freeze-thread visits
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast toggle
open
$ wait-trace frame1_presented 1
$ mkdir data/lyncast/ranking.json
$ thaw-thread 1s
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: not saved: Is a directory (os error 21)
$ wait-exit
stderr: lyncast: $TREE/data/lyncast/ranking.json: not saved: Is a directory (os error 21)
$ count-trace window_closed
1
```

### The selection stays when the window closes after a copy

A copy closes the window. The daemon keeps running with no window, so the clipboard still holds the entry, and a second toggle opens a second window.

```summon
$ lyncast daemon --trace
$ seed text/plain;charset=utf-8 alpha
$ wait-clips 1
$ lyncast toggle
open
$ wait-trace frame1_presented 1
$ lyncast activate clip:1
copied clip 1
$ lyncast status
closed
$ wait-trace window_closed 1
$ paste text/plain;charset=utf-8 got
$ cat got; echo
alpha
$ lyncast toggle
open
$ wait-trace frame1_presented 2
$ count-trace window_opened
2
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
```

### The compositor's end stops the daemon with the window open

The compositor goes while the window is open. The daemon stops as Quit does and exits 1. Here the test holds the writer still again, and lets it go on a second later, once `ranking.json` is a directory. The write's warning comes before the daemon's last line, so the stop waited for the writer. The clip row is in the database.

```summon
$ lyncast daemon --trace
$ seed text/plain;charset=utf-8 alpha
$ wait-clips 1
$ freeze-thread visits
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast toggle
open
$ wait-trace frame1_presented 1
$ mkdir data/lyncast/ranking.json
$ thaw-thread 1s
$ stop-compositor
$ wait-exit
stderr: lyncast: $TREE/data/lyncast/ranking.json: not saved: Is a directory (os error 21)
stderr: lyncast: the Wayland connection closed
exit: 1
$ sqlite3 data/lyncast/clips.sqlite3 "select id, body from clips"
1|alpha
```

### Keys typed under anvil reach the palette

The keys go through Xvfb's XTEST and anvil to the palette. Enter launches Firefox, and the window closes once Enter is up. The visit keeps `fire`, the field's text, as a search term.

```summon
$ lyncast daemon --trace
$ lyncast toggle
open
$ wait-trace frame1_presented 1
$ keys f i r e Return
$ wait-summon closed
record: ["firefox"]
$ wait-trace window_closed 1
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
$ grep -o '"terms":\["fire"\]' data/lyncast/ranking.json
"terms":["fire"]
```

### Quit from the palette stops the daemon

Quit lyncast, run from the palette, stops the daemon through the same steps as a Quit from a client: the socket goes, the window closes, and the daemon exits 0.

```summon
$ lyncast daemon --trace
$ lyncast toggle
open
$ wait-trace frame1_presented 1
$ keys q u i t Return
$ wait-exit
$ count-trace window_closed
1
$ test -e "$RUNTIME/lyncast.sock" || echo gone
gone
```

### A stdout that does not drain holds up only the trace

stdout is full before the daemon starts, so the trace thread waits in its first write, and every PROBE line after the 64 that wait behind it is dropped. Forty toggles open and close twenty windows, `status` answers, and Quit replies and stops the daemon, which exits 0.

```summon
$ stall-stdout
$ lyncast daemon --trace
$ lyncast toggle
open
$ for i in $(seq 40); do XDG_RUNTIME_DIR=$RUNTIME $LYNCAST toggle; done | sort | uniq -c | awk '{print $2, $1}'
closed 20
open 20
$ lyncast status
open
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
```

Once the test reads what fills stdout, the line the thread held goes through, then the 64 lines that waited, and the next line, the 66th, counts the dropped ones. The lines after it go through as they come.

```summon
$ stall-stdout
$ lyncast daemon --trace
$ for i in $(seq 41); do XDG_RUNTIME_DIR=$RUNTIME $LYNCAST toggle; done | sort | uniq -c | awk '{print $2, $1}'
closed 20
open 21
$ drain-stdout
$ lyncast toggle
closed
$ wait-trace window_closed 21
$ count-trace app_ready
1
$ grep -n '^PROBE dropped=[1-9][0-9]*$' $RUNTIME/daemon.stdout | cut -d: -f1
66
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
```

Here the test holds the trace thread still once it has written the first summon's lines, so Quit's `window_closed` waits in the queue, and lets the thread go half a second after Quit has replied. The line is on stdout once the daemon has exited, so the daemon waited for it.

```summon
$ lyncast daemon --trace
$ lyncast toggle
open
$ wait-trace frame1_done 1
$ freeze-thread trace
$ lyncast activate cmd:quit
ran cmd:quit
$ thaw-thread 500ms
$ wait-exit
$ count-trace window_closed
1
```

Here stdout is full before the daemon starts and no window opens, so when Quit comes, the thread is still writing `app_ready` and no line waits behind it. The test reads what fills stdout half a second after Quit has replied, inside the daemon's 1 s wait. `app_ready` is on stdout once the daemon has exited, so the daemon waited for the line it was writing too.

```summon
$ stall-stdout
$ lyncast daemon --trace
$ lyncast activate cmd:quit
ran cmd:quit
$ sleep 0.5
$ drain-stdout
$ wait-exit
$ wait-trace app_ready 0
```

### The window's size and margins

The `geometry` block's first line is the window's size in logical pixels. Each line after it is a display's height and the top margin a window gets there. anvil's display under `scripts/test-env.sh` is 800 pixels tall.

```geometry
window 750x480
800 => 106
1080 => 200
1440 => 320
481 => 0
480 => 0
400 => 0
```

## GPUI comes from the lock that bench/probe measured

The binary depends on `gpui-pre` and `gpui-pre-platform`, both pinned to `=0.3.9`. The platform crate has its default features off and only `font-kit`, `wayland`, and `runtime_shaders` on, as in `bench/probe`, so the binary builds no X11 backend. In code the two crates are `gpui` and `gpui_platform`.

`Cargo.lock` took its GPUI packages from `bench/probe/Cargo.lock`, the lockfile that [notes/05](../research/notes/05-probe-summon-latency.md) measured, so every GPUI crate keeps the version the probe ran. The two locks were merged, and one `cargo fetch` then added the 22 crates that gpui's `test-support` feature needs. It changed no version that either lock held, and the same steps gave the same lock twice. Every build in `scripts/check.sh` runs with `--locked`.

The binary's dev-dependency on `gpui-pre` turns on `test-support`. Cargo builds each crate once with every feature that any target asks for, so `cargo test` builds the `lyncast` binary itself with `test-support`, and the Tier 2 tests run that build. Outside a test the build schedules work as a release build does: `test-support` adds the test platform, and `BackgroundExecutor::new` uses the test scheduler only for a test dispatcher (`gpui-pre` 0.3.9 `src/executor.rs:69-81`, read in source).

GPUI runs in the binary's own unit tests. `palette::tests::a_window_opens_and_closes` opens a window with the palette's options on GPUI's test platform, closes it, and sees GPUI's close observers run.
