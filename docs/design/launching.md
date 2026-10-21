# Launching goes through niri

This page describes how lyncast starts programs, for a maintainer and for the tests. The [decision log](../decisions.md#launching) is the source of truth. Where this page disagrees with it, the log wins.

## The runner sends each program to niri

Every program the daemon starts goes through one runner ([daemon.md](daemon.md#the-runner-starts-programs-and-tests-swap-in-a-recorder)): an app's Exec argv, a Desktop Action's, or a command's from daemon.md's table. The real runner sends the argv to niri over `$NIRI_SOCKET` as `Action::Spawn { command }`, in niri-ipc 26.4.0's serde types, so Enter on a Firefox that the scan found at `/usr/bin/firefox` is the line `{"Action":{"Spawn":{"command":["/usr/bin/firefox"]}}}`. An app's argv is built on the way, [below](#the-daemon-builds-an-apps-command-on-a-worker).

niri spawns the argv on a thread of its own. The child's stdin, stdout, and stderr are `/dev/null`, and it gets niri's environment and working directory, and an activation token that niri makes for each Spawn in `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`. When niri runs as a systemd service, it puts the child in a scope named `app-niri-<basename of argv[0]>-<pid>.scope` (niri v26.04 `src/input/mod.rs:696-699`, `src/utils/spawning.rs:64-83, 105-155, 416-439`, read in source). So lyncast passes no token, no scope, no environment, and no working directory, and the program gets niri's environment, not the daemon's. niri-ipc's `Action` has no variant that locks or suspends (`src/lib.rs:194-946`, read in source), so those commands are programs that niri spawns too.

Log Out is the one request that starts no program. It sends `{"Action":{"Quit":{"skip_confirmation":true}}}`, which is what `niri msg action quit --skip-confirmation` sends, with no process in between. lyncast has asked already, so it skips niri's own "Press Enter to confirm" dialog (`src/input/mod.rs:660-667`, read in source). niri runs an action in an idle callback of its event loop and answers once the callback is done (`src/ipc/server.rs:381-399`), but Quit's callback stops the loop (`src/input/mod.rs:667`), and calloop runs the idle callbacks last in a turn and checks the stop before the next (calloop 0.14.5 `src/loop_logic.rs:627-635, 657-673`), so niri exits with the answer unwritten. That is read in source, not seen on a niri. So a connection that niri closes after it reads Quit is Log Out's success. A connection closed with the request unread fails the read with `Connection reset by peer` instead, as [the failures below](#each-failure-of-niri-fails-the-launch-and-changes-nothing) show, so the close shows that niri read the Quit.

niri answers every other action with `{"Ok":"Handled"}` once it has run. It answers the same when the exec fails, which it only logs (`src/utils/spawning.rs:192-198`), and while the session is locked it drops a Spawn and still answers (`src/input/mod.rs:650-653`), both read in source. So `launched` and `ran` mean that niri took the request, not that the program started. lyncast can report only the failures that it finds itself: a program, a terminal, or a directory that is not there ([below](#the-daemon-builds-an-apps-command-on-a-worker)), and an exchange with niri that fails.

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

## The daemon builds an app's command on a worker

An app's command is built on the worker thread that sends it, because building it reads the disk, and the daemon's methods never do ([daemon.md](daemon.md#activation-rebuilds-the-row-and-runs-one-item)). Field codes were expanded at scan time ([ranking](ranking.md)), and nothing expands them again. `crates/lyncast/src/launch.rs` takes three steps, in order:

1. **The program.** The app's argv[0], from its Exec or a Desktop Action's, is found the way the scan found it ([decision log](../decisions.md#app-index-and-ranking)). An absolute path must be a file this user may execute, and it goes as it is. A bare name is looked up first in the entry's `Path`, when that is absolute, and then in each `PATH` directory, with a relative `PATH` entry under `Path`, or under the working directory without one. A program found under a relative directory is made absolute against the daemon's working directory, where the scan looked. The daemon sends the absolute path, so the launch runs the program that the scan found, whatever niri's `PATH` holds. GLib resolves argv[0] with `PATH` and the entry's `Path` before it spawns too (GLib 2.82.2 `gio/gdesktopappinfo.c:2974-3000`), and niri expands only a leading `~` (`src/utils/spawning.rs:105-113`, both read in source). A program that is gone fails with `not run: no program <name>`, and one whose path is not UTF-8, which niri's Spawn cannot carry, with `not run: <path>: not UTF-8`. The commands' programs, `loginctl`, `systemctl`, and `xdg-open`, stay bare, for niri's `PATH`.
2. **The terminal.** `Terminal=true` puts a terminal in front of the program, found the way the program is: `terminal` under `[launch]` in `config.toml`, a list whose first word names the program, or else the first of `xdg-terminal-exec`, `foot`, `alacritty -e`, `kitty`, `ghostty -e`, `wezterm start --`, `gnome-terminal --`, `konsole -e`, and `xterm -e` that is found. The app's argv follows the terminal's words. An empty list counts as unset, a list that is not a list of strings makes `config.toml` one that does not parse, which warns and leaves every key at its default ([clipboard and paste](clipboard-and-paste.md#clips-keeps-the-rows-in-memory-and-in-sqlite)), and a configured terminal that is not there fails with `not run: no program <name>`. With none found the launch fails with `not run: no terminal for app:<id>`. A Desktop Action of a terminal app runs in the terminal too, as GLib runs it (`gdesktopappinfo.c:2701-2714, 2937-2943, 5193`, read in source).
3. **The working directory.** `Path=` wraps the command as `/bin/sh -c 'cd -- "$1" && shift && exec "$@"' lyncast <dir> <command>...`, because Spawn carries no working directory (niri-ipc `src/lib.rs:205-210`) and niri's children start in niri's (`src/utils/spawning.rs:115-120`, read in source). GLib spawns in `Path` (`gdesktopappinfo.c:3043`, read in source). A relative `Path` is made absolute against the daemon's working directory, where the scan read it. First the daemon checks that `<dir>` is a directory, and fails with `not run: <dir>: <error>` or `not run: <dir>: not a directory`. The script keeps every argument as it is, `--` keeps a directory named `-P` a directory, and a missing directory ends the shell with 1 before the exec.

An entry without `Path` goes unwrapped, so its scope keeps the app's name, and with the script niri names the scope after `sh`. The check and niri's spawn are two steps, so a directory removed between them makes the shell exit 1 where no one sees it.

## Hongyi's niri machine has to answer these

The dev box has no niri, no seat, and no terminal but `xterm`, so these are unknown, not claims. Each comes with the check to run on a niri session.

- **The terminals' conventions.** Whether `foot`, `alacritty -e`, `kitty`, `ghostty -e`, `wezterm start --`, and `xdg-terminal-exec` each run the argv after them, with an argument that holds a space and one that starts with `-`: `niri msg action spawn -- <terminal> <words> sh -c 'printf "[%s]" "$@" > /tmp/t8' x 'a b' -n` for each, then `cat /tmp/t8`, which should read `[a b][-n]`. A wrong convention starts an empty terminal, or the terminal reads the app's flags as its own, and niri sends the child's stderr to `/dev/null`.
- **niri's working directory.** niri's children start in niri's, which is `$HOME` when `niri.service` starts it, since the unit sets no `WorkingDirectory` (`resources/niri.service`, read in source), and a user unit without one starts in the user's home (systemd.exec(5) of systemd 252 on the dev box). Whether Hongyi's session starts niri through that unit is for his machine to say. `niri msg action spawn -- sh -c 'pwd > /tmp/cwd1'` should write `$HOME`, and the `Path=` script spawned the same way into `/usr/share` should write `/usr/share`.
- **Power from a niri scope.** When niri runs as `niri.service`, it puts its children in an `app-niri-*.scope` under `user@.service`, not in the login session, and otherwise they stay in niri's own cgroup (`src/utils/spawning.rs:416-419`, read in source). In a scope, polkit's rule for an active session may not cover `systemctl suspend`, `reboot`, and `poweroff`. `niri msg action spawn -- sh -c 'busctl call org.freedesktop.login1 /org/freedesktop/login1 org.freedesktop.login1.Manager CanPowerOff > /tmp/can'` should write `s "yes"`, and the call changes nothing. On the dev box, which has no seat, it gave `challenge` both from a shell and under `systemd-run --user --scope` (measured on 2026-10-21).
- **Quit's answer.** niri exits with Quit's answer unwritten, as [Log Out](#the-runner-sends-each-program-to-niri) reads in source, so lyncast takes niri's close as Log Out's success. From a terminal of the outer session, `NIRI_SOCKET=<a nested niri's socket> niri msg action quit --skip-confirmation; echo "rc $?"` should close the nested niri and, if niri closes without an answer, print `Error: error communicating with niri` and `rc 1` (inferred from `src/ipc/client.rs:60-111`), where `rc 0` would mean that niri answers. Either way, the close has to come within 2 s, or Log Out fails with `not run: niri did not answer within 2 s`.
- **Lock Screen.** `loginctl lock-session` only sends logind's `Lock` signal, so nothing locks unless a locker such as `swayidle -w lock swaylock` listens for it. `busctl --system monitor org.freedesktop.login1` in one terminal and `niri msg action spawn -- loginctl lock-session` from another should show the signal, and the locker should show.

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

It also holds an app with an absolute Exec, and ranking.md's shell that only its `Path` directory holds:

```desktop share/applications/true.desktop
[Desktop Entry]
Type=Application
Name=True
Exec=/bin/true
```

```desktop share/applications/workdir-shell.desktop
[Desktop Entry]
Type=Application
Name=Workdir Shell
TryExec=sh
Exec=sh
Path=/bin
```

There are apps with a relative `Path`: one under the tree, one that is `.`, one that is missing, and one that is a file.

```desktop share/applications/notes.desktop
[Desktop Entry]
Type=Application
Name=Notes
Exec=notes --new
Path=work
```

```desktop share/applications/dot.desktop
[Desktop Entry]
Type=Application
Name=Dot
Exec=notes
Path=.
```

```desktop share/applications/gone-dir.desktop
[Desktop Entry]
Type=Application
Name=Gone Dir
Exec=notes
Path=gone
```

```desktop share/applications/file-dir.desktop
[Desktop Entry]
Type=Application
Name=File Dir
Exec=notes
Path=bin/notes
```

```executable bin/notes
#!/bin/sh
```

There are two terminal apps, one of them with a `Path`, and terminals in directories that a block adds to `PATH`:

```desktop share/applications/htop.desktop
[Desktop Entry]
Type=Application
Name=htop
Exec=htop -d 10
Terminal=true
Actions=tree;

[Desktop Action tree]
Name=Tree View
Exec=htop --tree
```

```desktop share/applications/top-here.desktop
[Desktop Entry]
Type=Application
Name=Top Here
Exec=htop
Terminal=true
Path=work
```

```executable bin/htop
#!/bin/sh
```

```executable old/xterm
#!/bin/sh
```

```executable terms/foot
#!/bin/sh
```

```executable terms/myterm
#!/bin/sh
```

```executable xte/xdg-terminal-exec
#!/bin/sh
```

```executable t1/konsole
#!/bin/sh
```

```executable t2/gnome-terminal
#!/bin/sh
```

```executable t3/wezterm
#!/bin/sh
```

```executable t4/ghostty
#!/bin/sh
```

```executable t5/kitty
#!/bin/sh
```

```executable t6/alacritty
#!/bin/sh
```

### A launch sends one Spawn to niri

Plan step 8 is done when this block passes. Enter on Firefox sends its Exec, whose `%u` expands to nothing, with the program the scan found, and niri's `Handled` makes it a launch.

```session
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
```

### The recorder writes the line niri would read

`lyncast daemon --record <file>`, which the harness adds to a daemon line that sets no `NIRI_SOCKET`, swaps the real runner for a recorder ([daemon.md](daemon.md#the-runner-starts-programs-and-tests-swap-in-a-recorder)). The recorder appends the line that the real runner would write to niri and sends nothing, so a `record:` line and a `niri:` line are the same bytes.

```session
$ lyncast daemon --headless
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
record: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
$ lyncast activate cmd:log-out --yes
ran cmd:log-out
record: {"Action":{"Quit":{"skip_confirmation":true}}}
```

### A Desktop Action sends its own argv, and a launch records a visit

A verb picks the Desktop Action, and Ctrl+Enter does what Enter does. Each launch records a visit, which is written by the time Quit replies.

```session
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:firefox.desktop --verb launch-action:new-window
launched app:firefox.desktop new-window
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox","--new-window"]}}}
$ lyncast activate app:firefox.desktop --chord ctrl-enter --query fir
launched app:firefox.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ sed -E 's/[0-9]+/N/g' data/lyncast/ranking.json
{"app:firefox.desktop":{"anchor":N,"last_opened":N,"terms":["fir"]}}
```

### Commands spawn their programs, and Log Out quits niri

A command that confirms still asks first and sends nothing. Log Out runs when niri answers, or when niri closes the connection after it reads the Quit, but not when it closes the connection with the Quit unread. Open Config sends `xdg-open` once it has made the file. Reload Apps, Clipboard History, and Quit lyncast send nothing to niri.

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
niri: {"Action":{"Quit":{"skip_confirmation":true}}}
$ fake-niri hangup
$ lyncast activate cmd:log-out --yes
ran cmd:log-out
niri: {"Action":{"Quit":{"skip_confirmation":true}}}
$ fake-niri reset
$ lyncast activate cmd:log-out --yes
stderr: lyncast: not run: $RUNTIME/niri.sock: Connection reset by peer (os error 104)
exit: 1
$ fake-niri
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
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
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
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
$ fake-niri drip {"Ok":"Handled"}
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: niri did not answer within 2 s
exit: 1
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
$ fake-niri
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
```

### argv[0] is the program the scan found

The entry's `Path`, `/bin`, and `$TREE/bin` on `PATH` both hold `sh`, and the `Path` comes first, so the daemon sends `/bin/sh` and runs it in `/bin`. An absolute Exec goes as it is. The second daemon's `PATH` is the relative `bin`, so it finds Firefox under its working directory, `$TREE`, and sends the absolute path. A program removed after the scan fails before anything is sent, and once Reload Apps has scanned again, the app is gone. The last daemon finds Firefox only in a directory whose name is not UTF-8.

```session
$ printf '#!/bin/sh\n' > bin/sh && chmod 755 bin/sh
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:workdir-shell.desktop
launched app:workdir-shell.desktop
niri: {"Action":{"Spawn":{"command":["/bin/sh","-c","cd -- \"$1\" && shift && exec \"$@\"","lyncast","/bin","/bin/sh"]}}}
$ lyncast activate app:true.desktop
launched app:true.desktop
niri: {"Action":{"Spawn":{"command":["/bin/true"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=bin lyncast daemon --headless
$ lyncast activate app:firefox.desktop
launched app:firefox.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/bin/firefox"]}}}
$ rm bin/firefox
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: no program firefox
exit: 1
$ lyncast activate cmd:reload-apps
ran cmd:reload-apps
$ lyncast activate app:firefox.desktop
stderr: lyncast: no app firefox.desktop
exit: 1
$ lyncast activate cmd:quit
ran cmd:quit
$ mkdir "$(printf 'b\377n')" && printf '#!/bin/sh\n' > "$(printf 'b\377n')/firefox" && chmod 755 "$(printf 'b\377n')/firefox"
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/b\x{FF}n lyncast daemon --headless
$ lyncast activate app:firefox.desktop
stderr: lyncast: not run: $TREE/b�n/firefox: not UTF-8
exit: 1
```

### Path= runs the program in its directory

A relative `Path` is made absolute against the daemon's working directory, which is `$TREE` here. A directory that is missing, or a file, fails before anything is sent. The two shell lines run the script itself: it lands in the directory with every argument as it was, and a missing directory exits 1 before the exec.

```session
$ mkdir work
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:notes.desktop
launched app:notes.desktop
niri: {"Action":{"Spawn":{"command":["/bin/sh","-c","cd -- \"$1\" && shift && exec \"$@\"","lyncast","$TREE/work","$TREE/bin/notes","--new"]}}}
$ lyncast activate app:dot.desktop
launched app:dot.desktop
niri: {"Action":{"Spawn":{"command":["/bin/sh","-c","cd -- \"$1\" && shift && exec \"$@\"","lyncast","$TREE","$TREE/bin/notes"]}}}
$ lyncast activate app:gone-dir.desktop
stderr: lyncast: not run: $TREE/gone: No such file or directory (os error 2)
exit: 1
$ lyncast activate app:file-dir.desktop
stderr: lyncast: not run: $TREE/bin/notes: not a directory
exit: 1
$ /bin/sh -c 'cd -- "$1" && shift && exec "$@"' lyncast "$TREE/work" /bin/sh -c 'pwd; printf "[%s]" "$@"; echo' x 'a b' -n
$TREE/work
[a b][-n]
$ mkdir -- -P && /bin/sh -c 'cd -- "$1" && shift && exec "$@"' lyncast -P /bin/sh -c 'pwd'
$TREE/-P
$ /bin/sh -c 'cd -- "$1" && shift && exec "$@"' lyncast "$TREE/gone" /bin/true 2>/dev/null; echo "rc $?"
rc 1
```

### Terminal apps run in the first terminal found

With no terminal on `PATH`, the launch fails. Each of the next nine daemons adds to the end of `PATH` the directory of the next terminal up the list, and its launch runs in that terminal, with the terminal's words, though `PATH` lists it last: `xterm -e`, then `konsole -e`, `gnome-terminal --`, `wezterm start --`, `ghostty -e`, `kitty`, `alacritty -e`, `foot`, and `xdg-terminal-exec`. So each terminal comes before every one after it in the list. A Desktop Action runs in the terminal too, and `Path` wraps the terminal. A terminal in `config.toml` comes before every terminal in the list. An empty list counts as unset, and a terminal that is not a list warns and leaves the defaults. A configured terminal that is not there fails.

```session
$ mkdir work
$ fake-niri
$ NIRI_SOCKET=$RUNTIME/niri.sock lyncast daemon --headless
$ lyncast activate app:htop.desktop
stderr: lyncast: not run: no terminal for app:htop.desktop
exit: 1
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/old/xterm","-e","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1 lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/t1/konsole","-e","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1:$TREE/t2 lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/t2/gnome-terminal","--","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1:$TREE/t2:$TREE/t3 lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/t3/wezterm","start","--","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1:$TREE/t2:$TREE/t3:$TREE/t4 lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/t4/ghostty","-e","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1:$TREE/t2:$TREE/t3:$TREE/t4:$TREE/t5 lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/t5/kitty","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1:$TREE/t2:$TREE/t3:$TREE/t4:$TREE/t5:$TREE/t6 lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/t6/alacritty","-e","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1:$TREE/t2:$TREE/t3:$TREE/t4:$TREE/t5:$TREE/t6:$TREE/terms lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/terms/foot","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate app:htop.desktop --verb launch-action:tree
launched app:htop.desktop tree
niri: {"Action":{"Spawn":{"command":["$TREE/terms/foot","$TREE/bin/htop","--tree"]}}}
$ lyncast activate app:top-here.desktop
launched app:top-here.desktop
niri: {"Action":{"Spawn":{"command":["/bin/sh","-c","cd -- \"$1\" && shift && exec \"$@\"","lyncast","$TREE/work","$TREE/terms/foot","$TREE/bin/htop"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/t1:$TREE/t2:$TREE/t3:$TREE/t4:$TREE/t5:$TREE/t6:$TREE/terms:$TREE/xte lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/xte/xdg-terminal-exec","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ mkdir -p config/lyncast && printf '[launch]\nterminal = ["myterm", "--title", "lyncast"]\n' > config/lyncast/config.toml
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/terms:$TREE/xte lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/terms/myterm","--title","lyncast","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ printf '[launch]\nterminal = []\n' > config/lyncast/config.toml
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/old/xterm","-e","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ printf '[launch]\nterminal = "myterm"\n' > config/lyncast/config.toml
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/terms lyncast daemon --headless
$ lyncast activate app:htop.desktop
launched app:htop.desktop
niri: {"Action":{"Spawn":{"command":["$TREE/terms/foot","$TREE/bin/htop","-d","10"]}}}
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/config/lyncast/config.toml: invalid type: string "myterm", expected a sequence at line 2 column 12; using the defaults
$ printf '[launch]\nterminal = ["nope"]\n' > config/lyncast/config.toml
$ NIRI_SOCKET=$RUNTIME/niri.sock PATH=$TREE/bin:$TREE/old:$TREE/terms:$TREE/xte lyncast daemon --headless
$ lyncast activate app:htop.desktop
stderr: lyncast: not run: no program nope
exit: 1
```

A niri whose loop is stuck accepts no connection, and no block can fill a backlog of 4,097. So `a_niri_that_accepts_nothing_fails_at_the_deadline` in `crates/lyncast/src/niri.rs` listens with a backlog of 0, which holds one connection, fills it, and expects the action to fail with `not run: niri did not answer within 2 s`.
