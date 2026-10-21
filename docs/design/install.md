# lyncast starts with the niri session

This page describes how lyncast is installed and started, for a maintainer and for the tests. The [decision log](../decisions.md#install) is the source of truth. Where this page disagrees with it, the log wins.

A systemd user unit runs `lyncast daemon` for as long as the niri session runs, and a niri bind runs `lyncast toggle`. Both name the binary by its installed path, `~/.local/bin/lyncast`, so neither depends on a `PATH`. The systemd user manager has a `PATH` of its own, `/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin` on the dev box, and `systemd-analyze verify` fails a unit whose `ExecStart` is a bare `lyncast daemon` with `Command lyncast is not executable` (both measured on the dev box, 2026-10-21). niri expands a leading `~` in a program it spawns (niri v26.04 `src/utils/spawning.rs:104-113`, read in source).

## The unit runs the palette daemon while niri runs

```systemd dist/systemd/lyncast.service
[Unit]
Description=lyncast launcher daemon
Documentation=https://github.com/hongyilyu/lyncast
PartOf=graphical-session.target
After=graphical-session.target
Requisite=graphical-session.target

[Service]
Type=exec
ExecStart=%h/.local/bin/lyncast daemon
Restart=on-failure
# lyncast exits 1 when it stops on an error it reports, such as another daemon
# holding the socket or the compositor going away, where a restart would fail
# the same way. A panic on its main thread exits 101 and a signal kills it,
# and both restart.
RestartPreventExitStatus=1
RestartSec=1

[Install]
WantedBy=niri.service
```

`systemctl --user enable lyncast.service` links the unit into `niri.service.wants`, so it starts with `niri.service` and never with another desktop, which would offer the daemon neither layer shell nor data control. `niri.service` orders itself before `graphical-session.target` (niri v26.04 `resources/niri.service:3-4`), and `niri --session` imports `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_CURRENT_DESKTOP`, `XDG_SESSION_TYPE`, and `NIRI_SOCKET` into the user manager before it tells systemd that it is ready (`src/main.rs:215-217, 236-241, 276-283`), all read in source. So `After=graphical-session.target` starts the daemon with them. `Requisite=` refuses a start outside a session, and `PartOf=` stops the daemon with the session. niri's own example units use the same three lines (niri v26.04 wiki `Example-systemd-Setup.md:21-30`, read in source).

`Type=exec` makes the start fail when the binary cannot run. The daemon binds its socket before GPUI connects to the compositor ([palette](palette.md#lyncast-daemon-runs-the-palette)), and it accepted its first connection 2.8 to 3.9 ms after its fork in 5 starts under anvil (measured on the dev box, 2026-10-21). So a toggle that comes before the socket is bound can happen only in the first moments of a login, and `Type=notify` would buy nothing that matters. niri also removes `NOTIFY_SOCKET` from what it spawns (`src/utils/spawning.rs:130-131`, read in source), so a `spawn-at-startup` daemon could not notify anyway.

The unit restarts the daemon only after a crash:

| How the daemon ends | Exit | The unit |
|---|---|---|
| Quit lyncast | 0 | stays down |
| Another daemon holds `lyncast.lock` | 1 | fails once |
| The compositor is unreachable at the start | 1 | fails once |
| The compositor goes | 1 | fails once, and the session's stop follows |
| A panic on the main thread | 101 | restarts after 1 s |
| A signal other than SIGTERM, SIGINT, SIGHUP, or SIGPIPE | killed | restarts after 1 s |
| `systemctl --user stop`, or the session's end | SIGTERM | stays down |

`Restart=on-failure` restarts after a non-zero exit, an unclean signal, or a timeout, and `RestartPreventExitStatus=1` takes exit 1 out of that (systemd.service(5) of systemd 252 on the dev box). In a private `systemd --user` on the dev box with a stand-in for niri, a SIGKILL restarted the daemon, Quit left it down, a second daemon beside it failed once, and the end of the compositor left the unit failed with no restart. Without `RestartPreventExitStatus=1`, the unit restarted a second daemon 5 times in about 5.5 s before it gave up, and each start failed with `lyncast: a daemon is already running` (all measured on 2026-10-21).

After a logout the unit shows `failed (Result: exit-code)`, because the daemon sees the compositor go before the session's stop reaches it. That state does not block the next session's start, even when a stand-in for `niri-session` leaves out its `systemctl --user reset-failed` (measured in the private manager, 2026-10-21), and `niri-session` runs that command before each session anyway (niri v26.04 `resources/niri-session:32-33`, read in source).

The daemon has no SIGTERM handler, so `systemctl --user stop` ends it at once, without the wait that Quit gives the last writes. Each write of `ranking.json` and `clips.sqlite3` lands whole or not at all ([daemon](daemon.md#visits-persist-in-rankingjson), [clipboard](clipboard-and-paste.md#clips-keeps-the-rows-in-memory-and-in-sqlite)), so a stop loses at most the visits and clips still queued.

A panic on any other thread ends that thread alone, because the workspace's `Cargo.toml` sets no `panic` strategy and lyncast installs no panic hook. The daemon can go on without the thread, and then the unit has nothing to restart. Nothing joins the clipboard's capture thread, for one, so a panic there would stop the history while the unit shows `active (running)` (each read in source, 2026-10-21). The [decision log](../decisions.md#install) records both this and the SIGTERM handler as follow-ups.

The unit's cgroup holds the daemon alone. The daemon sends every program it starts to niri as a `Spawn` request ([launching](launching.md#the-runner-sends-each-program-to-niri)), and niri puts each one in a scope of its own, so a stop or a restart of the unit closes no app.

Under `niri-session`, which imports the login shell's whole environment into the user manager (niri v26.04 `resources/niri-session:13-22, 35-36`, read in source), the daemon sees the shell's `PATH`, `XDG_DATA_DIRS`, and `LANG`. A variable that the login sets wins over the value that `~/.config/environment.d/*.conf` gives it. When `niri.service` starts some other way, the daemon gets only the user manager's own variables, niri's five, and what `environment.d` sets. An `Environment=` line in a drop-in of the unit wins over both, once the unit restarts (each measured in the private manager, 2026-10-21). The app scan reads `XDG_DATA_DIRS` and the locale variables ([ranking](ranking.md)), so for a palette that misses apps or names them in another language, the README's troubleshooting gives the unit those variables with `systemctl --user edit lyncast.service`.

## The niri file binds Mod+Space and blurs the palette

```kdl dist/niri/lyncast.kdl
// lyncast for niri 26.04 or later. Include this file at the end of
// ~/.config/niri/config.kdl with `include optional=true "lyncast.kdl"`, then
// run `niri validate`.

binds {
    Mod+Space repeat=false hotkey-overlay-title="Open lyncast" { spawn "~/.local/bin/lyncast" "toggle"; }
}

layer-rule {
    match namespace="^lyncast$"

    // The palette's corner radius in logical pixels, which niri cuts the blur to.
    geometry-corner-radius 12

    background-effect {
        blur true
        // Uncomment to blur the windows under the palette, not only the
        // wallpaper. niri marks that experimental, since the blur disappears
        // while the palette opens and closes.
        // xray false
    }
}
```

The file is included, never pasted, because a second `binds` node in one file fails to parse. An include after the user's own binds replaces a `Mod+Space` of theirs instead of failing, since a later bind replaces an earlier one with the same keys (niri v26.04 `niri-config/src/lib.rs:219-231`, wiki `Configuration:-Include.md:203-221`, read in source). A config that fails to parse at niri's start is replaced by niri's default config, which has no lyncast bind (`src/main.rs:150-156`, read in source), so the README runs `niri validate`. The include is `optional=true`, so a `config.kdl` that still names `lyncast.kdl` after the file is gone loads with a warning and keeps the user's other binds (`Configuration:-Include.md:123-144`, read in source). niri-config parsed each of these cases as this paragraph says (measured on the dev box with niri-config built from the v26.04 tag, 2026-10-21).

niri repeats a held bind by default (`Configuration:-Key-Bindings.md:65-70`, read in source), and `repeat=false` keeps a held `Mod+Space` from opening and closing the palette over and over. The rule's namespace is anchored, because niri matches it as a regular expression anywhere in a surface's namespace (`Configuration:-Layer-Rules.md:64-76`, read in source). The palette's surface carries `palette::NAMESPACE`, `lyncast`, on the Overlay layer (`WAYLAND_DEBUG=1` under anvil, 2026-10-21), and `the_niri_file_matches_the_window` checks the rule against that constant.

Blur comes from niri, because GPUI speaks only the KDE blur protocol, which niri lacks ([decision log](../decisions.md#lead-defaults-hongyi-can-override)). niri shows a background effect only through a translucent surface (`Window-Effects.md:11`, read in source), so the palette paints its background at 90% alpha ([palette](palette.md#the-window-lets-niris-blur-through)). niri cuts a rule's blur to the rule's `geometry-corner-radius` (`src/layer/mapped.rs:238-253`, read in source), so the rule's radius is the palette's, `theme::RADIUS`, which the same test checks. niri's wiki says that the setting affects only the shadow of a layer surface (`Configuration:-Layer-Rules.md:186`), and the source, which hands it to the blur, wins here. With niri's default xray, which any background effect turns on, the blur shows the wallpaper and not the windows under the palette. `xray false` blurs those windows at more cost, and niri calls it experimental (`Window-Effects.md:56, 61-70`, read in source).

`background-effect` in a layer rule is new in niri 26.04 (`Configuration:-Layer-Rules.md:231-233`), and v25.11's `niri-config/src/layer_rule.rs` has no such field, both read in source. A node that niri does not know fails the whole config, so lyncast needs niri 26.04 or later.

## The README installs the files

The README's install steps are this block:

```install
cargo build --release --locked
install -Dm755 target/release/lyncast ~/.local/bin/lyncast
install -Dm644 dist/systemd/lyncast.service ~/.config/systemd/user/lyncast.service
install -Dm644 dist/niri/lyncast.kdl ~/.config/niri/lyncast.kdl
test -e ~/.config/niri/config.kdl || cp /etc/niri/config.kdl ~/.config/niri/config.kdl
test -e ~/.config/niri/config.kdl && { grep -qxF 'include optional=true "lyncast.kdl"' ~/.config/niri/config.kdl || printf '\ninclude optional=true "lyncast.kdl"\n' >> ~/.config/niri/config.kdl; }
niri validate
systemctl --user daemon-reload
systemctl --user enable lyncast.service
```

The steps add the `include` line to `~/.config/niri/config.kdl` once, however often they run. niri reads `/etc/niri/config.kdl` while `~/.config/niri/config.kdl` does not exist, and at its start it writes its default config to `~/.config/niri/config.kdl` when neither exists (niri v26.04 `niri-config/src/lib.rs:558-629`, `src/main.rs:151`). A running niri loads the user's file as soon as it appears (`src/utils/watcher.rs:127-136, 151-156`), all read in source. A line appended to a missing file would thus give the session a config that holds only the include, with no bind but `Mod+Space`. So the steps copy the system file there first and append only to a file that exists. With neither file, the copy fails and the steps leave `config.kdl` missing, as the `cli` block below shows, and the README says to start niri once and run them again. The README also says not to log out when `niri validate` fails, since niri would then start with its default config. `install` replaces a binary that the daemon is running, and `systemctl --user restart lyncast.service` then runs the new one (measured in the private manager, 2026-10-21). The steps put each shipped file back whenever they run, so the README says that an `xray false` edit to `~/.config/niri/lyncast.kdl` has to be made again after an upgrade.

Where niri does not run as `niri.service`, the README offers this line for `config.kdl` instead:

```startup
spawn-at-startup "~/.local/bin/lyncast" "daemon"
```

niri runs it once it is ready, with the environment it gives a bind (`src/main.rs:250-255`), but discards its output (`src/utils/spawning.rs:116-120`) and never restarts it, both read in source. Beside the unit, the second daemon to start exits 1 with `lyncast: a daemon is already running` ([daemon.md](daemon.md#one-daemon-owns-the-socket)), and the unit does not restart it.

Each shipped command runs the installed binary with arguments that it takes. The bind's `toggle` asks the daemon, and the unit's and the `startup` line's `daemon` start the palette daemon, which needs a compositor ([palette](palette.md#lyncast-daemon-runs-the-palette)). niri discards a bind's output too, so a `Mod+Space` without a daemon fails with nothing to show, and the README's troubleshooting starts at `systemctl --user status lyncast.service`.

## Hongyi's niri machine has to answer these

The dev box has no niri, no seat, and no GPU but lavapipe, so these are unknown, not claims. [The check below](#the-check-on-hongyis-niri-machine) runs each one.

- **A fresh session starts the daemon, and Mod+Space opens the palette.** This is plan step 10's check. The private manager proved the unit's lines against a stand-in that imports niri's variables as `niri --session` does, not against niri.
- **niri-session's wait.** `niri-session` runs `systemctl --user --wait start niri.service` (`resources/niri-session:46-47`), and `--wait` needs a D-Bus connection to the user manager, which the private manager had none of, so its stand-in for `niri-session` polled instead.
- **The blur.** anvil draws no blur, so the blur on radv, the radius of its corners against the palette's, and the choice between xray and `xray false` are for Hongyi's eyes.
- **The environment.** Which of the login shell's variables reach the daemon depends on how the display manager starts `niri-session`.
- **polkit.** The unit changes nothing for Suspend, Reboot, and Power Off, which niri spawns into a scope of its own ([launching](launching.md#hongyis-niri-machine-has-to-answer-these)). That check stays where it is.

## The check on Hongyi's niri machine

Plan step 10 is done when a fresh niri session starts the daemon and `Mod+Space` opens the palette. Run these steps in the root of a checkout of master that holds this section and the README's Install section, and send back every output verbatim. The outputs go into `docs/research/notes/14-install-on-niri.md`, with the machine's facts from the first step, the commit, which checks passed, the blur that Hongyi picked, and anything that differed from these steps. Plan step 10 is then marked done, and the note is linked from the docs index and from this section.

### Check the session first

```sh
git log -1 --format='%H %s'
niri --version
systemctl --version | head -1
systemctl --user is-active niri.service graphical-session.target
tr '\0' ' ' < /proc/$(pgrep -xo niri)/cmdline; echo
systemctl --user is-active lyncast.service; pgrep -a lyncast
ls -l ~/.config/niri/config.kdl; grep -n 'Mod+Space\|lyncast' ~/.config/niri/config.kdl
```

niri must be 26.04 or later, both units `active`, and niri's command line `niri --session`. lyncast must not run yet. If `pgrep` lists one, quit it with `activate cmd:quit`, run with the path that `pgrep` printed, such as `~/.local/bin/lyncast activate cmd:quit`, and run the block's `pgrep` line again until it lists nothing. If `niri.service` is not active, niri did not start through `niri-session`, and the unit cannot start. Record that, and run [the check without the unit](#without-niriservice) instead. If `config.kdl` is missing, the install steps copy `/etc/niri/config.kdl` there. Note any `Mod+Space` bind of your own, which the include replaces.

### Install as the README says

Run the README's install steps from the repository's root. `niri validate` must log `config is valid` (`src/main.rs:102-107`). If it fails, stop and record the error, and do not log out, since niri would start with its default config. `systemctl --user enable` must print a `Created symlink` line for `~/.config/systemd/user/niri.service.wants/lyncast.service`. Then check the wiring without a start:

```sh
systemctl --user show -p WantedBy -p Type -p Restart -p RestartPreventExitStatus lyncast.service
tail -n 2 ~/.config/niri/config.kdl
```

It must show `Type=exec`, `Restart=on-failure`, `RestartPreventExitStatus=1`, and `WantedBy=niri.service`, and the last line of `config.kdl` must be the `include` line.

### Log out and in

Log out with `niri msg action quit`, or `Mod+Shift+E`, and log in again through the display manager's niri session. Do not start lyncast by hand. Then, in a terminal:

```sh
systemctl --user status lyncast.service
systemctl --user show -p NRestarts -p ExecMainStartTimestamp lyncast.service
systemctl --user show -p ActiveEnterTimestamp niri.service
journalctl --user -b -u lyncast.service --no-pager
tr '\0' '\n' < /proc/$(systemctl --user show -p MainPID --value lyncast.service)/environ | grep -E '^(WAYLAND_DISPLAY|NIRI_SOCKET|XDG_CURRENT_DESKTOP|PATH|XDG_DATA_DIRS|LANG)='
~/.local/bin/lyncast status
```

The unit must be `active (running)`, with `NRestarts=0` and a main process of `/home/<you>/.local/bin/lyncast daemon`. Its start must come within a second or two of niri's. The journal must hold the start and no line from lyncast, which prints nothing on a clean start. The environment must hold `WAYLAND_DISPLAY` and `NIRI_SOCKET`, and `lyncast status` must print `closed`.

### Press Mod+Space

Run this, and press `Mod+Space` while it sleeps:

```sh
sleep 3; ~/.local/bin/lyncast status; niri msg layers
```

It must print `open`, and `niri msg layers` must list the palette on the Overlay layer, where niri prints each surface's lines under its layer's line, with a blank line after the layer's last surface (`src/ipc/client.rs:221-262`, read in source):

```text
  Overlay layer:
    Surface:
      Namespace: "lyncast"
      Keyboard interactivity: exclusive
```

The palette is 750 by 480 logical pixels on the focused output, centered, a third of the way down the space it leaves. It is dark with rounded corners, and the wallpaper shows through it blurred. The field has the caret, and typing filters the list. Then check these, and take a screenshot of the open palette:

1. Escape closes the palette.
2. `Mod+Space` twice opens it and closes it again.
3. A held `Mod+Space` opens it once and does not flicker.
4. Enter on an app launches it through niri.
5. `2+3*4` shows a card with 14, and Enter copies it.

### Pick the blur

Open the palette over a window with clear detail, and note what shows through. Then uncomment `xray false` in `~/.config/niri/lyncast.kdl`, which niri reloads, run `niri validate`, and open the palette again. Note the blur of the windows beneath and anything odd while the palette opens and closes. Record which of the two Hongyi wants, and put the file back with `install -Dm644 dist/niri/lyncast.kdl ~/.config/niri/lyncast.kdl` if the shipped one stays. If neither shows a blur, record that too.

### Crash, Quit, and a second daemon

```sh
systemctl --user kill --signal=KILL lyncast.service; sleep 3
systemctl --user show -p NRestarts -p ActiveState -p SubState lyncast.service
~/.local/bin/lyncast daemon; echo "exit $?"
systemctl --user show -p NRestarts -p ActiveState lyncast.service
~/.local/bin/lyncast activate cmd:quit; sleep 3
systemctl --user show -p NRestarts -p ActiveState -p Result lyncast.service
systemctl --user start lyncast.service
```

The kill must leave `NRestarts=1`, `ActiveState=active`, and `SubState=running`. The second daemon must print `lyncast: a daemon is already running` and `exit 1`, and leave the unit as it was. Quit must leave `NRestarts=1`, `ActiveState=inactive`, and `Result=success`, and the last line brings the daemon back.

### A second fresh session

Log out and in once more, and run the first block of the log-in check again. The unit must be `active (running)` with `NRestarts=0`. That shows that the enable holds and that the `failed` state that the logout left does not block the next start. `journalctl --user -u lyncast.service --no-pager -n 20` must show `lyncast: the Wayland connection closed` from the logout, before the new start.

### Without `niri.service`

If `niri.service` was not active in the first step, run the README's install steps all the same, add the README's `spawn-at-startup` line to `~/.config/niri/config.kdl`, run `niri validate`, and log out and in. Then, in a terminal:

```sh
pgrep -a lyncast
~/.local/bin/lyncast status
~/.local/bin/lyncast daemon; echo "exit $?"
```

`pgrep` must list one `/home/<you>/.local/bin/lyncast daemon`, `lyncast status` must print `closed`, and the second daemon must print `lyncast: a daemon is already running` and `exit 1`. Then run [Press Mod+Space](#press-modspace) and [Pick the blur](#pick-the-blur) as they stand. The unit's checks, its restart after a kill, and its second session do not apply, because niri never restarts a daemon that `spawn-at-startup` started, so record them as skipped.

## The fixtures

The tests read the blocks above:

- `the_install_files_are_the_doc_blocks` in `crates/lyncast/tests/install.rs` reads the `systemd` and `kdl` blocks. Each names a file in the repository, which must equal the block byte for byte.
- `the_readme_holds_the_install_steps` in the same file checks that README.md holds the `install` block in a `sh` fence and the `startup` block in a `kdl` fence.
- `every_cli_block_runs_as_the_doc_says` in the same file runs the `cli` block below, as testing.md's harness runs its own, with bash and an empty environment but these. `HOME` is a fresh directory, which the test removes once the block passes. The commands run in its `src`, which holds the `systemd` and `kdl` blocks under `dist/`, the `install` block as `install.sh` with `/etc/niri/` read as `~/etc/niri/`, so no step reads the machine's own niri config, and a stub at `target/release/lyncast` in place of a build. `PATH` starts with a `bin` in `HOME` whose `cargo`, `niri`, and `systemctl` are stubs too. Each stub prints its name and arguments and does nothing else, so no command reaches the developer's own user manager. `XDG_RUNTIME_DIR` is a fresh directory under `/tmp`, which `systemd-analyze` needs. Its path is short, because with a runtime directory of 92 bytes, `systemd-analyze` warned that the paths of the host's sockets in it were too long (measured on the dev box, 2026-10-21).
- `every_argv_step_runs_as_the_doc_says` in the same file reads the bind's `spawn` arguments from the `kdl` block, the unit's `ExecStart`, and the `startup` line. Each program, with `%h/` read as `~/`, must be where the `install` block puts the binary. It runs each command's arguments with the test's build, with only a fresh `XDG_RUNTIME_DIR`, as `$RUNTIME`, and with `--record` after a `daemon`'s, as every test daemon has, and expects what the `argv` block's step of the same name says.
- `the_unit_restarts_only_a_crash` in the same file checks that the unit's `Restart=` is `on-failure` and that its `RestartPreventExitStatus=` is the exit code of every refusal the docs show: the `argv` block's `unit` step, the `a daemon is already running` line of daemon.md, and the `the Wayland connection closed` line of palette.md.
- `the_niri_file_matches_the_window` in `crates/lyncast/src/palette/tests.rs` checks that the `kdl` block's one `match namespace=` is `palette::NAMESPACE` between `^` and `$`, and that its `geometry-corner-radius` is `theme::RADIUS`.

The steps install twice, leave the `include` line once, put each file in place with its mode, and put the shipped `lyncast.kdl` back over an `xray false` edit. `systemd-analyze --user verify` prints nothing for the installed unit, and the copy with `Descripton=` shows why the test expects nothing at all, since verify warns about the unknown key on stderr and still exits 0. Without a `config.kdl` of the user's own, the steps copy the system's and add the line to the copy. With neither file, run as a pasted block runs, without `-e`, the copy fails, and the steps leave `config.kdl` missing.

```cli
$ mkdir -p ~/.config/niri && printf 'binds {\n    Mod+T { spawn "foot"; }\n}\n' > ~/.config/niri/config.kdl
$ sh -e install.sh
cargo build --release --locked
niri validate
systemctl --user daemon-reload
systemctl --user enable lyncast.service
$ sed -i 's|// xray false|xray false|' ~/.config/niri/lyncast.kdl && grep -c '^ *xray false$' ~/.config/niri/lyncast.kdl
1
$ sh -e install.sh
cargo build --release --locked
niri validate
systemctl --user daemon-reload
systemctl --user enable lyncast.service
$ cat ~/.config/niri/config.kdl
binds {
    Mod+T { spawn "foot"; }
}

include optional=true "lyncast.kdl"
$ cd ~ && find .config .local -type f ! -name config.kdl -printf '%m %p\n' | sort
644 .config/niri/lyncast.kdl
644 .config/systemd/user/lyncast.service
755 .local/bin/lyncast
$ cmp ~/.local/bin/lyncast target/release/lyncast && cmp ~/.config/systemd/user/lyncast.service dist/systemd/lyncast.service && cmp ~/.config/niri/lyncast.kdl dist/niri/lyncast.kdl
$ systemd-analyze --user verify ~/.config/systemd/user/lyncast.service 2>&1
$ sed 's/^Description=/Descripton=/' ~/.config/systemd/user/lyncast.service > ~/typo.service && systemd-analyze --user verify ~/typo.service 2>&1 | grep -c Descripton; echo "verify exit ${PIPESTATUS[0]}"
1
verify exit 0
$ rm ~/.config/niri/config.kdl && mkdir -p ~/etc/niri && printf 'binds {\n    Mod+Q { quit; }\n}\n' > ~/etc/niri/config.kdl
$ sh -e install.sh
cargo build --release --locked
niri validate
systemctl --user daemon-reload
systemctl --user enable lyncast.service
$ cat ~/.config/niri/config.kdl
binds {
    Mod+Q { quit; }
}

include optional=true "lyncast.kdl"
$ rm ~/.config/niri/config.kdl ~/etc/niri/config.kdl && sh install.sh 2>&1 | sed "s|$HOME|~|"
cargo build --release --locked
cp: cannot stat '~/etc/niri/config.kdl': No such file or directory
niri validate
systemctl --user daemon-reload
systemctl --user enable lyncast.service
$ ls ~/.config/niri
lyncast.kdl
```

The shipped commands, each without a daemon and without a compositor:

```argv
$ bind
stderr: lyncast: no daemon is listening on $RUNTIME/lyncast.sock
exit: 1
$ unit
stderr: lyncast: not started: the palette needs a Wayland compositor; run lyncast daemon --headless without one
exit: 1
$ startup
stderr: lyncast: not started: the palette needs a Wayland compositor; run lyncast daemon --headless without one
exit: 1
```

`systemd-analyze` joins the packages that [testing](testing.md#packages-on-amazon-linux-2023) lists. No test builds niri's config parser, because niri-config is not on crates.io, and it links libinput, which the dev box has only as a runtime library. A fresh niri session on Hongyi's machine checks the parse, the bind, and the blur.
