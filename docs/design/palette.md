# The palette is one window per summon

This page describes the palette, lyncast's GPUI window, for a maintainer and for the tests. The [decision log](../decisions.md#palette) is the source of truth. Where this page disagrees with it, the log wins.

## lyncast daemon runs the palette

`lyncast daemon` runs the daemon with the palette, and `lyncast daemon --headless` runs it without, as [daemon.md](daemon.md#start-the-daemon-with-lyncast-daemon) says. The palette needs a Wayland compositor. Without `WAYLAND_DISPLAY`, `lyncast daemon` prints `lyncast: not started: the palette needs a Wayland compositor; run lyncast daemon --headless without one` and exits 1 before it takes the lock. GPUI's platform, which lyncast allows only Wayland, would otherwise panic once the daemon held the lock (`gpui-pre-linux` 0.3.9 `src/linux/platform.rs:369-373`, read in source).

The daemon starts as the headless one does: the lock, the scan, the visits, the clips, the capture thread, and the socket, in daemon.md's order. Then GPUI's app runs, with `QuitMode::Explicit`, so the process stays when the last window closes and keeps the clipboard selection it serves. A client that connects before GPUI runs waits in the daemon's channel until GPUI takes it.

The daemon lives on GPUI's main thread. One task there, the owner task, hands it every event that the connection threads, the workers, the copier, the capture thread, and the writers queue, and each of them rings a bell that wakes the task once its event is queued. The palette's own requests go to the daemon through `Host::ask`, on the same thread, and get the daemon's typed answer back. Reads that change nothing, such as an app's icon or the clipboard history, go to the daemon with no request. The root list's query is the one read that goes through `Host::ask`, as a `query` request, so `Daemon::handle` gives the palette the rows it gives a socket client's `query`. Quit runs through the same `Owner` as the headless host's, in `Owner::stop`, `Stopping::wait`, `Owner::drain`, and `Owner::finish_blocking`: the socket goes, the work in flight and the last writes get 5 s, then the reply.

## A summon opens one window

The palette host holds the summon as a window. A socket client's answer changes it by daemon.md's rules: `toggle` opens a window on the root list when the summon is closed and closes it when it is open, a success closes it, and Clipboard History opens a window on the clipboard screen, or shows that screen in the open window. A failure leaves it. A Clipboard History that comes while a window is still opening, before GPUI has drawn it, opens that window on the clipboard screen.

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

A success closes the window once the activating key is up and no modifier is held, in whichever order the release and the modifiers' change come. Under Wayland they come as separate events (`gpui-pre-linux` 0.3.9 `src/linux/wayland/client.rs:2115-2140, 2258-2270`, read in source). The release of another key, or of an Enter the window never saw go down, closes nothing. niri's bind leaves such a release as the window maps. A success sent by a socket client closes the window at once, as daemon.md's rules say. Clipboard History's success shows the clipboard screen instead, and closes nothing.

A failure leaves the window open and shows its reason in the footer until the next key that edits the field or moves the selection. Log Out, Reboot, and Power Off ask first: the daemon answers with a confirmation, and the footer asks `Reboot?` with Enter to run and Escape to cancel. Enter there sends the same activation again with `confirmed`. A key that moves the selection or edits the field, and typed text, cancel the question.

Losing the keyboard focus closes the window, as TinyCast hides on focus loss ([notes/06](../research/notes/06-tinycast-behaviour-spec.md)), but only once the window has been active. The view tells the model whether the window is active as it opens and at each change. Under anvil the window turns active only with the first key that reaches it (a GPUI probe under anvil, 2026-10-17), and GPUI's test platform opens a window inactive (`gpui-pre` 0.3.9 `src/platform/test/window.rs:414-425`, read in source).

## Ctrl+K opens the Actions menu

Ctrl+K opens the selected row's Actions menu over the bottom right of the list, as [architecture.md](architecture.md#ctrlk-opens-the-actions-menu) describes it. The menu lists the row's items from its chord table, default first with the Enter key beside it. While the menu is open, the keys and text go to it. Up and Down move its selection, which stops at both ends. Text goes to the menu's own field, whose placeholder reads `Search for actions`, and the menu keeps the items whose folded label holds the folded text, as ranking folds a query. The menu's field moves its text and clips it as the root field does, and `the_caret_stays_in_the_field` composes and commits 1,024 bytes there too. A new filter selects the first item it keeps.

Opening the menu ends the root field's composition, and the composing text stays as typed. Enter or Ctrl+Enter sends `activate` with the selected item's verb, and the activation goes on as one sent by Enter does: the window closes after a success, a failure shows in the footer, and a command that asks first asks. Escape, Ctrl+K, and Backspace in an empty filter close the menu, and the repeats of a held Ctrl+K or Backspace do not. Opening and closing the menu leave a failure's reason in the footer. The root field keeps its text throughout, and the footer names the menu's selected item while no failure shows there.

## Tab opens the clipboard screen

Tab on the root list opens the clipboard screen, which lists the clipboard history, and the field's text stays as the screen's filter. Tab there goes back to the root list, with the text. The Clipboard History command opens the clipboard screen too, with an empty field, from the palette or from `lyncast activate cmd:clipboard-history`. The screen has its own placeholder, `Search clipboard history`, and the type filter's name beside the field. The field there ends where the type filter's name begins, and it moves and clips its text as the root field does, which `the_caret_stays_in_the_field` checks there too.

Escape takes one step back at a time, as TinyCast's palette does ([notes/06](../research/notes/06-tinycast-behaviour-spec.md)): it closes the Actions menu or a question first, then clears a field that holds text, then goes from a clipboard screen that came from the root list back to the root list, and then closes the palette. A summon that opened on the clipboard screen has no root list behind it, so Escape closes it from there, and Tab turns it into the root list. Backspace in an empty field goes back the same way and never closes the palette. A held Backspace's repeats only edit, so a held key that empties the filter stays on the screen. Tab's repeats do nothing. A failure's reason stays in the footer through Tab, Ctrl+T, and a step back that clears no text, also with the Actions menu open, until a key edits the field or moves the selection.

The list holds every clip in the history's order: the pins first, under `Pinned`, in the order they were pinned, then the rest by their last use, newest first, under the day of that use. A day is `Today`, `Yesterday`, or a date such as `Oct 15`, with its year when it is not this year, such as `Oct 15, 2025`, on the daemon's local wall clock, which `TZ` sets. Each row shows a square with its kind's letter and its title. A text's title is its first line that holds more than whitespace, trimmed, at most 200 characters. A file list's is its first file's name, with `+N` for the other paths its preview holds. An image's is `Image`.

The selection lands on the newest clip that is not pinned, or on the first when every clip is pinned, as TinyCast's does, each time the filter or the type filter changes. Up and Down move it, and it stops at both ends. The field's text filters the list: a clip stays when its preview holds every word of the text, each folded as ranking folds a query. An image has no preview, so only an empty field keeps it. Ctrl+T moves the type filter on, from All Types to Text Only, Images Only, Files Only, Links Only, Emails Only, and back to All Types. Text Only keeps links and emails too, because they are text. The root list has no type filter, so Ctrl+T does nothing there, and its repeats do nothing anywhere.

The list scrolls the selected clip into view, with its day's header or `Pinned` when the clip is the first under it, as the root list does with a section's header. A summon that opens on the clipboard screen lands on the newest clip that is not pinned, which many pins can push below the window's first view, so the list scrolls to it once the window has been laid out. Each line is as tall as the theme says, so the scroll needs no line that the list has measured.

The zone that the wall clock reads is the system's, which glibc loads at the first `localtime_r`, holding its zone lock, and never loads again for `localtime_r` (glibc 2.34 `time/tzset.c:367-374` and `567-577`, `time/localtime.c:28-41`, read in source). That load reads the file that `TZ` or `/etc/localtime` names, and a FIFO there would hold it forever, so the daemon loads the zone on a thread of its own as the palette starts and waits up to 1 s for it. A zone that does not load in time leaves the palette on UTC, with the warning `lyncast: the time zone did not load within 1 s, so the palette shows times in UTC`, and the palette never calls `localtime_r` again, because the thread that waits holds glibc's lock. That covers glibc only. Once an AT-SPI client, such as a screen reader, turns on a window's accessibility tree, GPUI reads the local time through chrono on the main thread at each frame (`gpui-pre` 0.3.9 `src/window.rs:3779-3788`, `src/window/a11y.rs:285`, and `src/window/a11y/debug.rs:84`), and chrono opens the file that `TZ` names itself (`chrono` 0.4.45 `src/offset/local/unix.rs:89-97` and `src/offset/local/tz_info/timezone.rs:37-57`). So with such a client, a FIFO there would still hold the main thread. These were read in source. `every_zone_reads_as_the_doc_says` in `crates/lyncast/src/palette/tests.rs` runs itself again for each line of the `zone` block, with only `TZ` and the test's own marker, `LYNCAST_ZONE_CHILD`, set, and reads the wall time of 2026-10-17 at 10:00 UTC there. `$FIFO` is a FIFO that no one writes.

```zone
XYZ-5:30 => 15:30
UTC0 => 10:00
:$FIFO => 10:00
```

The screen reads the history again each time the daemon takes an event that is not a read, such as a capture, so a clip copied in another app shows at once, and the selection stays on the clip it was on. The list keeps its scroll position across that read, so a row that stays where it was in the list stays where it was in the window. A selected clip that the read pushes past the list's edge scrolls back into view. `Owner::hand`, which every event goes through, counts those events, and the host tells the window when the count moves.

Enter copies the selected clip and closes the palette once Enter is up, as any success does. Ctrl+K's menu holds Copy, then Pin or Unpin, then Delete. Pin, Unpin, and Delete close the palette too, as every other success does.

Tab and Ctrl+T do nothing while the menu is open. Like any other key, Tab and Ctrl+T cancel a question.

### The preview pane and the information panel

The pane beside the list shows the selected clip. A text shows its preview, its first 1,024 bytes. An image decodes on GPUI's background executor from the bytes the daemon holds, read as a copy reads them, and the pane shows it scaled down to fit, with its size under it. Its size is read first, and an image of more than 32 million pixels shows its size and no pixels. A file list whose first file's name ends in `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, or `.bmp`, in any case, shows that file as an image, read as icons are read: never a FIFO or a device, and at most 25 MiB, as many bytes as a clip holds. An image larger than 840 by 480 pixels, the pane at twice the scale, is scaled down to fit that before it is drawn. Any other file list shows a badge with its first file's extension, and its paths.

The pane holds one image at a time: once the selection moves, the old image leaves the window's atlas. One load is out at a time, so at most one image decodes, whatever the rate at which the selection moves. While a load is out, the newest selected image waits for it, and a load whose clip is no longer selected once its image's size is read stops there and decodes nothing. The palette host holds the preview and its one load, not the window, so these hold across summons: a window that closes with a load out leaves the load to go on, and a summon that lands on the clip it loaded shows that image with no new load. `the_preview_holds_one_image` in `crates/lyncast/src/palette/tests.rs` moves the selection through the `clips atlas` block's clips, from one image to another and then to a text, and checks the window's atlas after each move.

```clips atlas
text gamma 2026-10-17T08:00
image 4x4 2026-10-17T09:00
image 8x8 2026-10-17T10:00
```

```clips loads
text notes 2026-10-17T06:00
image 4x4 2026-10-17T07:00
image 5x5 2026-10-17T08:00
image 6x6 2026-10-17T09:00
image 7x7 2026-10-17T10:00
```

In the block below, `loads:` counts the loads started, those that stopped, those that decoded, and the most out at once, over every window the block opens. Three Downs before any load runs start two loads, and the first stops before it decodes, so the pane shows the last image selected. A selection that leaves a clip whose image has decoded and comes back before the image lands starts no second load for it. Two Ups and an Escape close the window with a load out and an image waiting for it, and both go on, so the next summon lands on the image that waited and starts no load. The image of a load that lands once its clip is no longer selected is dropped, and the newest selected image loads next. A selection that moves to a text, or to the root list, drops the image that waited, so it never loads. One that moves there before the load out has read its image's size stops that load, so it decodes nothing.

```palette clips=loads counts
$ history
clipboard (Search clipboard history) All Types
== Today
> image: Image
  image: Image
  image: Image
  image: Image
  text: notes
preview: image 7x7 drawn 7x7
info: Source — · Type Image · Size 146 B · Copied Today at 10:00
footer: Copy (enter) · Actions (ctrl-k)
loads: 1 started, 0 stopped, 1 decoded, at most 1 out
$ press down down down
clipboard (Search clipboard history) All Types
== Today
  image: Image
  image: Image
  image: Image
> image: Image
  text: notes
preview: image 4x4 drawn 4x4
info: Source — · Type Image · Size 130 B · Copied Today at 07:00
footer: Copy (enter) · Actions (ctrl-k)
loads: 3 started, 1 stopped, 2 decoded, at most 1 out
$ dispatch up + decode + dispatch up down
clipboard (Search clipboard history) All Types
== Today
  image: Image
  image: Image
> image: Image
  image: Image
  text: notes
preview: image 5x5 drawn 5x5
info: Source — · Type Image · Size 138 B · Copied Today at 08:00
footer: Copy (enter) · Actions (ctrl-k)
loads: 4 started, 1 stopped, 3 decoded, at most 1 out
$ press up up escape
closed
loads: 6 started, 2 stopped, 4 decoded, at most 1 out
$ history
clipboard (Search clipboard history) All Types
== Today
> image: Image
  image: Image
  image: Image
  image: Image
  text: notes
preview: image 7x7 drawn 7x7
info: Source — · Type Image · Size 146 B · Copied Today at 10:00
footer: Copy (enter) · Actions (ctrl-k)
loads: 6 started, 2 stopped, 4 decoded, at most 1 out
$ dispatch down + decode + dispatch down
clipboard (Search clipboard history) All Types
== Today
  image: Image
  image: Image
> image: Image
  image: Image
  text: notes
preview: image 5x5 drawn 5x5
info: Source — · Type Image · Size 138 B · Copied Today at 08:00
footer: Copy (enter) · Actions (ctrl-k)
loads: 8 started, 2 stopped, 6 decoded, at most 1 out
$ dispatch up + decode + dispatch down down down
clipboard (Search clipboard history) All Types
== Today
  image: Image
  image: Image
  image: Image
  image: Image
> text: notes
preview: text "notes"
info: Source — · Type Text · Size 5 B · Copied Today at 06:00
footer: Copy (enter) · Actions (ctrl-k)
loads: 9 started, 2 stopped, 7 decoded, at most 1 out
$ dispatch up + dispatch down
clipboard (Search clipboard history) All Types
== Today
  image: Image
  image: Image
  image: Image
  image: Image
> text: notes
preview: text "notes"
info: Source — · Type Text · Size 5 B · Copied Today at 06:00
footer: Copy (enter) · Actions (ctrl-k)
loads: 10 started, 3 stopped, 7 decoded, at most 1 out
$ dispatch up + decode + dispatch up + dispatch tab
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
footer: Run (enter) · Actions (ctrl-k)
loads: 11 started, 3 stopped, 8 decoded, at most 1 out
$ dispatch tab + dispatch tab
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
footer: Run (enter) · Actions (ctrl-k)
loads: 12 started, 4 stopped, 8 decoded, at most 1 out
```

No preview goes through GPUI's image loader, which reads a path with `fs::read` and so waits on a FIFO (`gpui-pre` 0.3.9 `src/elements/img.rs:634`, read in source).

The information panel under the pane gives the clip's Source, Type, Path for a file list, Size, and Copied. Source shows `—` until plan step 8 brings the focused app from niri. Type is Text, Link, Email, Image, or Files. Path is the first file's path. Size is in bytes under 1,000, then in kB and MB of 1,000, with one decimal. Copied is the day and time of the clip's last use, which a capture, a copy, a pin, and an unpin set, because the daemon keeps no other time of a clip in memory.

## Rows show their app's icon

Each app row and command row starts with an icon 24 logical pixels wide. An app's icon comes from the `Icon=` key of its desktop entry. A command, and an app whose icon does not load, shows a rounded square with the first letter of its title instead. The list asks for an icon only as it lays out the icon's row, so a row it has not laid out reads nothing. It lays out the rows on screen and those within `theme::OVERDRAW`, 200 logical pixels, past its edges.

An `Icon=` value that starts with `/` names the file to read. Any other value names an icon, which lyncast looks up in two places:

1. The hicolor theme, at the sizes 48x48, scalable, 64x64, 128x128, 256x256, 512x512, 32x32, 24x24, and 16x16, in that order. At each size it tries `$XDG_DATA_HOME/icons`, `$HOME/.icons`, and each `XDG_DATA_DIRS` entry's `icons`, in that order, before it tries the next size. A scalable icon is `hicolor/scalable/apps/<name>.svg`, and any other is `hicolor/<size>/apps/<name>.png`.
2. Each `XDG_DATA_DIRS` entry's `pixmaps`, as `<name>.png` and then as `<name>.svg`.

lyncast has no icon-theme setting yet, so an icon that only a theme such as Adwaita holds does not show.

The first file that reads and decodes is the icon. lyncast reads each file as the app scan reads a desktop entry: it skips a FIFO, a device, and a file over 1 MiB, and it never opens them. A file whose name ends in `.svg` is an SVG. Any other file decodes by its content, as a PNG or any other format the `image` crate reads, if it is at most 1,024 pixels on each side. An SVG is drawn 48 pixels on its longer side. A raster icon larger than that is scaled down to fit, and a smaller one keeps its size. An icon that draws no pixel at all counts as no icon, and the lookup goes on to the next file.

lyncast draws an SVG with resvg itself, with no fonts and no way to resolve an image. An `<image>` or `<feImage>` element draws nothing, whether it names a file or holds the image's data, and text draws nothing either. GPUI's SVG renderer parses with usvg's default options, whose resolver reads the path that an `<image>` or a `<feImage>` names with `fs::read` (`usvg` 0.46.0 `src/parser/image.rs:85-110` and `src/parser/filter.rs:860`), and that read waits on a FIFO. usvg unzips a gzipped SVG before it parses it (`src/parser/mod.rs:98-106`), so a check of the file's bytes for `<image` would miss both cases. GPUI's image loader reads a path with `fs::read` too (`gpui-pre` 0.3.9 `src/elements/img.rs:634`), so lyncast hands GPUI decoded images only. All of these were read in source.

Icons load on GPUI's background executor, and a row draws its icon once it has loaded. The palette host keeps the 128 icons that rows asked for most recently, loaded or not, so the next summon draws the icons it holds at once. 128 icons of 48 by 48 pixels hold 1.1 MiB of pixels. An icon that leaves the cache leaves the atlas of every window too, so a long scroll keeps no more icons in a window's atlas than the cache holds.

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

These tests read the blocks below. The Tier 3 runner in `crates/lyncast/src/palette/tests.rs` reads the `desktop`, `clips`, and `palette` blocks. There too, `the_preview_holds_one_image` reads a `clips` block, `every_zone_reads_as_the_doc_says` reads the `zone` block, and `the_window_is_placed_as_the_geometry_block_says` reads the `geometry` block, which `the_geometry_is_as_the_doc_says` in `crates/lyncast/src/palette/theme.rs` reads too. `every_size_reads_as_the_doc_says` in `crates/lyncast/src/palette/clips.rs` reads the `sizes` block, and `every_icon_loads_as_the_doc_says` in `crates/lyncast/src/palette/icons.rs` reads the `env`, `icon`, and `icons` blocks. The test in `crates/lyncast/tests/palette.rs` reads the `env`, `desktop`, `executable`, and `summon` blocks, and runs each `summon` block as [clipboard-and-paste.md's](clipboard-and-paste.md#the-fixtures) tests run a `clipboard` block: in a fresh `$TREE`, inside its own `scripts/test-env.sh start` environment, whose `XDG_RUNTIME_DIR` is `$RUNTIME`, with only the `env` block's variables and the compositor's `WAYLAND_DISPLAY`. `VK_DRIVER_FILES` selects lavapipe, as `scripts/bench.sh` does.

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

`every_palette_block_runs_as_the_doc_says` in `crates/lyncast/src/palette/tests.rs` runs each `palette` block against a daemon and a palette host of its own, in a GPUI `TestAppContext` app. The daemon has the apps of the `desktop` blocks above, no visits, no data home, and no compositor, so a copy fails with `not copied: no Wayland compositor`. Its runner records each program to a scratch file. Its clock reads 2026-10-17 at 12:00 on a wall two hours east of UTC, for the daemon and the palette alike. Its history keeps every row, so no row expires, and `palette clips=<name>` starts it with the lines of the `clips <name>` block. A block is a list of steps. Each step is a `$ ` line, and the lines after it say what the palette shows once the step has run:

| Step | Does |
|---|---|
| `$ open`, `$ toggle` | sends `toggle` as a socket client sends it |
| `$ history` | sends `activate cmd:clipboard-history` as `lyncast activate` sends it |
| `$ capture <kind> <value>` | hands the daemon a capture as the capture thread does, at the clock's time, with a `clips` line's kind and value |
| `$ type <text>` | GPUI's `simulate_input`, one key press for each char |
| `$ press <keys>` | GPUI's `simulate_keystrokes`, one key press for each keystroke, in GPUI's syntax |
| `$ dispatch <keys>` | GPUI's `dispatch_keystroke` for each keystroke, which runs no task, so with ` + ` the next step comes before the tasks the keys started |
| `$ decode` | runs GPUI's tasks until one starts a background task, then only the background tasks, so the preview's load decodes and its result waits to land |
| `$ insert <text>` | the input handler's `replace_text_in_range`, as an input method's commit calls it. `\n` and `\r` are line breaks |
| `$ mark [<text>]` | the input handler's `replace_and_mark_text_in_range`, as an input method calls it while it composes |
| `$ unmark` | the input handler's `unmark_text` |
| `$ hold <key>` | GPUI's `simulate_event` with a key press marked `is_held`, as a held key repeats |
| `$ release <key>` | `simulate_event` with the key's release |
| `$ mods <none\|ctrl\|shift\|alt>` | `simulate_modifiers_change`. `$ press` changes no modifier, so a block holds Ctrl with `$ mods ctrl` before it presses `ctrl-enter` |
| `$ focus`, `$ blur` | the window turns active, or inactive. A blur acts only on an active window, as GPUI's test platform does |
| `$ stall-runner` | makes the runner's file a FIFO, so the next program waits in the runner and its activation stays out |
| `$ resume-runner` | lets the waiting program write its line, and makes the runner's file a file again |
| `<step> + <step>` | both steps before the wait, so the requests reach the daemon in one drain, before GPUI runs the window's deferred open |

After each step, the runner runs GPUI's tasks until none is ready and the daemon has no job out. While the runner is stalled, it waits for no job. Then it compares the palette's state with the lines after the step, and the programs the step started with the step's `record:` lines. A step with no `record:` line started none. The lines spell that state through the functions that give the view its labels and their order, but a key reads as the steps name it, a clip's kind as `clip list --json` spells it, and the preview pane by what it shows. So the lines do not check the key caps' text, the clips' letters, the pane's wording, or how the view lays the labels out. They show:

- `root "<text>"`, or `root (<placeholder>)` while the placeholder shows. On the clipboard screen, `clipboard "<text>" <type filter>`, or `clipboard (<placeholder>) <type filter>`.
- `== <header>` before each section that has one, and before each day's clips, or the pins'.
- Each row, after `> ` when it is selected and after two spaces when it is not. An app or a command reads `<title>  (<kind>)`. The calculator's card reads `= <answer>  (<expression> · <badge>[ · <words>])`. A clip reads `<kind>: <title>`, with its kind as `clip list --json` spells it.
- `No results` when the root list is empty, and `No clips` when the clipboard screen's is, as the list shows them.
- `preview: <pane>` for the selected clip: `text "<preview>"` or `files "<paths>"`, quoted as Rust quotes a string, `image <w>x<h> drawn <w>x<h>` with the image's size and the size the pane draws it at, `image too large <w>x<h>`, `no image: <why>`, or `loading`. Then `info: ` and the information panel's lines, joined by ` · `.
- `confirm <title>?` while a command asks to be confirmed.
- `menu "<filter>"`, or `menu (<placeholder>)`, while the Actions menu is open, then each item the filter keeps, after `> ` when it is selected and after two spaces when it is not, with `  (enter)` after the default. `No actions` when the filter keeps none.
- The footer: `footer: <default action> (enter) · Actions (ctrl-k)`, or `footer: <title> (enter) · Cancel (escape)` while a command asks, or `footer: pending` while an activation is out, or `footer: closing` after a success while the key is still down, or `footer: failed: <reason>`.
- `closed` once the window has gone, or `stopped` once the daemon's stop has finished too.
- In a block marked `counts`, a last line `loads: <started> started, <stopped> stopped, <decoded> decoded, at most <most> out` with the preview's loads so far. A load counts as decoded once it has begun to decode, on the background executor.

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
footer: Run (enter) · Actions (ctrl-k)
$ type 8x8
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter) · Actions (ctrl-k)
$ press ctrl-n
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press down
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-p
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter) · Actions (ctrl-k)
$ press down up up
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter) · Actions (ctrl-k)
$ press down
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type fox
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press home
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ type fire
root "firefox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press space
root "fire fox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press end backspace backspace backspace backspace
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type fox
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press left left
root "fox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ type i
root "fiox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press right
root "fiox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ insert fire\r\nfox
root "fire fox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ mark f
root "f"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ insert fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type 8x8
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ stall-runner
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ stall-runner
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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

The card's Copy Answer fails without a compositor, and the window stays open with the reason in the footer. A repeat of Enter neither sends again nor clears the reason, and an input method's unmark does not clear it either. The reason stays as the Actions menu opens and closes, by Ctrl+K, Escape, and Backspace, as Tab and Ctrl+T do nothing in the menu, and as Tab goes to the clipboard screen, Ctrl+T moves its type filter on, and Tab comes back. The next key that moves the selection clears it. A held Enter that the palette never saw go down sends nothing, and a repeat of Escape leaves the field's text. Copy Answer fails again, and typed text clears the reason too.

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
footer: Run (enter) · Actions (ctrl-k)
$ type 8x8
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: Copy Answer (enter) · Actions (ctrl-k)
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
$ press ctrl-k
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
menu (Search for actions)
> Copy Answer  (enter)
footer: failed: not copied: no Wayland compositor
$ press ctrl-k
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ press ctrl-k escape
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ press ctrl-k backspace
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ press ctrl-k tab ctrl-t ctrl-k
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ press tab
clipboard "8x8" All Types
No clips
footer: failed: not copied: no Wayland compositor
$ press ctrl-t
clipboard "8x8" Text Only
No clips
footer: failed: not copied: no Wayland compositor
$ press tab
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
footer: Open (enter) · Actions (ctrl-k)
$ hold enter
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ hold escape
root "8x8"
  = 64  (8 × 8 · Product · Sixty-Four)
== Results
> 8x8  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press up enter
root "8x8"
> = 64  (8 × 8 · Product · Sixty-Four)
== Results
  8x8  (Application)
footer: failed: not copied: no Wayland compositor
$ type 0
root "8x80"
> = 640  (8 × 80 · Product · Six Hundred Forty)
footer: Copy Answer (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ mods ctrl
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Open (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ mods ctrl
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type reboot
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ press escape escape
closed
```

### Keys the window did not see go down do nothing

niri's bind leaves the release of Space and a change of the modifiers as the window maps. Neither closes the window, and neither does the release of an Enter the window never saw go down. Clipboard History's success shows the clipboard screen, with no clips here, and the release of its Enter closes nothing.

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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ press enter
clipboard (Search clipboard history) All Types
No clips
$ release enter
clipboard (Search clipboard history) All Types
No clips
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ type quit
root "quit"
== Results
> Quit lyncast  (Command)
footer: Run (enter) · Actions (ctrl-k)
$ press enter
stopped
```

### Ctrl+K opens the selected row's Actions menu

The menu lists the row's items, Open first with its key, then the Desktop Action New Window, then Copy Path to .desktop. Text typed while the menu is open filters the menu, and the root field keeps its text. Enter runs the menu's selected item, here the Desktop Action.

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
footer: Run (enter) · Actions (ctrl-k)
$ insert fire\nfox
root "fire fox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-k
root "fire fox"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
$ type new
root "fire fox"
== Results
> Firefox  (Application)
menu "new"
> New Window
footer: New Window (enter) · Actions (ctrl-k)
$ press enter
root "fire fox"
== Results
> Firefox  (Application)
footer: closing
record: ["firefox","--new-window"]
$ release enter
closed
```

### The menu's keys

Down and Up move the menu's selection, which stops at both ends. A filter that keeps no item leaves Enter nothing to run. Backspace edits the filter, and in an empty filter it closes the menu. Ctrl+K and Escape close the menu too, and Escape does so before it clears the root field. The repeats of a held Backspace or Ctrl+K close nothing. Copy Path to .desktop fails without a compositor, and its reason shows in the footer. It stays as Ctrl+K opens the menu again, until text in the menu's filter clears it. After the next failure, Down in the menu clears it, and after the one after that, the Escape that clears the root field's text does.

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
footer: Run (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-k down down down
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
  Open  (enter)
  New Window
> Copy Path to .desktop
footer: Copy Path to .desktop (enter) · Actions (ctrl-k)
$ press up
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
  Open  (enter)
> New Window
  Copy Path to .desktop
footer: New Window (enter) · Actions (ctrl-k)
$ type zz
root "fire"
== Results
> Firefox  (Application)
menu "zz"
No actions
$ press enter
root "fire"
== Results
> Firefox  (Application)
menu "zz"
No actions
$ press backspace backspace
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
$ hold backspace
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
$ press backspace
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-k ctrl-k
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-k
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
$ hold ctrl-k
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
$ press escape
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-k up up down down
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
  Open  (enter)
  New Window
> Copy Path to .desktop
footer: Copy Path to .desktop (enter) · Actions (ctrl-k)
$ press enter
root "fire"
== Results
> Firefox  (Application)
footer: failed: not copied: no Wayland compositor
$ press ctrl-k
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: failed: not copied: no Wayland compositor
$ type c
root "fire"
== Results
> Firefox  (Application)
menu "c"
> Copy Path to .desktop
footer: Copy Path to .desktop (enter) · Actions (ctrl-k)
$ press enter
root "fire"
== Results
> Firefox  (Application)
footer: failed: not copied: no Wayland compositor
$ press ctrl-k down
root "fire"
== Results
> Firefox  (Application)
menu (Search for actions)
  Open  (enter)
> New Window
  Copy Path to .desktop
footer: New Window (enter) · Actions (ctrl-k)
$ press down enter
root "fire"
== Results
> Firefox  (Application)
footer: failed: not copied: no Wayland compositor
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
footer: Run (enter) · Actions (ctrl-k)
$ press escape
closed
```

### Opening the menu ends the root field's composition

An input method composes `f` in the root field. Ctrl+K ends the composition and keeps the `f`, so text the field takes after the menu closes goes after it. In the menu, the input method's unmark ends the composition in the menu's filter, and its placeholder shows again.

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
footer: Run (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ mark f
root "firef"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-k escape
root "firef"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ insert ox
root "firefox"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ press ctrl-k
root "firefox"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
$ mark
root "firefox"
== Results
> Firefox  (Application)
menu ""
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
$ unmark
root "firefox"
== Results
> Firefox  (Application)
menu (Search for actions)
> Open  (enter)
  New Window
  Copy Path to .desktop
footer: Open (enter) · Actions (ctrl-k)
```

### A menu pick of a command that asks first

Run, picked from Reboot's menu, asks to be confirmed, and Enter at the question runs Reboot.

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
footer: Run (enter) · Actions (ctrl-k)
$ type reboot
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter) · Actions (ctrl-k)
$ press ctrl-k
root "reboot"
== Results
> Reboot  (Command)
menu (Search for actions)
> Run  (enter)
footer: Run (enter) · Actions (ctrl-k)
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
$ release enter
closed
```

### The clipboard screen's blocks start with a history

A `clips <name>` block is a history for `palette clips=<name>`, or for a test that names it. Each line is a capture at a wall time of the test clock's, in order, so the clips take the ids 1, 2, and so on: `text <text>`, `files <URI list>`, or `image <w>x<h>`, with `\n` for a line break, and the time last. An image is an orange PNG of that size, and one over 32 million pixels is a PNG's header alone, which only the size check reads. `pin <id> <time>` pins a clip. A `$ capture` step takes the same kinds and values, with no time. `$TREE` is a directory of the block's own, with `photo.png`, an orange PNG of 3 by 2, `fifo.png`, a FIFO, and `big.png`, a byte over 25 MiB. Its path is 240 bytes long, so the size of a file list that names a file in it, which counts the list's bytes, reads the same on every machine.

```clips history
image 2x1 2025-10-15T12:00
text a note\nwith two lines 2026-10-15T12:00
files file:///etc/hostname\nfile:///etc/os-release 2026-10-16T18:30
text https://example.org/a 2026-10-17T08:00
text me@example.org 2026-10-17T08:30
text \n  alpha\nbeta 2026-10-17T09:00
pin 4 2026-10-17T10:00
```

### The clipboard screen groups the history by day

Clipboard History from a client opens the summon on the clipboard screen. The pin comes first, then the days. The selection lands on the newest clip that is not pinned, and its preview and its information show beside the list. Up and Down stop at both ends. A summon that opened on the clipboard screen closes on Escape.

```palette clips=history
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press up up
clipboard (Search clipboard history) All Types
== Pinned
> link: https://example.org/a
== Today
  text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "https://example.org/a"
info: Source — · Type Link · Size 21 B · Copied Today at 10:00
footer: Copy (enter) · Actions (ctrl-k)
$ press down down down
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
  text: alpha
  email: me@example.org
== Yesterday
> files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: files "/etc/hostname\n/etc/os-release"
info: Source — · Type Files · Path /etc/hostname · Size 43 B · Copied Yesterday at 18:30
footer: Copy (enter) · Actions (ctrl-k)
$ press down down down
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
  text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
> image: Image
preview: image 2x1 drawn 2x1
info: Source — · Type Image · Size 77 B · Copied Oct 15, 2025 at 12:00
footer: Copy (enter) · Actions (ctrl-k)
$ press escape
closed
```

### Pins come in the order they were pinned

When every clip is pinned, the selection lands on the first. A text's title stops at 200 chars.

```clips pins
text 012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789 2026-10-17T08:00
text two 2026-10-17T09:00
pin 2 2026-10-17T10:00
pin 1 2026-10-17T11:00
```

```palette clips=pins
$ history
clipboard (Search clipboard history) All Types
== Pinned
> text: two
  text: 01234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789
preview: text "two"
info: Source — · Type Text · Size 3 B · Copied Today at 10:00
footer: Copy (enter) · Actions (ctrl-k)
```

### The information panel's sizes

Each line of the `sizes` block is a clip's size in bytes and what the information panel shows for it. A unit test in `crates/lyncast/src/palette/clips.rs` reads it.

```sizes
999 => 999 B
1000 => 1.0 kB
1549 => 1.5 kB
1550 => 1.6 kB
999949 => 999.9 kB
999950 => 1.0 MB
26214400 => 26.2 MB
```

### The filter keeps the clips that hold every word

The field's words filter the clips, and the selection lands on the newest clip that is not pinned. Ctrl+T moves the type filter on, from All Types through each kind and back, and Text Only keeps the link and the email. An image has no preview, so a filter with text never keeps it. Escape clears the field and keeps the type filter. Ctrl+T's repeats do nothing.

```palette clips=history
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ type EXAMPLE
clipboard "EXAMPLE" All Types
== Pinned
  link: https://example.org/a
== Today
> email: me@example.org
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
$ type  me
clipboard "EXAMPLE me" All Types
== Today
> email: me@example.org
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
$ press escape
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press ctrl-t
clipboard (Search clipboard history) Text Only
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Oct 15
  text: a note
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ hold ctrl-t
clipboard (Search clipboard history) Text Only
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Oct 15
  text: a note
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press ctrl-t
clipboard (Search clipboard history) Images Only
== Oct 15, 2025
> image: Image
preview: image 2x1 drawn 2x1
info: Source — · Type Image · Size 77 B · Copied Oct 15, 2025 at 12:00
footer: Copy (enter) · Actions (ctrl-k)
$ type a
clipboard "a" Images Only
No clips
$ press escape ctrl-t
clipboard (Search clipboard history) Files Only
== Yesterday
> files: hostname +1
preview: files "/etc/hostname\n/etc/os-release"
info: Source — · Type Files · Path /etc/hostname · Size 43 B · Copied Yesterday at 18:30
footer: Copy (enter) · Actions (ctrl-k)
$ press ctrl-t
clipboard (Search clipboard history) Links Only
== Pinned
> link: https://example.org/a
preview: text "https://example.org/a"
info: Source — · Type Link · Size 21 B · Copied Today at 10:00
footer: Copy (enter) · Actions (ctrl-k)
$ press ctrl-t
clipboard (Search clipboard history) Emails Only
== Today
> email: me@example.org
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
$ press ctrl-t
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
```

### Tab carries the text, and Escape walks back

Tab takes the field's text to the clipboard screen, where it filters the clips. Escape clears it there, then goes back to the root list, and then closes the window. A copy that fails on the clipboard screen leaves its reason in the footer as Backspace goes back, Tab comes again, and Escape goes back.

```palette clips=history
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
footer: Run (enter) · Actions (ctrl-k)
$ type lines
root "lines"
No results
$ press tab
clipboard "lines" All Types
== Oct 15
> text: a note
preview: text "a note\nwith two lines"
info: Source — · Type Text · Size 21 B · Copied Oct 15 at 12:00
footer: Copy (enter) · Actions (ctrl-k)
$ hold tab
clipboard "lines" All Types
== Oct 15
> text: a note
preview: text "a note\nwith two lines"
info: Source — · Type Text · Size 21 B · Copied Oct 15 at 12:00
footer: Copy (enter) · Actions (ctrl-k)
$ press tab
root "lines"
No results
$ press tab escape
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ press tab
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press enter
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: failed: not copied: no Wayland compositor
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
footer: failed: not copied: no Wayland compositor
$ press tab escape
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
footer: failed: not copied: no Wayland compositor
$ press escape
closed
```

### Clipboard History in the palette shows the clipboard screen

Enter on Clipboard History shows the clipboard screen with an empty field. Backspace in its empty field goes back to the root list, and a held Backspace's repeats only edit. A summon that opened on the clipboard screen becomes the root list on Tab, and Backspace never closes it.

```palette clips=history
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
footer: Run (enter) · Actions (ctrl-k)
$ type clip
root "clip"
== Results
> Clipboard History  (Command)
footer: Run (enter) · Actions (ctrl-k)
$ press enter
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ release enter
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ type qz
clipboard "qz" All Types
No clips
$ hold backspace
clipboard "q" All Types
No clips
$ hold backspace
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ hold backspace
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ press escape
closed
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press backspace
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press tab
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
footer: Run (enter) · Actions (ctrl-k)
$ press escape
closed
```

### Clipboard History from a client shows the clipboard screen in the open window

A client's Clipboard History shows the clipboard screen in the open window, with an empty field, and Escape goes back to the root list. One that comes while the window is still opening opens it on the clipboard screen.

```palette clips=history
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
footer: Run (enter) · Actions (ctrl-k)
$ type fire
root "fire"
== Results
> Firefox  (Application)
footer: Open (enter) · Actions (ctrl-k)
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
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
footer: Run (enter) · Actions (ctrl-k)
$ toggle
closed
$ toggle + history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press escape
closed
```

### Enter copies a clip, and its menu pins, unpins, and deletes

Enter copies the selected clip, which fails here with no compositor, and the reason stays in the footer as the menu opens. The menu's Pin, Unpin, and Delete close the window once Enter is up, as every success does, and the screen shows each change as soon as the daemon has made it, while Enter is still down. A pin goes after the pins before it, and an unpinned clip keeps the place its pin gave it among the rest, with its unpin as its last use. Once the selected clip is deleted, the selection stays where it was, on the clip that takes its place.

```palette clips=history
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press enter
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: failed: not copied: no Wayland compositor
$ press ctrl-k
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
menu (Search for actions)
> Copy  (enter)
  Pin
  Delete
footer: failed: not copied: no Wayland compositor
$ press down enter
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
> text: alpha
== Today
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 12:00
footer: closing
$ release enter
closed
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
  text: alpha
== Today
> email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
$ press up ctrl-k
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
> text: alpha
== Today
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 12:00
menu (Search for actions)
> Copy  (enter)
  Unpin
  Delete
footer: Copy (enter) · Actions (ctrl-k)
$ type un
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
> text: alpha
== Today
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 12:00
menu "un"
> Unpin
footer: Unpin (enter) · Actions (ctrl-k)
$ press enter
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 12:00
footer: closing
$ release enter
closed
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 12:00
footer: Copy (enter) · Actions (ctrl-k)
$ press ctrl-k
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 12:00
menu (Search for actions)
> Copy  (enter)
  Pin
  Delete
footer: Copy (enter) · Actions (ctrl-k)
$ press down down enter
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: closing
$ release enter
closed
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
```

### A capture shows at once, and the selection stays

The screen reads the history again when the daemon takes a capture. The new clip leads Today, and the selection stays on the clip it was on.

```palette clips=history
$ history
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> text: alpha
  email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "\n  alpha\nbeta"
info: Source — · Type Text · Size 13 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
  text: alpha
> email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
$ capture text gamma
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
  text: gamma
  text: alpha
> email: me@example.org
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
$ capture text me@example.org
clipboard (Search clipboard history) All Types
== Pinned
  link: https://example.org/a
== Today
> email: me@example.org
  text: gamma
  text: alpha
== Yesterday
  files: hostname +1
== Oct 15
  text: a note
== Oct 15, 2025
  image: Image
preview: text "me@example.org"
info: Source — · Type Email · Size 14 B · Copied Today at 12:00
footer: Copy (enter) · Actions (ctrl-k)
```

### The preview shows images, and file lists as images or paths

An image over 32 million pixels shows its size, and an image file that a file list names first shows as an image. A FIFO there is never opened, and a file over 25 MiB is never read. A file that is gone shows why, and any other file shows its paths. An image wider or taller than the pane is drawn scaled down to fit it: 1680 by 200 pixels at 840 by 100, and 200 by 1000 at 96 by 480.

```clips previews
image 200x1000 2026-10-17T07:00
image 1680x200 2026-10-17T07:30
image 6000x6000 2026-10-17T08:00
files file://$TREE/big.png 2026-10-17T08:30
files file://$TREE/photo.png 2026-10-17T09:00
files file://$TREE/fifo.png 2026-10-17T10:00
files file://$TREE/gone.PNG 2026-10-17T11:00
files file://$TREE/notes.txt 2026-10-17T11:30
```

```palette clips=previews
$ history
clipboard (Search clipboard history) All Types
== Today
> files: notes.txt
  files: gone.PNG
  files: fifo.png
  files: photo.png
  files: big.png
  image: Image
  image: Image
  image: Image
preview: files "$TREE/notes.txt"
info: Source — · Type Files · Path $TREE/notes.txt · Size 257 B · Copied Today at 11:30
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Today
  files: notes.txt
> files: gone.PNG
  files: fifo.png
  files: photo.png
  files: big.png
  image: Image
  image: Image
  image: Image
preview: no image: $TREE/gone.PNG: No such file or directory (os error 2)
info: Source — · Type Files · Path $TREE/gone.PNG · Size 256 B · Copied Today at 11:00
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Today
  files: notes.txt
  files: gone.PNG
> files: fifo.png
  files: photo.png
  files: big.png
  image: Image
  image: Image
  image: Image
preview: no image: $TREE/fifo.png: not a regular file
info: Source — · Type Files · Path $TREE/fifo.png · Size 256 B · Copied Today at 10:00
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Today
  files: notes.txt
  files: gone.PNG
  files: fifo.png
> files: photo.png
  files: big.png
  image: Image
  image: Image
  image: Image
preview: image 3x2 drawn 3x2
info: Source — · Type Files · Path $TREE/photo.png · Size 257 B · Copied Today at 09:00
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Today
  files: notes.txt
  files: gone.PNG
  files: fifo.png
  files: photo.png
> files: big.png
  image: Image
  image: Image
  image: Image
preview: no image: $TREE/big.png: over 25 MiB
info: Source — · Type Files · Path $TREE/big.png · Size 255 B · Copied Today at 08:30
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Today
  files: notes.txt
  files: gone.PNG
  files: fifo.png
  files: photo.png
  files: big.png
> image: Image
  image: Image
  image: Image
preview: image too large 6000x6000
info: Source — · Type Image · Size 65 B · Copied Today at 08:00
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Today
  files: notes.txt
  files: gone.PNG
  files: fifo.png
  files: photo.png
  files: big.png
  image: Image
> image: Image
  image: Image
preview: image 1680x200 drawn 840x100
info: Source — · Type Image · Size 7.1 kB · Copied Today at 07:30
footer: Copy (enter) · Actions (ctrl-k)
$ press down
clipboard (Search clipboard history) All Types
== Today
  files: notes.txt
  files: gone.PNG
  files: fifo.png
  files: photo.png
  files: big.png
  image: Image
  image: Image
> image: Image
preview: image 200x1000 drawn 96x480
info: Source — · Type Image · Size 6.6 kB · Copied Today at 07:00
footer: Copy (enter) · Actions (ctrl-k)
```

### Tab and Ctrl+T end a question, and do nothing in the menu

A question ends at Ctrl+T and at Tab, as at any key that moves or edits, and on the root list Ctrl+T does nothing else. While the menu is open, Tab and Ctrl+T do nothing.

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
footer: Run (enter) · Actions (ctrl-k)
$ type reboot
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter) · Actions (ctrl-k)
$ press enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ press ctrl-t
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter) · Actions (ctrl-k)
$ press enter
root "reboot"
== Results
> Reboot  (Command)
confirm Reboot?
footer: Reboot (enter) · Cancel (escape)
$ press tab
clipboard "reboot" All Types
No clips
$ press tab
root "reboot"
== Results
> Reboot  (Command)
footer: Run (enter) · Actions (ctrl-k)
$ press ctrl-k tab ctrl-t
root "reboot"
== Results
> Reboot  (Command)
menu (Search for actions)
> Run  (enter)
footer: Run (enter) · Actions (ctrl-k)
```

### The summon blocks run under anvil

A `summon` block's `lyncast daemon` line has no `--headless`. Its daemon's stdout is `$RUNTIME/daemon.stdout`, as daemon.md's harness appends it. A block may use clipboard-and-paste.md's lines, such as `seed`, `paste`, `wait-exit`, `freeze-thread`, and `stop-compositor`, and these:

- `$ wait-trace <event> <seq>` waits until `$RUNTIME/daemon.stdout` holds that summon's line for the event.
- `$ count-trace <event>` prints how many lines for the event it holds.
- `$ keys <key>...` types keys into the compositor with `python3 -I scripts/x11.py <display> focus keys <key>...`, on the X display that `$RUNTIME/display` names. Each key is an X keysym name, such as `f` or `Return`, and `ctrl+v` holds Ctrl while it taps `v`. The script gives the X input focus to anvil's window, sends the keys through XTEST, and returns once the X server has handled them.
- `$ wait-summon <open|closed>` runs `lyncast status` until it prints that state.
- `$ stall-stdout` gives the next daemon a stdout that does not drain: a pipe that the test fills before the daemon starts, in place of `$RUNTIME/daemon.stdout`. `$ drain-stdout` reads what fills it, and from then on what the daemon writes to stdout goes on to `$RUNTIME/daemon.stdout`.
- `$ shot <name> [<color>...]` reads the screen, which Xvfb keeps in `$RUNTIME/Xvfb_screen0`, until it shows a box of pixels that differ from the top-left pixel, the compositor's background, in which each `<color>` covers at least 1% of the box, and two reads in a row agree. The colors let a block wait for what the window draws after its first frame, such as an image that loads. It prints `palette at <x>,<y> <w>x<h>` for the box, `edge <color>` when every pixel on the box's edge has that color, or `edge mixed`, and `colors` with each color that covers at least 1% of the box, most first. A color is six hex digits. It writes the screen to `target/tmp/palette-shots/<block line>-<name>.png`.

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

### A zone file that never answers leaves the palette on UTC

`TZ` names a FIFO that no one writes, so the zone's load waits on it forever. The start waits 1 s for the zone, warns, and goes on with UTC. Clipboard History opens the clipboard screen and draws it, `status` answers, and Quit stops the daemon.

```summon
$ mkfifo zone.fifo
$ TZ=:$TREE/zone.fifo lyncast daemon --trace
$ lyncast activate cmd:clipboard-history
ran cmd:clipboard-history
$ wait-trace frame1_presented 1
$ lyncast status
open
$ wait-stderr lyncast: the time zone did not load within 1 s, so the palette shows times in UTC
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
stderr: lyncast: the time zone did not load within 1 s, so the palette shows times in UTC
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

### The icon blocks load without a display

`every_icon_loads_as_the_doc_says` makes a tree from the `icon` blocks and loads each line of the `icons` block, with the `env` block's `XDG_DATA_HOME`, `XDG_DATA_DIRS`, and `HOME`. An `icon <path>` block makes the file at `<path>` under `$TREE`:

- `png <w>x<h>` writes an opaque PNG of that size, and `png <w>x<h> <bytes>` pads the file with zeros to that many bytes.
- `fifo` makes a FIFO, and a program writes a byte to it each time something opens it to read.
- Any other line is the file's text, with `$TREE` replaced.

Each line of the `icons` block is an `Icon=` value, `=>`, and then the file the icon loaded from and the icon's size in pixels, or nothing when no icon loads. The test also fails when a load opened a FIFO, also through an SVG's `<image>` or `<feImage>`.

Firefox's hicolor icon wins over its pixmap. A 48x48 icon wins over a scalable one, and a scalable one over a larger one. A size comes before a root, so `deep`'s 48x48 icon in `XDG_DATA_DIRS` wins over its 256x256 one in `XDG_DATA_HOME`, and at one size `XDG_DATA_HOME` comes first, then `~/.icons`. `big` and `electron` are scaled down to 48 pixels on their longer side, `term` keeps its 32 pixels, and the 16 by 8 `sketch` is drawn 48 by 24. `huge` is over 1,024 pixels on a side, and `heavy` is over 1 MiB. `piped`'s 48x48 file is a FIFO, so its scalable one loads. `linked`'s SVG holds only an `<image>`, so it draws nothing, and its 64x64 PNG loads. `filtered` holds only a `<feImage>`, and `lettered` only text.

```icon share/icons/hicolor/48x48/apps/firefox.png
png 48x48
```

```icon share/pixmaps/firefox.png
png 32x32
```

```icon share/icons/hicolor/scalable/apps/editor.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#5e8bff"/></svg>
```

```icon share/icons/hicolor/48x48/apps/both.png
png 48x48
```

```icon share/icons/hicolor/scalable/apps/both.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#5e8bff"/></svg>
```

```icon share/icons/hicolor/256x256/apps/big.png
png 256x128
```

```icon data/icons/hicolor/256x256/apps/deep.png
png 256x256
```

```icon share/icons/hicolor/48x48/apps/deep.png
png 48x48
```

```icon data/icons/hicolor/48x48/apps/mine.png
png 48x48
```

```icon home/.icons/hicolor/48x48/apps/mine.png
png 48x48
```

```icon share/icons/hicolor/48x48/apps/mine.png
png 48x48
```

```icon home/.icons/hicolor/48x48/apps/old.png
png 48x48
```

```icon share/icons/hicolor/48x48/apps/old.png
png 48x48
```

```icon share/pixmaps/term.png
png 32x32
```

```icon share/pixmaps/term.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#5e8bff"/></svg>
```

```icon share/pixmaps/sketch.svg
<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><rect width="16" height="8" fill="#5e8bff"/></svg>
```

```icon share/pixmaps/electron.png
png 1024x1024
```

```icon share/pixmaps/huge.png
png 2048x2048
```

```icon share/pixmaps/heavy.png
png 48x48 1048577
```

```icon opt/logo
png 64x64
```

```icon share/icons/hicolor/48x48/apps/piped.png
fifo
```

```icon share/icons/hicolor/scalable/apps/piped.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#5e8bff"/></svg>
```

```icon share/pixmaps/fifo.png
fifo
```

```icon share/icons/hicolor/scalable/apps/linked.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><image href="$TREE/share/pixmaps/fifo.png" width="64" height="64"/></svg>
```

```icon share/icons/hicolor/64x64/apps/linked.png
png 64x64
```

```icon share/pixmaps/filtered.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><filter id="f"><feImage href="$TREE/share/pixmaps/fifo.png"/></filter><rect width="64" height="64" filter="url(#f)"/></svg>
```

```icon share/pixmaps/lettered.svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><text x="8" y="56" font-family="DejaVu Sans" font-size="56">A</text></svg>
```

```icons
firefox => share/icons/hicolor/48x48/apps/firefox.png 48x48
editor => share/icons/hicolor/scalable/apps/editor.svg 48x48
both => share/icons/hicolor/48x48/apps/both.png 48x48
big => share/icons/hicolor/256x256/apps/big.png 48x24
deep => share/icons/hicolor/48x48/apps/deep.png 48x48
mine => data/icons/hicolor/48x48/apps/mine.png 48x48
old => home/.icons/hicolor/48x48/apps/old.png 48x48
term => share/pixmaps/term.png 32x32
sketch => share/pixmaps/sketch.svg 48x24
electron => share/pixmaps/electron.png 48x48
huge =>
heavy =>
$TREE/share/icons/hicolor/48x48/apps/firefox.png => share/icons/hicolor/48x48/apps/firefox.png 48x48
$TREE/opt/logo => opt/logo 48x48
piped => share/icons/hicolor/scalable/apps/piped.svg 48x48
fifo =>
linked => share/icons/hicolor/64x64/apps/linked.png 48x48
filtered =>
lettered =>
nothing =>
```

`icons_stay_bounded` in `crates/lyncast/src/palette/tests.rs` puts 400 apps with an icon each in the list and moves the selection from the first app to the last. After each step the cache holds the icons of the rows the list has laid out and no others, up to 128. The first app's icon is in the window's atlas once it has loaded. The next summon's window draws the same image. At the end that icon has left the cache and the atlas, and the last app's icon is in both.

### A screenshot under anvil shows the palette

The window shows at the place the `geometry` block gives for anvil's 1280 by 800 output: (1280 − 750) / 2 = 265 from the left, and (800 − 480) / 3, in whole pixels, 106 from the top. Its edge is the border color. The background covers most of it, then the selected row, then the letter squares of the rows with no icon and the footer's keys, then the borders. The block checks no text pixels, which change with the fonts a machine has.

```summon
$ lyncast daemon --trace
$ lyncast toggle
open
$ wait-trace frame1_presented 1
$ shot palette
palette at 265,106 750x480
edge 3a3a3e
colors 1e1e20 2e2e33 2a2a2e 3a3a3e
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
```

### A screenshot under anvil shows the clipboard screen

Clipboard History from a client opens the window on the clipboard screen, at the same place. The image, seeded after the text, is the newest clip, so it is selected, and the pane shows it once it has loaded, as the shot waits for its green, `3c8c6e`. Then the background covers most of the box, the image 4.0%, the selected row 3.1%, and the borders 1.3%. The letter squares and the keys cover 0.9%, under the line, and text only takes pixels from them.

```summon
$ python3 -c 'import struct, sys, zlib; w, h = 160, 90; chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data)); rows = (b"\0" + b"\x3c\x8c\x6e" * w) * h; sys.stdout.buffer.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))' > green.png
$ lyncast daemon --trace
$ seed text/plain;charset=utf-8 alpha
$ wait-clips 1
$ seed-file image/png green.png
$ wait-clips 2
$ lyncast activate cmd:clipboard-history
ran cmd:clipboard-history
$ wait-trace frame1_presented 1
$ shot clipboard 3c8c6e
palette at 265,106 750x480
edge 3a3a3e
colors 1e1e20 3c8c6e 2e2e33 3a3a3e
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
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
