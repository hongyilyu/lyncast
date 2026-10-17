# The daemon answers every request on one socket

This page describes the lyncast daemon and the CLI that talks to it, for a maintainer and for the tests. The [decision log](../decisions.md#daemon-and-cli) is the source of truth. Where this page disagrees with it, the log wins.

The palette and the CLI send the same requests. The CLI sends them over a Unix socket, and from plan step 7 the palette hands them to the daemon inside the process. So a CLI test takes the palette's path.

## Start the daemon with lyncast daemon

`lyncast daemon --headless` runs the daemon without GPUI. It takes the socket's lock, scans the apps, loads the visits, binds the socket, and then answers requests until a `Quit` command. `--headless` is required until plan step 7 builds the palette, the same way `--json` is required for `query`.

The daemon claims `$XDG_RUNTIME_DIR/lyncast.sock` in three moves:

1. It takes an exclusive `flock` on `$XDG_RUNTIME_DIR/lyncast.lock`. If another process holds the lock, it prints `lyncast: a daemon is already running` and exits 1. The lock comes before the scan, so a second daemon fails at once and never reads `ranking.json`. The open does not wait for a FIFO's reader, so a FIFO at that path fails the daemon at once. Any other lock file that is not a regular file, such as a link to a device, fails it with `not a regular file`.
2. Holding the lock, it removes any `lyncast.sock` left by a daemon that died without cleaning up, such as one killed with SIGKILL. Only the lock holder touches the socket, so two daemons that start at once cannot both bind.
3. It binds the socket and listens.

The kernel releases the lock when the process exits, however it exits. A `Quit` command also removes the socket. The lock file stays, because removing it would let a second daemon lock a new file while the first still holds the old one.

If the daemon cannot start the thread that accepts connections or the one that writes the visits, it prints `lyncast: not started: <error>` and exits 1.

Without `XDG_RUNTIME_DIR`, the daemon and every client print `lyncast: XDG_RUNTIME_DIR is not set` and exit 1, as Wayland clients refuse to run without it. A socket path holds at most 107 bytes, so the tests put `XDG_RUNTIME_DIR` under `/tmp`, as `scripts/test-env.sh` does.

The daemon reads its environment once, at start. The app scan reads `XDG_DATA_HOME`, `XDG_DATA_DIRS`, `PATH`, the locale variables, and `XDG_CURRENT_DESKTOP` there, with `HOME` in place of an unset `XDG_DATA_HOME`, and Reload Apps scans again with the same values. The visits use the same data home, and Open Config reads `XDG_CONFIG_HOME`, or `HOME` in its place, from the same start. A client's environment changes nothing but the socket path.

## The CLI sends one request and prints the reply

Each command connects, sends one request, prints the reply, and exits:

| Command | Request | Prints on success |
|---|---|---|
| `lyncast toggle` | `toggle` | the new summon state, `open` or `closed` |
| `lyncast status` | `status` | the summon state |
| `lyncast query <text> --json` | `query` | one JSON object per row, as [ranking](ranking.md#lyncast-query-prints-one-json-object-per-row) describes |
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

1. It rebuilds the row's `Subject` from current state. An app comes from the index, a command from its key, and a calculator row from evaluating the key's text again. If the state has no such row, the request fails, for example with `no app nope.desktop` or `calc:10/0 has no answer`. Clips come in plan step 6, so until then every clip key fails with `no clip <id>`.
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
| `Pin(clip)`, `Unpin(clip)`, `Delete(clip)` | `pinned clip <id>`, `unpinned clip <id>`, `deleted clip <id>`, from plan step 6 |

Starting a program, Open Config, and Reload Apps run on a worker thread, and the daemon answers other requests while they run. Their reply comes when the work ends. If the worker thread cannot start, the request fails at once with `not run: <error>`, nothing is recorded, and Quit does not wait for it. Two Reload Apps can overlap, and their scans can end in either order. Each scan has a number in the order it started. A scan's apps replace the index only when that scan started after the one the index came from, so a scan that ends after a newer one changes nothing.

The commands run like this:

| Command | Runs |
|---|---|
| Clipboard History | opens the summon. Plan step 7 shows the clipboard screen. |
| Open Config | creates `$XDG_CONFIG_HOME/lyncast/config.toml` if it is missing, then runs `xdg-open <path>` |
| Reload Apps | scans the apps again |
| Quit lyncast | stops taking requests, waits for the work in flight and the last write of the visits, gives up the lock, replies, and exits 0 |
| Lock Screen | `loginctl lock-session` |
| Suspend | `systemctl suspend` |
| Log Out | `niri msg action quit --skip-confirmation` |
| Reboot | `systemctl reboot` |
| Power Off | `systemctl poweroff` |

Log Out skips niri's own prompt, because lyncast has already asked. `XDG_CONFIG_HOME` defaults to `$HOME/.config`, as the XDG spec says. A relative value counts as unset, as the spec also says. Without an absolute `XDG_CONFIG_HOME` or `HOME`, Open Config fails with `not run: XDG_CONFIG_HOME and HOME are not set` and runs nothing.

Quit goes in this order:

1. It removes the socket, so no new client connects. A request that still arrives, on a connection opened before, gets `failed` with `the daemon is quitting` at once while steps 2 and 3 wait. One that arrives during step 4 waits until the process exits, up to 1 s, and then gets that reply or finds the connection closed. Either way it runs nothing.
2. It waits for the work in flight, such as a program the runner is starting, and sends each reply to its client. So a client exits 1 only when its program did not start.
3. It waits for the last write of the visits, gives up the lock, and replies. A client that runs after the reply finds no daemon, and a new daemon can start at once and load every visit.
4. It waits up to 1 s for its replies, Quit's among them, to be written, and exits 0. The exit closes every connection, so it would cut off a reply that is still being written.

Steps 2 and 3 wait 5 s at most in all. After that the daemon takes no more requests or results. A request or a job's result that is still queued gets `failed` with `the daemon is quitting`, even when the job's program started, and work still running gets no reply. So Quit takes 6 s at most, plus the time to handle the one request or result in hand when the 5 s run out. Work or a write still running, or a result that the daemon did not take, keeps the lock until the process exits, so a daemon started in its place never writes `ranking.json` at the same time.

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

The daemon owns one `Visits`. It loads `$XDG_DATA_HOME/lyncast/ranking.json` at start. After every recorded visit it leaves a copy of the visits in one slot, in place of any copy that one writer thread has not taken yet. The writer takes the newest copy, writes it to `ranking.json.<pid>.tmp` in the same directory, where `<pid>` is the daemon's process ID, and renames that over the file, so a crash leaves the old file or the new one. Two daemons with their own `XDG_RUNTIME_DIR` hold their own locks and can share a data home, so the process ID keeps each out of the other's temporary file, and each rename puts one daemon's whole copy in place. The daemon keeps answering while a write waits on the disk, and at most one copy waits in the slot meanwhile. Quit waits for the last write. `XDG_DATA_HOME` resolves as it does for the app scan: an absolute `XDG_DATA_HOME`, else `$HOME/.local/share`. With neither, the daemon keeps visits in memory only.

The file is one JSON object. Each member is a key with its visit, in key order:

```json
{"app:firefox.desktop":{"anchor":1760650000000,"last_opened":1760600000000,"terms":["fir"]}}
```

`anchor` and `last_opened` are milliseconds since the Unix epoch, from 1970 to the end of 9999. `terms` holds the last three distinct search terms in their fold, most recent first, and a term over 1,024 bytes is not kept. On load the daemon folds each stored term again, drops empty, long, and repeated terms, and keeps the first three, so a file edited by hand still follows that rule. A member whose key no longer parses is dropped.

The daemon reads only a regular file of at most 64 MiB, after symlinks. It checks the path first and never opens a FIFO or a device, because opening a device can change it, as opening a serial port resets the board on it. Then it opens the file without waiting for a FIFO's writer and checks the file it opened, so a FIFO swapped in after the first check cannot block it. A real file holds a few kilobytes. No entry at all is a fresh install, but a symlink to nothing is a file that cannot be read.

The daemon keeps a bad file for the user and never writes over it:

- If the file cannot be read, is not JSON of this shape, or holds a time outside that range, the daemon prints `lyncast: <path>: <error>; moved it to ranking.json.bad` on stderr, renames it, and starts with no visits.
- If `ranking.json.bad` is already there, the daemon does not move the new bad file over it. It prints `lyncast: <path>: <error>; not moved to ranking.json.bad: it already exists; the visits stay in memory`. If the rename fails, the line names the rename's error after `not moved to ranking.json.bad: `. Either way the daemon starts with no visits and writes no file until it exits.

The daemon never writes a file it would not read, so a copy over 64 MiB fails with `not saved: over 64 MiB`, and the file stays as it was. A write that fails prints `lyncast: <path>: not saved: <error>`, and the next visit writes the file again.

## The wire is one JSON line each way

A client connects, writes one request as compact JSON and a newline, and reads one reply line. Then both sides close. A connection holds no state, so the palette and many CLI clients can talk to the daemon at once. Keys travel as their strings and are parsed at the socket, and verbs and chords use the spellings of `query --json`.

A request line holds at most 1 MiB, with its newline. A line with no newline in its first 1 MiB gets `failed` with `bad request: no newline in the first 1 MiB`, and the daemon runs none of it, even when the bytes before the cut are a whole request. A request that does not parse gets a `failed` reply that names the error. Either way the daemon keeps serving.

Requests:

| Request | Fields |
|---|---|
| `{"request":"toggle"}` | |
| `{"request":"status"}` | |
| `{"request":"query","text":...}` | the query as typed |
| `{"request":"activate","key":...,"via":...,"confirmed":...,"query":...}` | `via` is `{"chord":"enter"}` or `{"verb":"pin"}` |

Replies:

| Reply | Fields |
|---|---|
| `{"reply":"summon","state":...}` | `open` or `closed` |
| `{"reply":"rows","rows":[...]}` | the rows, each as `query --json` prints it |
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

- A line that starts with `$ ` is a command. Words split as in [ranking's](ranking.md#lyncast-query-prints-one-json-object-per-row) `cli` blocks, and a command may set variables before `lyncast`.
- `$ lyncast daemon ...` with no expected lines after it starts the daemon in the background, with its stdout and stderr appended to `$RUNTIME/daemon.stdout` and `$RUNTIME/daemon.stderr`, and the test waits until the socket accepts. With expected lines, it runs to the end like any command.
- Each line after a command is what that command prints. A plain line is stdout. A line that starts with `stderr: ` is stderr. `exit: <n>` is the exit code, 0 when absent. `$TREE` and `$RUNTIME` in an expected line stand for the two paths.

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

### One daemon owns the socket

A second daemon finds the lock taken. Without a daemon, or without `XDG_RUNTIME_DIR`, every client fails.

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
$ lyncast daemon
stderr: error: the following required arguments were not provided:
stderr:   --headless
stderr: 
stderr: Usage: lyncast daemon --headless
stderr: 
stderr: For more information, try '--help'.
exit: 2
```

### The socket speaks one JSON line each way

A `wire` block sends each `>` line, with a newline, to a daemon started as a session starts one, each on a connection of its own. It expects the `<` line after it as the reply.

```wire
> {"request":"status"}
< {"reply":"summon","state":"closed"}
> {"request":"toggle"}
< {"reply":"summon","state":"open"}
> {"request":"query","text":"fire"}
< {"reply":"rows","rows":[{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}]}
> not json
< {"reply":"failed","reason":"bad request: expected ident at line 1 column 2"}
```

A `wire unterminated` block sends each `>` line twice, each time on a new connection. The first time it has no newline, and the test closes its half of the connection after it. The second time it is padded with spaces to 1 MiB, and `not json` and a newline follow. Both get the `<` reply. A toggle that ran would have replied with the summon state.

```wire unterminated
> {"request":"toggle"}
< {"reply":"failed","reason":"bad request: no newline in the first 1 MiB"}
```

A `wire full` block pads each `>` line with spaces to 1 MiB with its newline, the most a request line may hold, and expects the `<` line as a `wire` block does.

```wire full
> {"request":"status"}
< {"reply":"summon","state":"closed"}
```
