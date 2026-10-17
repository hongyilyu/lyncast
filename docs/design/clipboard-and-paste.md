# Clipboard history copies now and keeps one seam for paste

In the MVP, Enter on a clip or calculator row copies the entry and closes the palette. Paste must plug in later per platform without a redesign. The [decision log](../decisions.md#clipboard-and-paste) is the source of truth and overrides this page. Hongyi decided capture types, the clipboard screen, previews, and retention. A probe settled the marker. The rest are lead defaults.

The evidence is [notes/07](../research/notes/07-paste-seam-workflow.md), with three design candidates, a judge, and two reviews. Claims keep its evidence labels.

## A capture thread owns the Watcher

In `wl-clipboard-rs` 0.9.4, `Watcher::next_event` blocks in `poll` with no timeout, and the `Offer` it returns borrows the `Watcher` (`src/watch.rs:308-315`, read in source). So a separate capture thread owns the `Watcher`, applies the three filters below, and sends entries over a channel.

## History keeps text, images, and files

The thread captures text, images, and files. Files arrive as `text/uri-list` or `x-special/gnome-copied-files` and point at the original paths. lyncast derives links and emails from text on read, as TinyCast does. Unpinned entries expire after 3 months by default, and pinned entries never expire. Source is the app that niri IPC reports as focused when the selection changed. It is a best guess, because data-control names no source client.

## Three filters keep own writes, passwords, and large entries out

The watcher lists each offer's MIME types, so each filter is a string compare (`src/watch.rs:193-198`, read in source).

The first filter skips lyncast's own writes, which all carry a type that starts with `application/x-lyncast-own`. Each copy offers `application/x-lyncast-own;copy=<pid>.<n>`, where `<pid>` is the daemon's process ID and `<n>` the copy's number, and the marker holds the same bytes as the payload. A probe under Xvfb plus anvil read 12 copies of `image/png` with an empty marker through `MimeType::Any`, and 5 reads got the marker and 0 bytes (measured). `wl-clipboard-rs` keeps offered types in a `HashMap`, so a reader that takes the first type can pick the marker (`src/copy.rs:264`, read in source).

The second filter skips the password-manager hint `x-kde-passwordManagerHint` (`src/utils.rs:21`, read in source). The third skips any entry over 25 MiB.

## The clipboard screen follows TinyCast's

Tab, or the Clipboard History command, opens the clipboard screen. Hongyi chose TinyCast's layout ([decision log](../decisions.md#clipboard-and-paste)):

- A filter input at the top. Its text filters entries, and the selection lands on the newest unpinned entry.
- A type filter with All Types, Text Only, Images Only, Files Only, Links Only, and Emails Only. Links and emails are kinds derived from text, so the filter reads them from the same rows.
- Rows grouped by day, newest first, with pins above the groups.
- A preview pane. Text shows its content. Images show a preview capped in size, decoded from the blob only for the selected row. Files show their icon, plus an image preview when the file is an image. The MVP renders no PDFs or documents. Reading the freedesktop thumbnail cache (`~/.cache/thumbnails`) is the first follow-up.
- An information panel with Source, Type, Path (files only), Size, and Copied (the capture time).
- A footer with the default action and Enter, plus Actions and Ctrl+K. The Actions menu offers Copy, Pin or Unpin, and Delete.

The preview decodes one image at a time and drops it when the selection moves. That keeps image memory bounded by one preview, not by the history size. Vicinae reached 1.2 to 2.7 GB while scrolling images on niri, as reported in its issue #998 ([notes/01](../research/notes/01-landscape.md)).

## The capture thread reads one representation per offer

The thread takes each offer the `Watcher` reports through the filters above, in order. An offer that carries the marker is lyncast's own write: the thread records nothing, and when the marker names this daemon's process, it tells the copier the copy's number, which is how a copy learns that the compositor took it. Another lyncast process numbers its copies from 1 too, so its marks confirm nothing here. An offer with the password hint is dropped. Every other offer gives one representation:

1. Files, when the offer has `text/uri-list` or `x-special/gnome-copied-files` and the list names a local file. The thread reads `text/uri-list` first. It keeps each `file://` line whose host is empty or `localhost`, and drops the rest, such as comments, other schemes, and the `copy` or `cut` that starts a GNOME list. A list that names no local file, such as a link offered as `text/uri-list`, gives an image or text instead.
2. Else an image: `image/png` when it is offered, else the first `image/*` type. An empty image is dropped.
3. Else text, through wl-clipboard-rs's order of text types. Empty text and text that is not UTF-8 are dropped.

A read stops after 25 MiB and one byte, and the reads of one offer stop after 2 s in all, however many reads the offer takes. Either way the entry is dropped. A 26 MiB offer stopped in 21 ms, and a stopped source's read returned at the 2 s deadline in 10 of 10 runs, after which the next copy was still captured ([the step 6 prototype](../research/notes/11-clipboard-prototype.md), measured).

The thread runs only when the daemon has `WAYLAND_DISPLAY`. A daemon that cannot reach the compositor it names prints `lyncast: not started: <error>` and exits 1 before it binds the socket. When the `Watcher` returns an error later, the compositor has gone. The daemon then stops as Quit does, without a reply to send, prints `lyncast: the Wayland connection closed`, and exits 1. After anvil stopped, `next_event` failed within 29 ms and failed again at once on every later call, so a loop that retried would spin (measured).

The thread reads, and it never writes or syncs, so only an offer's reads can hold it up, for 2 s at most. The `Watcher` takes in one go the offers that came while a read held the thread, and each of those holds it 2 s more. Once Quit has begun and no job is out, the daemon cancels the `Watcher`, and the thread ends after the offer it is reading and reads none of those that wait behind it. The writer of the clips ends after that, so every entry the thread handed over reaches it.

## Clips keeps the rows in memory and in sqlite

`Clips` is the only owner of clip rows, and core ranks a borrowed slice, because a second copy went out of sync in review. It holds every row in memory and loads them at start from `$XDG_DATA_HOME/lyncast/clips.sqlite3`. A row keeps its id, kind, type, size, times, pin, the SHA-256 of what it holds, and a preview: the first 1,024 bytes of its text or of its paths, cut at a character boundary. Whether a text is a link or an email comes from its whole text when it arrives. The database keeps the kind, the hash, and the preview beside the bytes, so a load reads no text, derives nothing again, and holds at most the previews in memory. The full text lives only in the database, and an image's bytes live in `$XDG_DATA_HOME/lyncast/blobs/<sha256 hex>`, one file per distinct image.

The daemon never waits on the disk, as with `ranking.json`. One writer thread owns `clips.sqlite3` and `blobs/`, and each change to a row goes to it in order. A change that fails prints `lyncast: <path>: not saved: <error>`. A pin, an unpin, or a promotion that fails leaves the row as it is in memory, and a delete that fails leaves the row in the file for the next daemon. A new row that fails, its blob or its row, goes from the history too, and its warning ends with `; the clip was not recorded`, so the history never lists a row whose bytes are nowhere, and a later capture of the same bytes records it again. A new row brings its bytes. For an image, the writer writes the blob through a temporary file named with the process ID, and then the row, and a blob that fails to write keeps its row out of the database. A delete removes the row first and then the blob, once no saved row names it, so a crash between the two leaves a blob that no row names, never a row whose blob is gone. A copy reads a row's text or blob through the writer, so a row that the writer has not written yet reads too. The data directory and `blobs/` are created with mode 0700, and `clips.sqlite3` and each blob with mode 0600, because the history holds whatever the user copied, secrets among them. Without an absolute data home, the rows and what they hold live in memory only.

New rows wait for the writer with their bytes. A new row whose bytes would bring what waits past 64 MiB is not recorded, and the daemon prints `lyncast: <path>: not saved: 64 MiB of clips wait for the disk; the clip was not recorded`, so a stalled disk cannot grow the daemon without bound.

The writer thread opens and loads `clips.sqlite3`, and the start waits 5 s at most for it. The writer reads the file only when it is a regular file after symlinks, and creates it when there is none, so a FIFO or a device there fails at once. A file that is not a regular file, a symlink to nothing, a file that SQLite cannot read, and a file whose ids have run out each print `lyncast: <path>: <error>; the clips stay in memory`. The rows then live in memory only, and the daemon never writes the file. SQLite opens the file, and the journal beside it, by path after that check, and an open of a FIFO waits for good, so a load that has not ended in 5 s prints `lyncast: <path>: not loaded within 5 s; the clips stay in memory` too. Its thread writes nothing when it ends. The daemon holds SQLite's exclusive lock until its writer ends, so a second daemon with the same data home finds the database locked and keeps its rows in memory.

The daemon gives each new row the next id itself, one above every id the database has used, so a reply can name the row before the database has it, and the id of a deleted row never comes back. Neither does the id of a new row that the file did not save: the next change that the file saves raises its count of used ids to that id. A database whose ids leave no next one is one the daemon cannot use, and once the ids run out in memory, a new row is not recorded and the daemon prints `lyncast: no clip id is left; the clip was not recorded`. A capture or a recorded write whose kind and bytes match a row promotes that row: it moves to the front and keeps its id. A pinned row stays where it is. The `Watcher`'s first event is the current selection, so without this every daemon start would add the clipboard again (measured, 23 of 23).

The screen order is pins first, in the order they were pinned, then the rest by last use, newest first. A pin is a use. Every insert, promotion, and pin takes the next number in one sequence, so two in the same millisecond keep their order. Unpinning keeps the row's number, so the row goes back where its pin put it.

Unpinned rows expire 90 days after their last use, and pins never expire. Unpinning starts a row's age again, so a row pinned for longer than that does not expire as soon as it is unpinned. `[clipboard] retention-days` in `$XDG_CONFIG_HOME/lyncast/config.toml` sets the days, and 0 keeps rows forever. The daemon prunes at start and after each insert. The daemon reads `config.toml` only when it is a regular file of at most 1 MiB after symlinks, so a FIFO there cannot hold up the start. A `config.toml` that cannot be read, does not parse, or gives `retention-days` a value that is not a whole number from 0 up prints `lyncast: <path>: <error>; using the defaults` and leaves the defaults. The daemon leaves every other key alone.

## A copy waits for the compositor to take it

`lyncast-core` says what to hand over, never how:

```rust
pub enum Delivery { Clip(ClipId), Answer(CalcAnswer), Text(String) }
```

`Clip` names a stored row, so core carries no bytes. `Answer` is a calculator answer, which only the calculator can build, and `Text` is a path that Copy Path hands over. The daemon hands each copy to the host, which passes it to the copier thread, so the daemon never waits for it. `Capture::start` starts the copier with the capture thread, which reports each marked offer to it. The copier runs one copy at a time and numbers each one, and the daemon records copies in the order the compositor took them:

1. The copier reads the row's bytes through the writer: its text from the database, or its blob. A blob that is gone fails the copy with `clip <id>: blob missing`, before anything is offered. A read that the writer has not answered within 5 s fails the copy with `not copied: the writer of the clips did not answer within 5 s`, so a stalled disk holds up each copy, and each copy behind it, 5 s at most.
2. It prepares a source on a thread of its own, with the payload's types and the marker `application/x-lyncast-own;copy=<pid>.<n>`, which holds the same bytes. wl-clipboard-rs prepares it with a roundtrip to the compositor, and it sends the `set_selection` it queues only once it serves the source.
3. If the source is ready within 500 ms, the copier lets the thread serve it. Otherwise the thread never serves it, so the selection never reaches the compositor, and the copy fails with `not copied: the compositor did not take the selection`. The thread still waits for the compositor with the payload, and until it ends, a new copy fails at once with `not copied: an earlier copy still waits for the compositor`, so a stalled compositor holds one payload and one thread, not one for each copy. A prepare that fails fails the copy with `not copied: <error>`.
4. Once the source is served, the copy waits up to 2.5 s for the capture thread to report the marked offer with its number. That is the 2 s the capture thread gives an offer, the longest one offer can hold the report up, and a margin. Then the daemon records the write and replies. A `Clip` promotes its row. An `Answer` and a `Text` insert a text row, or promote one with the same text. A report that does not come in time fails the copy with `not copied: the clipboard watcher did not confirm the selection`, though the compositor may hold the selection. Either way nothing is recorded. A number from a copy that gave up goes by, and only this process's marks count, so neither a late report nor another process's mark confirms another copy.

The wait is measured. Across processes under load, a read right after `copy_multi` returned the previous selection 173 times in 3,000, and 0 times in 6,000 after the wait. The wait took 0.3 ms at the median and 237 ms at most, and the first marked offer after a write was that write in 6,000 of 6,000 tries.

A text row offers `text/plain;charset=utf-8`, which wl-clipboard-rs also offers as its other text types. An image offers its stored type. A file entry offers its paths as `text/plain;charset=utf-8`, and its list as `text/uri-list` and as `x-special/gnome-copied-files`. wl-clipboard-rs offers the types in no set order, so a reader that takes the first type may get any of them. It fills its other text types from the first text source it is given (`src/copy.rs:861-897`, read in source). The paths come first, so those types hold the paths, the marker holds them too, and a paste into a text field gets them.

If the write fails, the palette stays open, shows the reason, and keeps the row.

## lyncast clip list shows the history

`lyncast clip list --json` prints one JSON object per row, in screen order:

| Field | Value |
|---|---|
| `id` | the clip id |
| `kind` | `text`, `link`, `email`, `image`, or `files` |
| `pinned` | `true` or `false` |
| `mime` | the stored type |
| `bytes` | the stored size |
| `preview` | the row's preview, or `null` for an image |

`link` and `email` are text whose whole trimmed content is one `http://` or `https://` URL with no whitespace, or one address of the form `local@domain.tld`. They are derived from the whole text when the row arrives, and the database keeps the kind. `lyncast clip list` sends the `clips` request, and the daemon answers with `{"reply":"clips","clips":[...]}`, each clip as the table spells it. `lyncast activate clip:<id>` copies the row, and `--verb pin`, `--verb unpin`, and `--verb delete` change it. They print `copied clip <id>`, `pinned clip <id>`, `unpinned clip <id>`, and `deleted clip <id>`.

Source is filled from plan step 8, which brings the niri IPC client. Until then it is empty.

## The fixtures

The tests in `crates/lyncast/tests/clipboard.rs` read the `env` and `session` blocks below, with the harness of [daemon.md](daemon.md#the-fixtures): a fresh `$TREE` and `$RUNTIME` for each block, the `env` block below, `session` lines, `stderr:`, `exit:`, and `record:` lines, and shell lines through `/bin/sh`. `crates/lyncast-core/tests/clip.rs` reads the `retention`, `kind`, and `uris` blocks.

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

A `session` block runs with no compositor, as Tier 1 does. Its shell lines add rows to `clips.sqlite3` with `sqlite3` while no daemon runs, because a running daemon holds the file's lock. A row's columns are its id, kind, type, size, text, the SHA-256 of what it holds in hex, preview, creation time, last use, pin, and sequence number.

### Without a compositor the history still answers

With no `WAYLAND_DISPLAY`, no capture thread runs and a copy fails, but the stored rows load and change. The first daemon creates the database, which only the user may open, in a directory that only the user may list. The rows load as the database keeps them, with their kinds and previews: a link, an email, a file list's paths, and an image without a preview. Pins come first in the order they were pinned, and an unpinned row goes back to the place its pin gave it. A pin, an unpin, and a delete close the summon and record no visit, so no `ranking.json` appears. The changes reach the database, so the next daemon shows the same list.

```session
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate clip:1 --verb pin
stderr: lyncast: no clip 1
exit: 1
$ lyncast activate cmd:quit
ran cmd:quit
$ stat -c %a data/lyncast data/lyncast/clips.sqlite3
700
600
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1)"
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (2, 'link', 'text/plain;charset=utf-8', 23, ' https://example.org/a ', '90eec24f494163b854df68f6fcb8feab4755335c0c73119a27c7ee52974cea47', ' https://example.org/a ', 0, unixepoch() * 1000, 0, 2)"
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (3, 'email', 'text/plain;charset=utf-8', 14, 'me@example.org', '855c94f3498940b241d6b5e78bb6f9293b941953b2799f8b925bd25180571d77', 'me@example.org', 0, unixepoch() * 1000, 0, 3)"
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (4, 'files', 'text/uri-list', 38, 'file:///etc/hostname' || char(10) || 'file:///tmp/a%20b', '34584a5c97220e11a055a677ac6f7554c88cc378dd609350fa15c13d8b19f772', '/etc/hostname' || char(10) || '/tmp/a b', 0, unixepoch() * 1000, 0, 4)"
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (5, 'image', 'image/png', 69, null, '2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20', null, 0, unixepoch() * 1000, 0, 5)"
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":5,"kind":"image","pinned":false,"mime":"image/png","bytes":69,"preview":null}
{"id":4,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":38,"preview":"/etc/hostname\n/tmp/a b"}
{"id":3,"kind":"email","pinned":false,"mime":"text/plain;charset=utf-8","bytes":14,"preview":"me@example.org"}
{"id":2,"kind":"link","pinned":false,"mime":"text/plain;charset=utf-8","bytes":23,"preview":" https://example.org/a "}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ lyncast toggle
open
$ lyncast activate clip:3 --verb pin
pinned clip 3
$ lyncast status
closed
$ lyncast activate clip:1 --verb pin
pinned clip 1
$ lyncast clip list --json
{"id":3,"kind":"email","pinned":true,"mime":"text/plain;charset=utf-8","bytes":14,"preview":"me@example.org"}
{"id":1,"kind":"text","pinned":true,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
{"id":5,"kind":"image","pinned":false,"mime":"image/png","bytes":69,"preview":null}
{"id":4,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":38,"preview":"/etc/hostname\n/tmp/a b"}
{"id":2,"kind":"link","pinned":false,"mime":"text/plain;charset=utf-8","bytes":23,"preview":" https://example.org/a "}
$ lyncast toggle
open
$ lyncast activate clip:3 --verb unpin
unpinned clip 3
$ lyncast status
closed
$ lyncast activate clip:1 --verb pin
stderr: lyncast: clip:1 has no action pin
exit: 1
$ lyncast toggle
open
$ lyncast activate clip:5 --verb delete
deleted clip 5
$ lyncast status
closed
$ lyncast activate clip:5
stderr: lyncast: no clip 5
exit: 1
$ lyncast activate clip:2
stderr: lyncast: not copied: no Wayland compositor
exit: 1
$ lyncast activate calc:2+3
stderr: lyncast: not copied: no Wayland compositor
exit: 1
$ lyncast clip list --json
{"id":1,"kind":"text","pinned":true,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
{"id":3,"kind":"email","pinned":false,"mime":"text/plain;charset=utf-8","bytes":14,"preview":"me@example.org"}
{"id":4,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":38,"preview":"/etc/hostname\n/tmp/a b"}
{"id":2,"kind":"link","pinned":false,"mime":"text/plain;charset=utf-8","bytes":23,"preview":" https://example.org/a "}
$ lyncast activate cmd:quit
ran cmd:quit
$ ls data/lyncast
clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "select id, pinned, seq from clips"
1|1|7
2|0|2
3|0|6
4|0|4
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":1,"kind":"text","pinned":true,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
{"id":3,"kind":"email","pinned":false,"mime":"text/plain;charset=utf-8","bytes":14,"preview":"me@example.org"}
{"id":4,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":38,"preview":"/etc/hostname\n/tmp/a b"}
{"id":2,"kind":"link","pinned":false,"mime":"text/plain;charset=utf-8","bytes":23,"preview":" https://example.org/a "}
```

### Old rows expire at start

Row 1 was last used 91 days ago, row 2 89 days ago, and row 3 is a pin last used in 1970. With `retention-days = 0` every row stays. A config that does not parse warns, and the default of 90 days removes row 1. Unpinning row 3 starts its age again, so with `retention-days = 1` the next start removes row 2 and keeps row 3.

```session
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 3, 'old', 'cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4', 'old', 0, (unixepoch() - 91 * 86400) * 1000, 0, 1)"
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (2, 'text', 'text/plain;charset=utf-8', 6, 'recent', '034a7e52c5c9534b709dc1dba403868399b0949f7c1933a67325c22077ffc221', 'recent', 0, (unixepoch() - 89 * 86400) * 1000, 0, 2)"
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (3, 'text', 'text/plain;charset=utf-8', 6, 'pinned', '3fab5c181bd28a09b64397df76ae2bfaf1eac182979b5fdb7a342858004f36af', 'pinned', 0, 0, 1, 3)"
$ mkdir -p config/lyncast && printf '[clipboard]\nretention-days = 0\n' > config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":3,"kind":"text","pinned":true,"mime":"text/plain;charset=utf-8","bytes":6,"preview":"pinned"}
{"id":2,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":6,"preview":"recent"}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":3,"preview":"old"}
$ lyncast activate cmd:quit
ran cmd:quit
$ printf '[clipboard]\nretention-days = "ninety"\n' > config/lyncast/config.toml
$ lyncast daemon --headless
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/config/lyncast/config.toml: invalid type: string "ninety", expected u32 at line 2 column 18; using the defaults
$ lyncast clip list --json
{"id":3,"kind":"text","pinned":true,"mime":"text/plain;charset=utf-8","bytes":6,"preview":"pinned"}
{"id":2,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":6,"preview":"recent"}
$ lyncast activate clip:3 --verb unpin
unpinned clip 3
$ lyncast activate cmd:quit
ran cmd:quit
$ printf 'theme = "dark"\n\n[clipboard]\nretention-days = 1\n' > config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":3,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":6,"preview":"pinned"}
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id from clips"
3
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/config/lyncast/config.toml: invalid type: string "ninety", expected u32 at line 2 column 18; using the defaults
```

The daemon reads `config.toml` only when it is a regular file of at most 1 MiB after symlinks, and it never opens a FIFO there, so a FIFO cannot hold up the start. A FIFO, a directory, a symlink to nothing, and a file over 1 MiB each warn, and the daemon starts with the defaults.

```session
$ mkdir -p config/lyncast && mkfifo config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rm config/lyncast/config.toml && mkdir config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rmdir config/lyncast/config.toml && ln -s missing.toml config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rm config/lyncast/config.toml && truncate -s 1048577 config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/config/lyncast/config.toml: not a regular file; using the defaults
lyncast: $TREE/config/lyncast/config.toml: not a regular file; using the defaults
lyncast: $TREE/config/lyncast/config.toml: No such file or directory (os error 2); using the defaults
lyncast: $TREE/config/lyncast/config.toml: over 1 MiB; using the defaults
```

A `retention` line gives a row's last use in days before now, whether it is pinned, and `retention-days`, and says whether the row expires. The core predicate reads these lines, and the store applies the same predicate at start and after each insert.

```retention
used 89d ago, unpinned, keep 90 => kept
used 90d ago, unpinned, keep 90 => kept
used 91d ago, unpinned, keep 90 => expired
used 400d ago, pinned, keep 90 => kept
used 400d ago, unpinned, keep 0 => kept
used 2d ago, unpinned, keep 1 => expired
```

### A database the daemon cannot use stays as it is

A file that SQLite cannot read, a FIFO, a symlink to nothing, and a file whose largest id leaves no next one each leave the rows in memory. The daemon never opens the FIFO, writes over the file, or creates the symlink's target. SQLite opens the journal beside the file itself, after the daemon's check, so a FIFO there holds the load, and after 5 s the daemon starts with its rows in memory and leaves the file as it was.

```session
$ mkdir -p data/lyncast && echo not a database > data/lyncast/clips.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cat data/lyncast/clips.sqlite3
not a database
$ rm data/lyncast/clips.sqlite3 && mkfifo data/lyncast/clips.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rm data/lyncast/clips.sqlite3 && ln -s missing.sqlite3 data/lyncast/clips.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ ls data/lyncast
clips.sqlite3
$ rm data/lyncast/clips.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (9223372036854775807, 'text', 'text/plain;charset=utf-8', 3, 'max', '9baf3a40312f39849f46dad1040f2f039f1cffa1238c41e9db675315cfad39b6', 'max', 0, unixepoch() * 1000, 0, 1)"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ mkfifo data/lyncast/clips.sqlite3-journal
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ rm data/lyncast/clips.sqlite3-journal
$ sqlite3 data/lyncast/clips.sqlite3 "select id, preview from clips"
9223372036854775807|max
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: file is not a database; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: not a regular file; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: No such file or directory (os error 2); the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: no clip id is left after 9223372036854775807; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: not loaded within 5 s; the clips stay in memory
```

### A second daemon finds the history locked

A second daemon with its own `XDG_RUNTIME_DIR` and the same data home keeps its clips in memory. Once the first daemon quits, the next one loads the database.

```session
$ lyncast daemon --headless
$ mkdir $RUNTIME/two
$ XDG_RUNTIME_DIR=$RUNTIME/two lyncast daemon --headless
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: database is locked; the clips stay in memory
$ XDG_RUNTIME_DIR=$RUNTIME/two lyncast activate cmd:quit
ran cmd:quit
$ lyncast activate cmd:quit
ran cmd:quit
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: database is locked; the clips stay in memory
```

### A text's kind and a file list's paths

A `kind` line gives a whole text and the kind it shows as. A `uris` line gives one line of a `text/uri-list` or `x-special/gnome-copied-files` list and the local path it names, or nothing after `=>` when it names none. Either side may write `\u{XXXX}` for a character.

```kind
alpha => text
https://example.org/a => link
http://example.org => link
\u{20}https://example.org/a\u{A} => link
https://example.org/a b => text
https:// => text
ftp://example.org => text
me@example.org => email
\u{9}me@example.org\u{A} => email
me@example => text
@example.org => text
me@.org => text
me@example. => text
me@you@example.org => text
me @example.org => text
```

```uris
file:///etc/hostname => /etc/hostname
file://localhost/etc/hostname => /etc/hostname
file:///etc/hostname\u{D} => /etc/hostname
file:///tmp/a%20b => /tmp/a b
file:///tmp/%FF => /tmp/\u{FFFD}
file:///tmp/a%2 => /tmp/a%2
file:///tmp/a%zz => /tmp/a%zz
file:///tmp/a%+1 => /tmp/a%+1
file://example.org/etc/hostname =>
file:/etc/hostname =>
https://example.org/a =>
# a comment =>
copy =>
```

## The daemon owns the selection and must stay alive

smithay forwards each paste read to the client that set the selection. When that client destroys its source or exits, smithay clears the selection, and niri keeps no copy (read in source). A probe saw the selection empty when the copier exited (measured).

The palette cannot own it, because GPUI's Wayland clipboard needs a focused window and an input serial (read in source). The resident daemon owns it through a source that wl-clipboard-rs prepares and that the daemon serves from a thread of its own (`src/copy.rs:555-585`, read in source). `--headless` does the same.

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

Four had no fix in the log or the spec. Plan step 6 settled three on 2026-10-17 ([decision log](../decisions.md#clipboard-and-paste)):

- No test covers Enter, close, and still serving. Both reviews ask for a Tier 2 case, which plan step 7 adds with the palette.
- `deliver` returned before the compositor applied the selection. A copy now waits for its own marked offer, as [a copy waits for the compositor to take it](#a-copy-waits-for-the-compositor-to-take-it) says.
- Back-to-back seeding races the watcher. A Tier 2 test waits for each seed's row before it seeds again.
- A daemon restart empties the clipboard. The daemon does not offer the newest row again at start.
