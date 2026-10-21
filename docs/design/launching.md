# Launching goes through niri

This page describes how lyncast starts programs, for a maintainer and for the tests. The [decision log](../decisions.md#launching) is the source of truth. Where this page disagrees with it, the log wins.

## The runner sends each program to niri

Every program the daemon starts goes through one runner ([daemon.md](daemon.md#the-runner-starts-programs-and-tests-swap-in-a-recorder)): an app's Exec argv, a Desktop Action's, or a command's from daemon.md's table. The real runner sends the argv to niri over `$NIRI_SOCKET` as `Action::Spawn { command }`, in niri-ipc 26.4.0's serde types, so Firefox's Enter is the line `{"Action":{"Spawn":{"command":["firefox"]}}}`.

niri spawns the argv on a thread of its own. The child's stdin, stdout, and stderr are `/dev/null`, and it gets niri's environment and working directory, and an activation token that niri makes for each Spawn in `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`. When niri runs as a systemd service, it puts the child in a scope named `app-niri-<basename of argv[0]>-<pid>.scope` (niri v26.04 `src/input/mod.rs:696-699`, `src/utils/spawning.rs:64-83, 105-155, 416-439`, read in source). So lyncast passes no token, no scope, no environment, and no working directory, and the program gets niri's environment, not the daemon's. niri-ipc's `Action` has no variant that locks or suspends (`src/lib.rs:194-946`, read in source), so those commands are programs that niri spawns too.

niri answers `{"Ok":"Handled"}` once the action has run on its event loop (`src/ipc/server.rs:381-399`). It answers the same when the exec fails, which it only logs (`src/utils/spawning.rs:192-198`), and while the session is locked it drops a Spawn and still answers (`src/input/mod.rs:650-653`), both read in source. So `launched` and `ran` mean that niri took the request, not that the program started. The failures that lyncast can report are those of the exchange, below.

## The client sends one request per connection

The daemon reads `NIRI_SOCKET` once, at start, with the rest of its environment, and an empty value counts as unset. niri sets the variable for its children and imports it into the systemd user environment (`src/main.rs:196-200, 276-290`, read in source). Its path holds niri's process ID (`src/ipc/server.rs:79-83`), so lyncast never looks for a socket itself.

Each request takes a connection of its own, in `crates/lyncast/src/niri.rs`:

1. Connect to `$NIRI_SOCKET`.
2. Write one compact JSON line.
3. Read one line back.
4. Close.

One deadline of 2 s covers the connect, the write, and the read, and the reply line holds at most 1 MiB with its newline. niri reads a request line at a time on its event loop and answers each with one line (`src/ipc/server.rs:187-225`, read in source). Each step can stall:

- **The read.** A reply that comes one byte every 150 ms passes a timeout of 2 s on each read and takes 2.55 s in all, so only one deadline over the whole exchange fails it at 2 s. niri-ipc's own `Socket::send` sets no timeout (niri-ipc `src/socket.rs:51-61`, read in source), and against a niri that never answered it had not returned after 3 s (measured).
- **The connect.** niri accepts on its event loop (`src/ipc/server.rs:85-99`, read in source), so a niri whose loop is stuck accepts nothing. A listener's backlog held 4,097 connections, and after them a plain `UnixStream::connect` waited for good, while lyncast's connect, which sets the socket's send timeout first, failed at the deadline (measured). Clients that gave up and closed did not free the backlog (measured).
- **The write.** A request that does not fit the socket's buffer, to a niri that reads nothing, fails at the deadline too (measured with 4 MiB).

A failure prints `lyncast: <reason>` and exits 1, records no visit, and leaves the summon as it was ([daemon.md](daemon.md#activation-rebuilds-the-row-and-runs-one-item)):

| Case | Reason |
|---|---|
| `NIRI_SOCKET` is unset or empty | `not run: NIRI_SOCKET is not set` |
| The connect fails | `not run: <path>: <error>`, such as `No such file or directory (os error 2)`, or `Connection refused (os error 111)` for a socket whose niri has gone |
| The write or the read fails | `not run: <path>: <error>`, such as `Connection reset by peer (os error 104)` for a niri that closes the connection with the request unread |
| No whole answer before the deadline | `not run: niri did not answer within 2 s` |
| niri closes before a whole line | `not run: niri closed the connection without an answer` |
| niri answers `Err` | `not run: niri: <message>` |
| A line that is not a reply, or no newline in the first 1 MiB | `not run: niri sent a bad reply: <error>` |
| A reply to another request | `not run: niri sent a bad reply: not an answer to an action` |

niri reads a request before it runs it, so a request whose answer did not come in time may still have run.

## The fixtures

`crates/lyncast/tests/launching.rs` reads the blocks below and runs each `session` block with [daemon.md's harness](daemon.md#the-fixtures): a fresh `$TREE` and `$RUNTIME`, the `env` block below, and the same lines, plus these:

- `$ fake-niri` starts a fake niri at `$RUNTIME/niri.sock` that answers each request with `{"Ok":"Handled"}`. `$ fake-niri <line>` answers with that line instead, the rest of the line as it is, spaces and all. `$ fake-niri stall` reads each request and never answers, `$ fake-niri hangup` reads each request and closes the connection, `$ fake-niri reset` closes each connection unread once the request has come, `$ fake-niri drip <line>` answers with the line one byte every 150 ms, and `$ fake-niri flood` answers with 2 MiB and no newline. `$ fake-niri gone` leaves a socket file there that nothing listens on. Each `$ fake-niri` line replaces the fake before it, and the block's end stops the fake.
- The fake reads each request line as niri does and appends it to `$RUNTIME/niri.log` before it answers. `niri: <line>` after a command is a request line it read while the command ran, and with no `niri:` line it read none.
- A `lyncast daemon` line that sets `NIRI_SOCKET` gets no `--record`, so its daemon sends each program to the fake. The test fails such a line unless the value is `$RUNTIME/niri.sock` or empty, so no test reaches a real niri.

The fake is `crates/lyncast/tests/common/niri.rs`.

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

The tree holds daemon.md's Firefox:

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

### A launch sends one Spawn to niri

Plan step 8 is done when this block passes. Enter on Firefox sends its Exec, whose `%u` expands to nothing, and niri's `Handled` makes it a launch.

```session
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
niri: {"Action":{"Spawn":{"command":["firefox"]}}}
```

### A Desktop Action sends its own argv, and a launch records a visit

A verb picks the Desktop Action, and Ctrl+Enter does what Enter does. Each launch records a visit, which is written by the time Quit replies.

```session
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:firefox.desktop --verb launch-action:new-window
launched app:firefox.desktop new-window
niri: {"Action":{"Spawn":{"command":["firefox","--new-window"]}}}
$ lyncast activate app:firefox.desktop --chord ctrl-enter --query fir
launched app:firefox.desktop
niri: {"Action":{"Spawn":{"command":["firefox"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ sed -E 's/[0-9]+/N/g' data/lyncast/ranking.json
{"app:firefox.desktop":{"anchor":N,"last_opened":N,"terms":["fir"]}}
```

### Commands spawn their programs

A command that confirms still asks first and sends nothing. Open Config sends `xdg-open` once it has made the file. Reload Apps, Clipboard History, and Quit lyncast send nothing to niri.

```session
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
niri: {"Action":{"Spawn":{"command":["loginctl","lock-session"]}}}
$ lyncast activate cmd:suspend
ran cmd:suspend
niri: {"Action":{"Spawn":{"command":["systemctl","suspend"]}}}
$ lyncast activate cmd:reboot
stderr: lyncast: Reboot? Run again with --yes to confirm.
exit: 1
$ lyncast activate cmd:reboot --yes
ran cmd:reboot
niri: {"Action":{"Spawn":{"command":["systemctl","reboot"]}}}
$ lyncast activate cmd:power-off --yes
ran cmd:power-off
niri: {"Action":{"Spawn":{"command":["systemctl","poweroff"]}}}
$ lyncast activate cmd:log-out --yes
ran cmd:log-out
niri: {"Action":{"Spawn":{"command":["niri","msg","action","quit","--skip-confirmation"]}}}
$ lyncast activate cmd:open-config
ran cmd:open-config
niri: {"Action":{"Spawn":{"command":["xdg-open","$TREE/config/lyncast/config.toml"]}}}
$ lyncast activate cmd:reload-apps
ran cmd:reload-apps
$ lyncast activate cmd:clipboard-history
ran cmd:clipboard-history
$ lyncast activate cmd:quit
ran cmd:quit
```

### Each failure of niri fails the launch and changes nothing

Each activation below fails in its own way, and none records a visit or closes the summon. Where the fake read the request, the `niri:` line shows it.

```session
$ NIRI_SOCKET= lyncast daemon --headless
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: NIRI_SOCKET is not set
exit: 1
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast toggle
open
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: $RUNTIME/niri.sock: No such file or directory (os error 2)
exit: 1
$ fake-niri gone
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: $RUNTIME/niri.sock: Connection refused (os error 111)
exit: 1
$ fake-niri {"Err":"error parsing request"}
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: niri: error parsing request
exit: 1
niri: {"Action":{"Spawn":{"command":["firefox"]}}}
$ fake-niri not json
$ lyncast activate cmd:lock-screen
stderr: lyncast: not run: niri sent a bad reply: expected value at line 1 column 1
exit: 1
niri: {"Action":{"Spawn":{"command":["loginctl","lock-session"]}}}
$ fake-niri {"Ok":{"Version":"26.04"}}
$ lyncast activate cmd:suspend
stderr: lyncast: not run: niri sent a bad reply: not an answer to an action
exit: 1
niri: {"Action":{"Spawn":{"command":["systemctl","suspend"]}}}
$ fake-niri flood
$ lyncast activate cmd:reboot --yes
stderr: lyncast: not run: niri sent a bad reply: no newline in the first 1 MiB
exit: 1
niri: {"Action":{"Spawn":{"command":["systemctl","reboot"]}}}
$ fake-niri hangup
$ lyncast activate cmd:power-off --yes
stderr: lyncast: not run: niri closed the connection without an answer
exit: 1
niri: {"Action":{"Spawn":{"command":["systemctl","poweroff"]}}}
$ fake-niri reset
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: $RUNTIME/niri.sock: Connection reset by peer (os error 104)
exit: 1
$ lyncast status
open
$ lyncast activate cmd:quit
ran cmd:quit
$ test -e data/lyncast/ranking.json || echo no visits
no visits
```

### A niri that does not answer fails within 2 s

A niri that reads the request and stalls fails the launch at the deadline. So does a reply that comes too slowly in all: one byte every 150 ms takes 2.55 s, though each read waits only 150 ms, so a client with a timeout on each read would print `launched` here. Once the fake answers again, the next launch goes through.

```session
$ fake-niri stall
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: niri did not answer within 2 s
exit: 1
niri: {"Action":{"Spawn":{"command":["firefox"]}}}
$ fake-niri drip {"Ok":"Handled"}
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: niri did not answer within 2 s
exit: 1
niri: {"Action":{"Spawn":{"command":["firefox"]}}}
$ fake-niri
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
niri: {"Action":{"Spawn":{"command":["firefox"]}}}
```

A niri whose loop is stuck accepts no connection, and no block can fill a backlog of 4,097. So `a_niri_that_accepts_nothing_fails_at_the_deadline` in `crates/lyncast/src/niri.rs` listens with a backlog of 0, which holds one connection, fills it, and expects the action to fail with `not run: niri did not answer within 2 s`.
