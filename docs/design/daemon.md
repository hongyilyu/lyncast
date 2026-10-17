# The daemon answers every request on one socket

This page describes the lyncast daemon and the CLI that talks to it, for a maintainer and for the tests. The [decision log](../decisions.md#daemon-and-cli) is the source of truth. Where this page disagrees with it, the log wins.

The palette and the CLI send the same requests. The CLI sends them over a Unix socket, and from plan step 7 the palette hands them to the daemon inside the process. So a CLI test takes the palette's path.

## Start the daemon with lyncast daemon

`lyncast daemon --headless` runs the daemon without GPUI. It takes the socket's lock, scans the apps, loads the visits and the clips, starts the capture thread when it has `WAYLAND_DISPLAY`, binds the socket, and then answers requests until a `Quit` command or until the compositor goes. `--headless` is required until plan step 7 builds the palette, the same way `--json` is required for `query`.

The daemon claims `$XDG_RUNTIME_DIR/lyncast.sock` in three moves:

1. It takes an exclusive `flock` on `$XDG_RUNTIME_DIR/lyncast.lock`. If another process holds the lock, it prints `lyncast: a daemon is already running` and exits 1. The lock comes before the scan, so a second daemon fails at once and never reads `ranking.json`. The open does not wait for a FIFO's reader, so a FIFO at that path fails the daemon at once. Any other lock file that is not a regular file, such as a link to a device, fails it with `not a regular file`.
2. Holding the lock, it removes any `lyncast.sock` left by a daemon that died without cleaning up, such as one killed with SIGKILL. Only the lock holder touches the socket, so two daemons that start at once cannot both bind.
3. It binds the socket and listens.

The kernel releases the lock when the process exits, however it exits. A `Quit` command also removes the socket. The lock file stays, because removing it would let a second daemon lock a new file while the first still holds the old one.

If the daemon cannot start the thread that writes its warnings, the one that accepts connections, the one that writes the visits, the one that writes the clips, or the capture thread, or cannot reach the compositor that `WAYLAND_DISPLAY` names, it prints `lyncast: not started: <error>` and exits 1. When the compositor goes later, the daemon stops as Quit does, prints `lyncast: the Wayland connection closed`, and exits 1 ([clipboard and paste](clipboard-and-paste.md#the-capture-thread-reads-one-representation-per-offer)).

Without `XDG_RUNTIME_DIR`, the daemon and every client print `lyncast: XDG_RUNTIME_DIR is not set` and exit 1, as Wayland clients refuse to run without it. A socket path holds at most 107 bytes, so the tests put `XDG_RUNTIME_DIR` under `/tmp`, as `scripts/test-env.sh` does.

The daemon reads its environment once, at start. The app scan reads `XDG_DATA_HOME`, `XDG_DATA_DIRS`, `PATH`, the locale variables, and `XDG_CURRENT_DESKTOP` there, with `HOME` in place of an unset `XDG_DATA_HOME`, and Reload Apps scans again with the same values. The visits and the [clipboard history](clipboard-and-paste.md#clips-keeps-the-rows-in-memory-and-in-sqlite) use the same data home. Open Config and the read of `config.toml` use `XDG_CONFIG_HOME`, or `HOME` in its place, from the same start, and the capture thread runs only when `WAYLAND_DISPLAY` is set then. A client's environment changes nothing but the socket path.

## One thread writes the warnings

The daemon prints each warning, such as a file it cannot read or a write that failed, as one line on stderr that starts with `lyncast: `. One thread writes every warning, so a stderr that stops draining, such as a pipe that no one reads, holds up that thread and no request: the daemon still starts and answers. Up to 8 warnings wait for the thread. A warning that does not fit is dropped, and once stderr drains, `lyncast: <n> warnings were dropped while stderr was full` follows the warnings that waited. The start waits up to 1 s for its warnings before it binds the socket, so a client that the socket answers finds them on stderr. Quit waits the same before it replies, and a daemon that stops on an error waits the same before it prints why, so its last line comes last.

## The CLI sends one request and prints the reply

Each command connects, sends one request, prints the reply, and exits:

| Command | Request | Prints on success |
|---|---|---|
| `lyncast toggle` | `toggle` | the new summon state, `open` or `closed` |
| `lyncast status` | `status` | the summon state |
| `lyncast query <text> --json` | `query` | one JSON object per row, as [ranking](ranking.md#lyncast-query-prints-one-json-object-per-row) describes |
| `lyncast clip list --json` | `clips` | one JSON object per clip, as [clipboard and paste](clipboard-and-paste.md#lyncast-clip-list-shows-the-history) describes |
| `lyncast activate <key>` | `activate` | one line that says what happened |

`activate` takes these options:

- `--chord enter` or `--chord ctrl-enter` names the keys pressed. Enter is the default.
- `--verb <verb>` picks an item from the row's Actions menu instead, with the verb spelled as `query --json` spells it, such as `copy-path` or `launch-action:new-window`. `--chord` and `--verb` together are a usage error.
- `--yes` confirms a command that asks for confirmation.
- `--query <text>` is the search text the row was found with. Frecency keeps it as a search term. It defaults to empty, and it may start with a hyphen.

The key may start with a hyphen after its prefix, as in `calc:-5+3`.

The exit code is part of the contract:

- `0` means the request succeeded.
- `1` means it did not. The reason goes to stderr as `lyncast: <reason>`. A command that asks for confirmation, a missing daemon, and a failed action all exit 1.
- `2` means a usage error. clap prints it.

`3` is kept for a stale rate snapshot, once currency exists.

## Activation rebuilds the row and runs one item

An `activate` request carries a key, the chord or verb, the confirmation, and the query. The daemon does this:

1. It rebuilds the row's `Subject` from current state. An app comes from the index, a command from its key, a clip from the clipboard history, and a calculator row from evaluating the key's text again. If the state has no such row, the request fails, for example with `no app nope.desktop`, `no clip 9`, or `calc:10/0 has no answer`.
2. It takes the item from the subject's `Actions`: `on(chord)` for a chord, or `pick(verb)` for a verb. If `pick` finds nothing, the request fails with `<key> has no action <verb>`.
3. If the item runs a command that confirms, and the request is not confirmed, the daemon replies with a confirmation request and runs nothing. Log Out, Reboot, and Power Off confirm. The CLI prints `lyncast: Reboot? Run again with --yes to confirm.` and exits 1.
4. It runs the item. Copy items need the clipboard, which comes in plan step 6. Until then they fail with `not copied: no Wayland compositor`. Items that start a program go to the runner.
5. On success it records a visit for `Launch`, `LaunchAction`, and every `Run` but Quit's, and replies with the line below. A success closes the summon, except Clipboard History's, which opens it. On failure nothing is recorded and the summon stays as it was.

| Item | Success line |
|---|---|
| `Launch(app)` | `launched app:<id>` |
| `LaunchAction(app, action)` | `launched app:<id> <action id>` |
| `Run(command)` | `ran cmd:<id>` |
| `CopyAnswer(query)` | `copied <answer>`, from plan step 6 |
| `CopyClip(clip)` | `copied clip <id>`, from plan step 6 |
| `CopyPath(app)` | `copied <path>`, from plan step 6 |
| `Pin(clip)`, `Unpin(clip)`, `Delete(clip)` | `pinned clip <id>`, `unpinned clip <id>`, `deleted clip <id>` |

Starting a program, Open Config, and Reload Apps run on a worker thread, and the daemon answers other requests while they run. Their reply comes when the work ends. If the worker thread cannot start, the request fails at once with `not run: <error>`, nothing is recorded, and Quit does not wait for it. Two Reload Apps can overlap, and their scans can end in either order. Each scan has a number in the order it started. A scan's apps replace the index only when that scan started after the one the index came from, so a scan that ends after a newer one changes nothing.

The commands run like this:

| Command | Runs |
|---|---|
| Clipboard History | opens the summon. Plan step 7 shows the clipboard screen. |
| Open Config | creates `$XDG_CONFIG_HOME/lyncast/config.toml` if it is missing, then runs `xdg-open <path>` |
| Reload Apps | scans the apps again |
| Quit lyncast | stops taking requests, waits for the work in flight and the last writes of the visits and the clips, gives up the lock, replies, and exits 0 |
| Lock Screen | `loginctl lock-session` |
| Suspend | `systemctl suspend` |
| Log Out | `niri msg action quit --skip-confirmation` |
| Reboot | `systemctl reboot` |
| Power Off | `systemctl poweroff` |

Log Out skips niri's own prompt, because lyncast has already asked. `XDG_CONFIG_HOME` defaults to `$HOME/.config`, as the XDG spec says. A relative value counts as unset, as the spec also says. Without an absolute `XDG_CONFIG_HOME` or `HOME`, Open Config fails with `not run: XDG_CONFIG_HOME and HOME are not set` and runs nothing.

Quit goes in this order:

1. It removes the socket, so no new client connects. A request that still arrives, on a connection opened before, gets `failed` with `the daemon is quitting` at once while steps 2 and 3 wait. One that arrives during step 4 waits until the process exits, up to 1 s, and then gets that reply or finds the connection closed. Either way it runs nothing.
2. It waits for the work in flight, such as a program the runner is starting, and sends each reply to its client. So a client exits 1 only when its program did not start.
3. It cancels the capture thread's `Watcher` and waits for the thread to end after the offer in hand. Then it waits for the last writes of the visits and the clips, gives up the lock, and replies. The writer of the clips gives up SQLite's lock when it ends. A client that runs after the reply finds no daemon, and a new daemon can start at once and load every visit and clip.
4. It waits up to 1 s for its warnings to reach stderr and for its replies, Quit's among them, to be written, and exits 0. The exit closes every connection, so it would cut off a reply that is still being written.

Steps 2 and 3 wait 5 s at most in all. After that the daemon takes no more requests or results. A request or a job's result that is still queued gets `failed` with `the daemon is quitting`, even when the job's program started, and work still running gets no reply. So Quit takes 6 s at most, plus the time to handle the one request or result in hand when the 5 s run out. Work or a write still running, or a result that the daemon did not take, keeps the lock until the process exits, so a daemon started in its place never writes `ranking.json` at the same time, and finds `clips.sqlite3` locked.

Once Quit has begun, the daemon fails every request, the palette's from plan step 7 among them, and it says when no work or write is left. The host owns the order and the waits. It removes the socket and hands the daemon each request and job result until the daemon says so or the 5 s run out. It checks the time before it takes each one, because its channel gives back a queued one at once even after the 5 s. Each one still queued then fails without reaching the daemon, and the host gives up the lock only if the daemon said so. Then it sends the reply and waits up to 1 s for the replies to be written. Plan step 7's palette host has to follow the same steps.

## The runner starts programs, and tests swap in a recorder

Every program the daemon starts goes through one `Runner`, as an argv: a command's argv from the table above, or an app's `Exec` argv, or a Desktop Action's. The runner has two forms:

- The real runner starts programs through niri IPC, which plan step 8 builds. Until then it fails with `not run: launching through niri is not built yet`.
- `lyncast daemon --record <file>` swaps in a recorder. It appends each argv to `<file>` as one compact JSON array per line, and reports success. It writes one whole line at a time, so the lines of two programs started at once never mix. The tests start every daemon that serves with it, so no test starts a real program.

A Tier 1 test also runs the daemon with an empty environment plus the doc's variables, so `PATH` holds only the test tree and `NIRI_SOCKET`, `WAYLAND_DISPLAY`, and `WAYLAND_SOCKET` are unset.

## The summon is open or closed

The daemon keeps one summon state, `closed` or `open`. Without GPUI it opens no window. `toggle` flips the state, the Clipboard History command opens it, and every other successful activation closes it. A failed activation leaves it alone, so an open palette stays open to show the reason. From plan step 7 an open summon is a real window, and closing the window closes the summon.

## The root list puts the calculator card first

A query fans out to the calculator and to ranking, and the merge orders the rows by `Section`. The calculator's card, when the text is math, is the one Answer row at index 0. Ranking's rows follow.

The card row's key is `calc:<raw query>`, its kind is `calculator`, and its title is the answer's display, such as `-2`. In `query --json` it also carries a `card` object, after `title`, with the five fields of the answer: `expression`, `source_badge`, `display`, `copy`, and `target_badge`, which is `null` when the answer has no words. No other row has a `card` field.

## Visits persist in ranking.json

The daemon owns one `Visits`. It loads `$XDG_DATA_HOME/lyncast/ranking.json` at start. After every recorded visit it leaves a copy of the visits in one slot, in place of any copy that one writer thread has not taken yet. The writer takes the newest copy, writes it to `ranking.json.<pid>.tmp` in the same directory, where `<pid>` is the daemon's process ID, and renames that over the file, so a crash leaves the old file or the new one. A write that fails removes its temporary file, because no later daemon has its name. Two daemons with their own `XDG_RUNTIME_DIR` hold their own locks and can share a data home, so the process ID keeps each out of the other's temporary file, and each rename puts one daemon's whole copy in place. The daemon keeps answering while a write waits on the disk, and at most one copy waits in the slot meanwhile. Quit waits for the last write. `XDG_DATA_HOME` resolves as it does for the app scan: an absolute `XDG_DATA_HOME`, else `$HOME/.local/share`. With neither, the daemon keeps visits in memory only.

The file is one JSON object. Each member is a key with its visit, in key order:

```json
{"app:firefox.desktop":{"anchor":1760650000000,"last_opened":1760600000000,"terms":["fir"]}}
```

`anchor` and `last_opened` are milliseconds since the Unix epoch, from 1970 to the end of 9999. `terms` holds the last three distinct search terms in their fold, most recent first, and a term over 1,024 bytes is not kept. On load the daemon folds each stored term again, drops empty, long, and repeated terms, and keeps the first three, so a file edited by hand still follows that rule. A member whose key no longer parses is dropped.

The daemon reads only a regular file of at most 64 MiB, after symlinks. It checks the path first and never opens a FIFO or a device, because opening a device can change it, as opening a serial port resets the board on it. Then it opens the file without waiting for a FIFO's writer and checks the file it opened, so a FIFO swapped in after the first check cannot block it. A session cannot swap the file while a daemon starts, so `a_fifo_swapped_in_for_ranking_json_never_blocks_a_start` in `crates/lyncast/tests/daemon.rs` keeps swapping a link to a file for a link to a FIFO while 40 daemons start, and expects each to bind. A real file holds a few kilobytes. No entry at all is a fresh install, but a symlink to nothing is a file that cannot be read.

The daemon keeps a bad file for the user and never writes over it:

- If the file cannot be read, is not JSON of this shape, or holds a time outside that range, the daemon prints `lyncast: <path>: <error>; moved it to ranking.json.bad` on stderr, moves it, and starts with no visits. The move is one rename that fails rather than replace a `ranking.json.bad` that is already there (`renameat2` with `RENAME_NOREPLACE`), so even a daemon with another runtime directory and the same data home can neither replace a kept bad file nor lose a file that another daemon has just written. On a filesystem that cannot rename that way, the move fails.
- If `ranking.json.bad` is already there, the daemon does not move the new bad file over it. It prints `lyncast: <path>: <error>; not moved to ranking.json.bad: it already exists; the visits stay in memory`. If the move fails, the line names its error after `not moved to ranking.json.bad: `. Either way the daemon starts with no visits and writes no file until it exits.

The daemon never writes a file it would not read, so a copy over 64 MiB fails with `not saved: over 64 MiB`, and the file stays as it was. A write that fails prints `lyncast: <path>: not saved: <error>`, and the next visit writes the file again.

## The wire is one JSON line each way

A client connects, writes one request as compact JSON and a newline, and reads one reply line. Then both sides close. A connection holds no state, so the palette and many CLI clients can talk to the daemon at once. Keys travel as their strings and are parsed at the socket, and verbs and chords use the spellings of `query --json`.

A request line holds at most 1 MiB, with its newline. A line with no newline in its first 1 MiB gets `failed` with `bad request: no newline in the first 1 MiB`, and the daemon runs none of it, even when the bytes before the cut are a whole request. A request that does not parse gets a `failed` reply that names the error. Either way the daemon keeps serving. The daemon closes a connection without a reply when 5 s pass with no byte of its request, and it stops writing a reply when 5 s pass with no room in the socket to write it. These limits apply to each read and write, so a client that sends slowly is still served.

Requests:

| Request | Fields |
|---|---|
| `{"request":"toggle"}` | |
| `{"request":"status"}` | |
| `{"request":"query","text":...}` | the query as typed |
| `{"request":"clips"}` | |
| `{"request":"activate","key":...,"via":...,"confirmed":...,"query":...}` | `via` is `{"chord":"enter"}` or `{"verb":"pin"}` |

Replies:

| Reply | Fields |
|---|---|
| `{"reply":"summon","state":...}` | `open` or `closed` |
| `{"reply":"rows","rows":[...]}` | the rows, each as `query --json` prints it |
| `{"reply":"clips","clips":[...]}` | the clips, each as `clip list --json` prints it |
| `{"reply":"done","message":...}` | the success line |
| `{"reply":"confirm","title":...}` | the command's title |
| `{"reply":"failed","reason":...}` | the reason, without the `lyncast: ` prefix |

## The fixtures

The tests in `crates/lyncast/tests/daemon.rs` read the blocks below. Each `session` and `wire` block gets a fresh directory, `$TREE`, with every `desktop` and `executable` block written under it, and its own `XDG_RUNTIME_DIR`, `$RUNTIME`, a fresh directory under `/tmp`. The tests run the real binary with only the `env` block's variables and `$TREE` as the working directory. The test removes a block's `$TREE` once the block passes.

```env
HOME=$TREE/home
XDG_DATA_HOME=$TREE/data
XDG_CONFIG_HOME=$TREE/config
XDG_DATA_DIRS=$TREE/share
XDG_RUNTIME_DIR=$RUNTIME
XDG_CURRENT_DESKTOP=niri
LANG=C
PATH=$TREE/bin
```

The tree holds Firefox, with one Desktop Action, and 8x8, an app named like a product:

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

A `session` block is a terminal transcript against one environment:

- A line that starts with `$ ` is a command. A command that runs `lyncast` splits into words as in [ranking's](ranking.md#lyncast-query-prints-one-json-object-per-row) `cli` blocks, and it may set variables before `lyncast`, whose values may hold `$TREE` and `$RUNTIME`. Any other command runs through `/bin/sh -c`, with `$TREE` as its working directory and only `PATH=/usr/bin:/bin`, `TREE`, and `RUNTIME` in its environment, so a session can write, change, and read files. Once the block has started a daemon in the background, `DAEMON` holds the newest one's process ID too.
- Every `$ lyncast daemon ...` line that names no `--record` of its own gets `--record $RUNTIME/runs` added, so no daemon a test starts can run a real program, even one that the test expects to fail. With no expected lines after it, the line starts the daemon in the background, with its stdout and stderr appended to `$RUNTIME/daemon.stdout` and `$RUNTIME/daemon.stderr`, and the test waits until the socket accepts. A line that sets `XDG_RUNTIME_DIR` waits for the socket there. With expected lines, it runs to the end like any command.
- Each line after a command is what that command prints. A plain line is stdout. A line that starts with `stderr: ` is stderr. `exit: <n>` is the exit code, 0 when absent. `record: <argv>` is a line the command added to `$RUNTIME/runs`, and with no `record:` line the command added none. `$TREE` and `$RUNTIME` in an expected line stand for the two paths.
- `$ stall-stderr` gives the next daemon a stderr that does not drain: a pipe that the test fills before the daemon starts, in place of `$RUNTIME/daemon.stderr`. `$ drain-stderr` reads what fills it, so the newest daemon's writes to stderr go through. `$ wait-exit` waits for the newest daemon to exit, prints what it wrote to stderr since it started, and exits as the daemon did.

At the end of a block the test stops any daemon still running.

### Each request round-trips

`toggle` and `status` share the summon state.

```session
$ lyncast daemon --headless
$ lyncast status
closed
$ lyncast toggle
open
$ lyncast status
open
$ lyncast toggle
closed
```

A query goes to the daemon, which ranks with the environment it read at start, so a client's `XDG_DATA_DIRS` hides nothing. The fresh install's empty list is Suggestions, Clipboard History then the apps alphabetically, and then Commands.

```session
$ lyncast daemon --headless
$ lyncast query fire --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
$ XDG_DATA_DIRS=$TREE/nowhere lyncast query fire --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
$ lyncast query "" --json
{"key":"cmd:clipboard-history","index":0,"kind":"command","section":"suggestions","title":"Clipboard History","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"app:8x8.desktop","index":1,"kind":"app","section":"suggestions","title":"8x8","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:firefox.desktop","index":2,"kind":"app","section":"suggestions","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"cmd:lock-screen","index":3,"kind":"command","section":"commands","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:log-out","index":4,"kind":"command","section":"commands","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:open-config","index":5,"kind":"command","section":"commands","title":"Open Config","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:power-off","index":6,"kind":"command","section":"commands","title":"Power Off","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:quit","index":7,"kind":"command","section":"commands","title":"Quit lyncast","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reboot","index":8,"kind":"command","section":"commands","title":"Reboot","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":9,"kind":"command","section":"commands","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:suspend","index":10,"kind":"command","section":"commands","title":"Suspend","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
```

### The calculator card comes first

The card is the one Answer row, with its `card` object, and ranking's rows follow it, so `8x8` shows 64 and then the app. A bare negated number is no card, and an answer with no words, such as one that is not whole, has a `null` target badge.

```session
$ lyncast daemon --headless
$ lyncast query 2+3*4 --json
{"key":"calc:2+3*4","index":0,"kind":"calculator","section":"answer","title":"14","card":{"expression":"2 + 3 × 4","source_badge":"Sum","display":"14","copy":"14","target_badge":"Fourteen"},"actions":[{"verb":"copy","label":"Copy Answer","chords":["enter","ctrl-enter"]}]}
$ lyncast query -5+3 --json
{"key":"calc:-5+3","index":0,"kind":"calculator","section":"answer","title":"-2","card":{"expression":"−5 + 3","source_badge":"Sum","display":"-2","copy":"-2","target_badge":"Minus Two"},"actions":[{"verb":"copy","label":"Copy Answer","chords":["enter","ctrl-enter"]}]}
$ lyncast query -(2+3) --json
{"key":"calc:-(2+3)","index":0,"kind":"calculator","section":"answer","title":"-5","card":{"expression":"−(2 + 3)","source_badge":"Negation","display":"-5","copy":"-5","target_badge":"Minus Five"},"actions":[{"verb":"copy","label":"Copy Answer","chords":["enter","ctrl-enter"]}]}
$ lyncast query --5 --json
$ lyncast query 8x8 --json
{"key":"calc:8x8","index":0,"kind":"calculator","section":"answer","title":"64","card":{"expression":"8 × 8","source_badge":"Product","display":"64","copy":"64","target_badge":"Sixty-Four"},"actions":[{"verb":"copy","label":"Copy Answer","chords":["enter","ctrl-enter"]}]}
{"key":"app:8x8.desktop","index":1,"kind":"app","section":"results","title":"8x8","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
$ lyncast query 123456789012345+0.5 --json
{"key":"calc:123456789012345+0.5","index":0,"kind":"calculator","section":"answer","title":"123,456,789,012,345.5","card":{"expression":"123,456,789,012,345 + 0.5","source_badge":"Sum","display":"123,456,789,012,345.5","copy":"123456789012345.5","target_badge":null},"actions":[{"verb":"copy","label":"Copy Answer","chords":["enter","ctrl-enter"]}]}
```

### Commands that confirm ask first

Without `--yes`, Reboot, Power Off, and Log Out ask and run nothing. With it, the recorder gets the argv.

```session
$ lyncast daemon --headless
$ lyncast activate cmd:reboot
stderr: lyncast: Reboot? Run again with --yes to confirm.
exit: 1
$ lyncast activate cmd:reboot --yes
ran cmd:reboot
record: ["systemctl","reboot"]
$ lyncast activate cmd:power-off
stderr: lyncast: Power Off? Run again with --yes to confirm.
exit: 1
$ lyncast activate cmd:log-out
stderr: lyncast: Log Out? Run again with --yes to confirm.
exit: 1
$ lyncast activate cmd:log-out --yes
ran cmd:log-out
record: ["niri","msg","action","quit","--skip-confirmation"]
```

Commands that do not confirm run at once, and Ctrl+Enter runs the default too.

```session
$ lyncast daemon --headless
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast activate cmd:suspend --chord ctrl-enter
ran cmd:suspend
record: ["systemctl","suspend"]
```

### Open Config creates the file once

Open Config creates an empty `config.toml` and opens it. A second run keeps what the file holds.

```session
$ lyncast daemon --headless
$ lyncast activate cmd:open-config
ran cmd:open-config
record: ["xdg-open","$TREE/config/lyncast/config.toml"]
$ wc -c config/lyncast/config.toml
0 config/lyncast/config.toml
$ echo 'theme = "dark"' > config/lyncast/config.toml
$ lyncast activate cmd:open-config
ran cmd:open-config
record: ["xdg-open","$TREE/config/lyncast/config.toml"]
$ cat config/lyncast/config.toml
theme = "dark"
```

Without `HOME`, and with a relative `XDG_CONFIG_HOME`, the daemon has no config home, so Open Config fails and runs nothing.

```session
$ env -u HOME XDG_CONFIG_HOME=config lyncast daemon --headless
$ lyncast activate cmd:open-config
stderr: lyncast: not run: XDG_CONFIG_HOME and HOME are not set
exit: 1
```

### Reload Apps scans again

A new desktop file shows only after Reload Apps. The scan uses the environment the daemon read at start, so the client's `XDG_DATA_DIRS` changes nothing. A session cannot hold one scan back to end it after a newer one, so a unit test in `crates/lyncast/src/daemon/mod.rs` ends two scans out of order itself, with the first `desktop` block above as the newer scan's one app.

```session
$ lyncast daemon --headless
$ printf '[Desktop Entry]\nType=Application\nName=Nightly\nExec=firefox\n' > share/applications/nightly.desktop
$ lyncast query nightly --json
$ XDG_DATA_DIRS=$TREE/nowhere lyncast activate cmd:reload-apps
ran cmd:reload-apps
$ lyncast query nightly --json
{"key":"app:nightly.desktop","index":0,"kind":"app","section":"results","title":"Nightly","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

### Apps launch through the runner

Enter launches the app's Exec, and a verb picks a Desktop Action. A verb the row does not offer fails. A session runs one command at a time, so `launches_at_once_record_whole_lines` in `crates/lyncast/tests/daemon.rs` starts two launches at once itself. Each has an argument of 18,000 bytes, more than a FIFO takes in one write, and the test checks that the recorder holds each argv on a line of its own. `--chord` and `--verb` read only the spellings that `query --json` prints, so `Enter`, `ctrl+enter`, the label `open`, `launch-action` and `launch-action:`, which name no action, and the empty text are usage errors.

```session
$ lyncast daemon --headless
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
record: ["firefox"]
$ lyncast activate app:firefox.desktop --verb launch-action:new-window
launched app:firefox.desktop new-window
record: ["firefox","--new-window"]
$ lyncast activate app:firefox.desktop --verb pin
stderr: lyncast: app:firefox.desktop has no action pin
exit: 1
$ lyncast activate app:nope.desktop
stderr: lyncast: no app nope.desktop
exit: 1
$ lyncast activate app:firefox.desktop --chord Enter
stderr: error: invalid value 'Enter' for '--chord <CHORD>': unknown chord
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate app:firefox.desktop --chord ctrl+enter
stderr: error: invalid value 'ctrl+enter' for '--chord <CHORD>': unknown chord
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate app:firefox.desktop --chord ""
stderr: error: invalid value '' for '--chord <CHORD>': unknown chord
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate app:firefox.desktop --verb open
stderr: error: invalid value 'open' for '--verb <VERB>': unknown verb
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate app:firefox.desktop --verb launch-action
stderr: error: invalid value 'launch-action' for '--verb <VERB>': unknown verb
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate app:firefox.desktop --verb launch-action:
stderr: error: invalid value 'launch-action:' for '--verb <VERB>': unknown verb
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate app:firefox.desktop --verb ""
stderr: error: invalid value '' for '--verb <VERB>': unknown verb
stderr: 
stderr: For more information, try '--help'.
exit: 2
```

### Copies fail until the clipboard exists

A calculator key is evaluated again, and a clip key that names no clip fails.

```session
$ lyncast daemon --headless
$ lyncast activate calc:2+3*4
stderr: lyncast: not copied: no Wayland compositor
exit: 1
$ lyncast activate calc:10/0
stderr: lyncast: calc:10/0 has no answer
exit: 1
$ lyncast activate app:firefox.desktop --verb copy-path
stderr: lyncast: not copied: no Wayland compositor
exit: 1
$ lyncast activate clip:9
stderr: lyncast: no clip 9
exit: 1
```

### A successful activation closes the summon

A failure leaves it open, and Clipboard History opens it.

```session
$ lyncast daemon --headless
$ lyncast toggle
open
$ lyncast activate calc:2+3
stderr: lyncast: not copied: no Wayland compositor
exit: 1
$ lyncast status
open
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast status
closed
$ lyncast activate cmd:clipboard-history
ran cmd:clipboard-history
$ lyncast status
open
```

### A program that fails to start leaves the visits and the summon alone

Here `$RUNTIME/runs` is a directory, so the recorder fails, as the real runner fails until plan step 8. The summon stays open, and Log Out gets no visit, so it still follows Lock Screen for `lo`, where a visit would put it first. Open Config creates `config.toml` before the runner opens it, so a failed Open Config still leaves the file.

```session
$ mkdir $RUNTIME/runs
$ lyncast daemon --headless
$ lyncast toggle
open
$ lyncast activate cmd:log-out --yes
stderr: lyncast: not run: $RUNTIME/runs: Is a directory (os error 21)
exit: 1
$ lyncast status
open
$ lyncast query lo --json
{"key":"cmd:lock-screen","index":0,"kind":"command","section":"results","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:log-out","index":1,"kind":"command","section":"results","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":2,"kind":"command","section":"results","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
$ lyncast activate cmd:open-config
stderr: lyncast: not run: $RUNTIME/runs: Is a directory (os error 21)
exit: 1
$ wc -c < config/lyncast/config.toml
0
```

### Visits rank Suggestions and survive a restart

An activation records a visit, so Firefox leads Suggestions. Quit removes the socket, writes `ranking.json`, and keeps the lock file, and a new daemon loads the visits. The times in the file come from the clock, so `sed` masks their digits. `f` finds Firefox alone, because an `f` inside a word, as in Power Off and Open Config, scores 2, below the 2.5 that a one-letter query needs ([ranking](ranking.md#frizbee-scores-with-tinycasts-points)).

```session
$ lyncast daemon --headless
$ lyncast activate app:firefox.desktop --query Fir
launched app:firefox.desktop
record: ["firefox"]
$ lyncast activate cmd:quit
ran cmd:quit
$ lyncast status
stderr: lyncast: no daemon is listening on $RUNTIME/lyncast.sock
exit: 1
$ ls $RUNTIME
daemon.stderr
daemon.stdout
lyncast.lock
runs
$ sed -E 's/[0-9]+/N/g' data/lyncast/ranking.json
{"app:firefox.desktop":{"anchor":N,"last_opened":N,"terms":["fir"]}}
$ lyncast daemon --headless
$ lyncast query "" --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"suggestions","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"cmd:clipboard-history","index":1,"kind":"command","section":"suggestions","title":"Clipboard History","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"app:8x8.desktop","index":2,"kind":"app","section":"suggestions","title":"8x8","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"cmd:lock-screen","index":3,"kind":"command","section":"commands","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:log-out","index":4,"kind":"command","section":"commands","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:open-config","index":5,"kind":"command","section":"commands","title":"Open Config","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:power-off","index":6,"kind":"command","section":"commands","title":"Power Off","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:quit","index":7,"kind":"command","section":"commands","title":"Quit lyncast","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reboot","index":8,"kind":"command","section":"commands","title":"Reboot","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":9,"kind":"command","section":"commands","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:suspend","index":10,"kind":"command","section":"commands","title":"Suspend","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
$ lyncast query f --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

### Stored visits load as the rules say

A member whose key no longer parses is dropped, and the stored terms are folded again, deduplicated, and cut to three. Log Out's anchor is in 2255, so its visit is warm and Log Out leads `lo`. The next write puts the five members in key order, not the order they were stored in, and keeps Log Out's times as they were stored. With five members, a write in another order is unlikely to match by chance.

```session
$ mkdir -p data/lyncast
$ echo '{"cmd:suspend":{"anchor":1,"last_opened":1,"terms":[]},"cmd:frobnicate":{"anchor":1,"last_opened":1,"terms":[]},"cmd:reboot":{"anchor":1,"last_opened":1,"terms":[]},"cmd:power-off":{"anchor":1,"last_opened":1,"terms":[]},"cmd:log-out":{"anchor":9000000000000,"last_opened":1760000000000,"terms":["Out","OUT","","log","x","y"]}}' > data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast query lo --json
{"key":"cmd:log-out","index":0,"kind":"command","section":"results","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:lock-screen","index":1,"kind":"command","section":"results","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":2,"kind":"command","section":"results","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
$ grep -o '"cmd:[a-z-]*":' data/lyncast/ranking.json
"cmd:lock-screen":
"cmd:log-out":
"cmd:power-off":
"cmd:reboot":
"cmd:suspend":
$ grep -o '"cmd:log-out":{[^}]*}' data/lyncast/ranking.json
"cmd:log-out":{"anchor":9000000000000,"last_opened":1760000000000,"terms":["out","log","x"]}
```

An anchor more than 10,240 days ahead, which a clock set back can leave, scores infinity, and the daemon loads it as stored. The entry's next visit sets the anchor 10,240 days from now, about 28 years, as [ranking](ranking.md#frecency-halves-every-10-days) says, so the entry leads Suggestions for about that long. The shell line prints how far ahead the stored anchor is.

```session
$ mkdir -p data/lyncast
$ echo '{"cmd:reboot":{"anchor":5000000000000,"last_opened":1760000000000,"terms":[]}}' > data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:reboot --yes
ran cmd:reboot
record: ["systemctl","reboot"]
$ lyncast activate cmd:quit
ran cmd:quit
$ anchor=$(grep -o '"anchor":[0-9]*' data/lyncast/ranking.json | cut -d : -f 2); echo $(( (anchor / 1000 - $(date +%s)) / 86400 / 365 )) years ahead
28 years ahead
```

### A bad ranking.json is kept, never written over

A file that is not JSON moves to `ranking.json.bad`, and the daemon starts with the fresh install's list. The next visit writes a new file.

```session
$ mkdir -p data/lyncast
$ echo not json > data/lyncast/ranking.json
$ lyncast daemon --headless
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: expected ident at line 1 column 2; moved it to ranking.json.bad
$ cat data/lyncast/ranking.json.bad
not json
$ lyncast query lo --json
{"key":"cmd:lock-screen","index":0,"kind":"command","section":"results","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:log-out","index":1,"kind":"command","section":"results","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":2,"kind":"command","section":"results","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
$ lyncast activate cmd:lock-screen --query Lock
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast activate cmd:quit
ran cmd:quit
$ sed -E 's/[0-9]+/N/g' data/lyncast/ranking.json
{"cmd:lock-screen":{"anchor":N,"last_opened":N,"terms":["lock"]}}
```

A time outside 1970 to 9999 fails the file the same way, at either end, and the two ends themselves load.

```session
$ mkdir -p data/lyncast
$ echo '{"cmd:reboot":{"anchor":253402300799999,"last_opened":0,"terms":[]}}' > data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ echo '{"cmd:reboot":{"anchor":253402300800000,"last_opened":0,"terms":[]}}' > data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rm data/lyncast/ranking.json.bad
$ echo '{"cmd:reboot":{"anchor":1,"last_opened":-9223372036854775808,"terms":[]}}' > data/lyncast/ranking.json
$ lyncast daemon --headless
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: 253402300800000 is not a time from 1970 to 9999 at line 1 column 39; moved it to ranking.json.bad
lyncast: $TREE/data/lyncast/ranking.json: -9223372036854775808 is not a time from 1970 to 9999 at line 1 column 60; moved it to ranking.json.bad
```

A file that cannot be read fails it too: a symlink to itself, a symlink to nothing, a FIFO, a symlink to a device, a sparse file of 1 TiB, over the cap, and a directory, which only a rename can move. Each daemon here moves one aside, and the shell line after it removes the bad file for the next. The daemon never opens the FIFO, so a program that waits to write to it, under a second name, still waits when `timeout` stops it after 2 s, and `timeout` exits 124.

```session
$ mkdir -p data/lyncast
$ ln -s ranking.json data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ readlink data/lyncast/ranking.json.bad
ranking.json
$ rm data/lyncast/ranking.json.bad && ln -s missing.json data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ readlink data/lyncast/ranking.json.bad
missing.json
$ rm data/lyncast/ranking.json.bad && mkfifo fifo && ln fifo data/lyncast/ranking.json
$ (timeout 2 sh -c ': > fifo'; echo $? > fifo.writer) > /dev/null 2>&1 & sleep 0.5
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ until [ -s fifo.writer ]; do sleep 0.1; done; cat fifo.writer
124
$ rm data/lyncast/ranking.json.bad && ln -s /dev/zero data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rm data/lyncast/ranking.json.bad && truncate -s 1T data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rm data/lyncast/ranking.json.bad && mkdir data/lyncast/ranking.json
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ ls data/lyncast
clips.sqlite3
ranking.json.bad
$ test -d data/lyncast/ranking.json.bad && echo directory
directory
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: Too many levels of symbolic links (os error 40); moved it to ranking.json.bad
lyncast: $TREE/data/lyncast/ranking.json: No such file or directory (os error 2); moved it to ranking.json.bad
lyncast: $TREE/data/lyncast/ranking.json: not a regular file; moved it to ranking.json.bad
lyncast: $TREE/data/lyncast/ranking.json: not a regular file; moved it to ranking.json.bad
lyncast: $TREE/data/lyncast/ranking.json: over 64 MiB; moved it to ranking.json.bad
lyncast: $TREE/data/lyncast/ranking.json: not a regular file; moved it to ranking.json.bad
```

When `ranking.json.bad` is already there, the new bad file stays where it is, and the visits stay in memory. Log Out's visit still ranks it first, and the daemon writes over neither file.

```session
$ mkdir -p data/lyncast
$ echo kept > data/lyncast/ranking.json.bad
$ ln -s ranking.json data/lyncast/ranking.json
$ lyncast daemon --headless
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: Too many levels of symbolic links (os error 40); not moved to ranking.json.bad: it already exists; the visits stay in memory
$ lyncast activate cmd:log-out --yes
ran cmd:log-out
record: ["niri","msg","action","quit","--skip-confirmation"]
$ lyncast query lo --json
{"key":"cmd:log-out","index":0,"kind":"command","section":"results","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:lock-screen","index":1,"kind":"command","section":"results","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":2,"kind":"command","section":"results","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
$ lyncast activate cmd:quit
ran cmd:quit
$ readlink data/lyncast/ranking.json
ranking.json
$ cat data/lyncast/ranking.json.bad
kept
```

### A file of 64 MiB loads, and no larger one is written

A file of exactly 64 MiB loads. This one holds Log Out's visit beside a member whose key is a long app ID. A visit would make the file larger, so the write fails, the file stays as it was, and the visits stay in memory. The test of Quit's deadline reuses this session's setup, which its `large-file` tag names.

```session large-file
$ mkdir -p data/lyncast
$ { printf '{"app:'; head -c 67108730 /dev/zero | tr '\0' x; printf '.desktop":{"anchor":1,"last_opened":1,"terms":[]},"cmd:log-out":{"anchor":9000000000000,"last_opened":1760000000000,"terms":[]}}'; } > data/lyncast/ranking.json
$ wc -c < data/lyncast/ranking.json
67108864
$ lyncast daemon --headless
$ lyncast query lo --json
{"key":"cmd:log-out","index":0,"kind":"command","section":"results","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:lock-screen","index":1,"kind":"command","section":"results","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":2,"kind":"command","section":"results","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: not saved: over 64 MiB
$ wc -c < data/lyncast/ranking.json
67108864
```

### A slow write holds up nothing but Quit

Here the daemon's temporary file is a FIFO, so the write after Lock Screen waits until something reads it, and the daemon still answers. The shell line starts a reader a second later, while Quit waits for the write. A FIFO cannot be synced, so the write then fails, and its warning reaches stderr before Quit replies. The failed write removes its temporary file, so only the clipboard history is left.

```session
$ mkdir -p data/lyncast
$ lyncast daemon --headless
$ mkfifo data/lyncast/ranking.json.$DAEMON.tmp
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ lyncast status
closed
$ (sleep 1; timeout 5 cat data/lyncast/ranking.json.$DAEMON.tmp) > /dev/null 2>&1 &
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: not saved: Invalid argument (os error 22)
$ ls data/lyncast
clips.sqlite3
```

### A stderr that does not drain holds up no start

Here the daemon's stderr is full before it starts, so its warning about `ranking.json` waits. The start waits 1 s for it and then binds, and the daemon answers. Once the test reads what filled the pipe, the warning goes through before Quit replies.

```session
$ mkdir -p data/lyncast && echo not json > data/lyncast/ranking.json
$ stall-stderr
$ lyncast daemon --headless
$ lyncast status
closed
$ drain-stderr
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
stderr: lyncast: $TREE/data/lyncast/ranking.json: expected ident at line 1 column 2; moved it to ranking.json.bad
```

### One daemon owns the socket

A second daemon finds the lock taken. A lock file that is not a regular file, such as a FIFO or a link to `/dev/null`, fails the daemon at once. Without a daemon, or without `XDG_RUNTIME_DIR`, every client fails. A daemon line without `--headless` is a usage error, and clap's usage line names the options the line gave, such as the `--record` given here, which the harness then does not add.

```session
$ lyncast toggle
stderr: lyncast: no daemon is listening on $RUNTIME/lyncast.sock
exit: 1
$ lyncast status
stderr: lyncast: no daemon is listening on $RUNTIME/lyncast.sock
exit: 1
$ lyncast query fire --json
stderr: lyncast: no daemon is listening on $RUNTIME/lyncast.sock
exit: 1
$ mkfifo $RUNTIME/lyncast.lock
$ lyncast daemon --headless
stderr: lyncast: $RUNTIME/lyncast.lock: No such device or address (os error 6)
exit: 1
$ rm $RUNTIME/lyncast.lock && ln -s /dev/null $RUNTIME/lyncast.lock
$ lyncast daemon --headless
stderr: lyncast: $RUNTIME/lyncast.lock: not a regular file
exit: 1
$ rm $RUNTIME/lyncast.lock
$ lyncast daemon --headless
$ lyncast daemon --headless
stderr: lyncast: a daemon is already running
exit: 1
$ env -u XDG_RUNTIME_DIR lyncast status
stderr: lyncast: XDG_RUNTIME_DIR is not set
exit: 1
$ env -u XDG_RUNTIME_DIR lyncast daemon --headless
stderr: lyncast: XDG_RUNTIME_DIR is not set
exit: 1
$ lyncast daemon --record runs
stderr: error: the following required arguments were not provided:
stderr:   --headless
stderr: 
stderr: Usage: lyncast daemon --headless --record <FILE>
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate cmd:reboot --chord enter --verb run
stderr: error: the argument '--chord <CHORD>' cannot be used with '--verb <VERB>'
stderr: 
stderr: Usage: lyncast activate --chord <CHORD> <KEY>
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate cmd:nope
stderr: error: invalid value 'cmd:nope' for '<KEY>': unknown command
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate foo:bar
stderr: error: invalid value 'foo:bar' for '<KEY>': unknown kind
stderr: 
stderr: For more information, try '--help'.
exit: 2
$ lyncast activate app:
stderr: error: invalid value 'app:' for '<KEY>': nothing after the colon
stderr: 
stderr: For more information, try '--help'.
exit: 2
```

### The socket speaks one JSON line each way

A `wire` block sends each `>` line, with a newline, to a daemon started as a session starts one, each on a connection of its own. It expects the `<` line after it as the reply, and `record:` lines after that, as in a session.

```wire
> {"request":"status"}
< {"reply":"summon","state":"closed"}
> {"request":"toggle"}
< {"reply":"summon","state":"open"}
> {"request":"query","text":"fire"}
< {"reply":"rows","rows":[{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}]}
> {"request":"clips"}
< {"reply":"clips","clips":[]}
> {"request":"activate","key":"cmd:reboot","via":{"chord":"enter"},"confirmed":false,"query":""}
< {"reply":"confirm","title":"Reboot"}
> {"request":"activate","key":"cmd:reboot","via":{"verb":"run"},"confirmed":true,"query":"reb"}
< {"reply":"done","message":"ran cmd:reboot"}
record: ["systemctl","reboot"]
> {"request":"activate","key":"clip:x","via":{"chord":"enter"},"confirmed":false,"query":""}
< {"reply":"failed","reason":"bad request: bad clip id in clip:x"}
> {"request":"activate","key":"cmd:reboot","via":{"chord":"Enter"},"confirmed":true,"query":""}
< {"reply":"failed","reason":"bad request: unknown chord in Enter"}
> {"request":"activate","key":"cmd:reboot","via":{"verb":"open"},"confirmed":true,"query":""}
< {"reply":"failed","reason":"bad request: unknown verb in open"}
> not json
< {"reply":"failed","reason":"bad request: expected ident at line 1 column 2"}
```

A `wire unterminated` block sends each `>` line twice, each time on a new connection. The first time it has no newline, and the test closes its half of the connection after it. The second time it is padded with spaces to 1 MiB, and `not json` and a newline follow. Both get the `<` reply, and neither records anything. A toggle that ran would have replied with the summon state, and a Reboot that ran would have recorded its argv.

```wire unterminated
> {"request":"toggle"}
< {"reply":"failed","reason":"bad request: no newline in the first 1 MiB"}
> {"request":"activate","key":"cmd:reboot","via":{"verb":"run"},"confirmed":true,"query":""}
< {"reply":"failed","reason":"bad request: no newline in the first 1 MiB"}
```

A `wire full` block pads each `>` line with spaces to 1 MiB with its newline, the most a request line may hold, and expects the `<` line as a `wire` block does.

```wire full
> {"request":"status"}
< {"reply":"summon","state":"closed"}
```

### Quit finishes the work in flight

A `wire fifo` block makes `$RUNTIME/runs` a FIFO and fills it before the daemon starts, so a program the daemon starts waits in the runner until the test reads the FIFO. A number before `>` or `<` names a connection, and the test opens every one before the first line. After each `>` line the test waits until the request takes effect: a reply comes, the daemon holds the recorder open once more, or the socket is gone. At a `record:` line it checks that no connection still waiting for its `<` line has a reply, and then reads the line from the FIFO, which lets that program finish. After the last line it expects the daemon to exit 0 within 2.5 s of the last `record:` line. That is half of Quit's 5 s, so a Quit that waits out its 5 s fails the block.

Lock Screen waits in the runner while Quit removes the socket. A request on a connection opened before Quit fails, and Quit replies only after Lock Screen's reply.

```wire fifo
1> {"request":"activate","key":"cmd:lock-screen","via":{"chord":"enter"},"confirmed":false,"query":""}
2> {"request":"activate","key":"cmd:quit","via":{"chord":"enter"},"confirmed":false,"query":""}
3> {"request":"status"}
3< {"reply":"failed","reason":"the daemon is quitting"}
record: ["loginctl","lock-session"]
1< {"reply":"done","message":"ran cmd:lock-screen"}
2< {"reply":"done","message":"ran cmd:quit"}
```

Quit takes no result after its 5 s. A `wire fifo` block runs no shell line, so it cannot write the 64 MiB file that makes each result take seconds to record. `quit_stops_taking_results_at_its_deadline` in `crates/lyncast/tests/daemon.rs` runs the setup lines of the session with that file, holds 64 of connection 1's Lock Screens in the runner, sends connection 2's Quit, and lets the 64 finish at once. Quit's reply has to come within 30 s. Each Lock Screen gets connection 1's reply or connection 3's, at least one gets connection 3's, and the daemon exits 0.

A second `wire fifo` block sends Quit after Lock Screen's reply. That Quit finds no work in flight, so only the write of Lock Screen's visit holds it up, and the daemon exits within the same 2.5 s.

```wire fifo
1> {"request":"activate","key":"cmd:lock-screen","via":{"chord":"enter"},"confirmed":false,"query":""}
record: ["loginctl","lock-session"]
1< {"reply":"done","message":"ran cmd:lock-screen"}
2> {"request":"activate","key":"cmd:quit","via":{"chord":"enter"},"confirmed":false,"query":""}
2< {"reply":"done","message":"ran cmd:quit"}
```
