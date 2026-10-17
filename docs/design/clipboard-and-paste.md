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

1. Files, when the offer has `text/uri-list` or `x-special/gnome-copied-files` and the list names a local file. The thread reads `text/uri-list` first. It keeps each `file://` line whose host is empty or `localhost`, and drops the rest, such as comments, other schemes, and the `copy` or `cut` that starts a GNOME list. A list that names no local file, such as a link offered as `text/uri-list`, gives an image or text instead, and so does a list that is not UTF-8, which names no file the thread can find.
2. Else an image: `image/png` when it is offered, else the first `image/*` type. An empty image is dropped.
3. Else text, through wl-clipboard-rs's order of text types. Empty text and text that is not UTF-8 are dropped.

A read stops after 25 MiB and one byte, and the reads of one offer stop after 2 s in all, however many reads the offer takes. Either way the entry is dropped. A 26 MiB offer stopped in 21 ms, and a stopped source's read returned at the 2 s deadline in 10 of 10 runs, after which the next copy was still captured ([the step 6 prototype](../research/notes/11-clipboard-prototype.md), measured).

The thread runs only when the daemon has `WAYLAND_DISPLAY`, and it needs a seat and ext- or wlr-data-control, which niri offers ([notes/03](../research/notes/03-wayland-niri-integration.md), read in source). A daemon that cannot reach the compositor it names, or whose compositor offers no seat or neither data-control protocol, prints `lyncast: not started: <error>` and exits 1 before it binds the socket. The daemon reaches its compositor only through `WAYLAND_DISPLAY`. wl-clipboard-rs connects through wayland-client, which takes the descriptor that `WAYLAND_SOCKET` names before it looks at `WAYLAND_DISPLAY` and removes the variable (wayland-client 0.31.15 `src/conn.rs:47-52`, read in source), so the `Watcher` could reach another compositor than the one that `WAYLAND_DISPLAY` names, or take a descriptor that the daemon holds for something else. So a start with `WAYLAND_SOCKET` set, even empty or not UTF-8, prints `lyncast: not started: WAYLAND_SOCKET is set; lyncast connects through WAYLAND_DISPLAY` and exits 1 before it connects to any compositor. When the `Watcher` returns an error later, the compositor has gone. The daemon then stops as Quit does, without a reply to send, prints `lyncast: the Wayland connection closed`, and exits 1. After anvil stopped, `next_event` failed within 29 ms and failed again at once on every later call, so a loop that retried would spin (measured).

The thread reads, and it never writes or syncs, so an offer holds it up for its reads, 2 s at most, and then for the work on what they read: the hash of the bytes, their copy into the entry, and a file list's preview, which decodes the paths only as far as it shows, inside a path too. For 25 MiB in a release build at a load average of 31 on 32 cores, that work took 66 ms at the median of 7 runs and 93 ms at most for text, 17 and 42 ms for an image, and 86 and 91 ms for a file list (measured). The `Watcher` takes in one go the offers that came while a read held the thread, and each of those holds it 2 s more. Once Quit has begun and no job is out, the daemon cancels the `Watcher`, and the thread ends after the offer it is reading and reads none of those that wait behind it. The writer of the clips ends after that, so every entry the thread handed over reaches it.

## Clips keeps the rows in memory and in sqlite

`Clips` is the only owner of clip rows, and core ranks a borrowed slice, because a second copy went out of sync in review. It holds every row in memory and loads them at start from `$XDG_DATA_HOME/lyncast/clips.sqlite3`. A row in memory keeps its id, kind, type, size, last use, pin, sequence number, the SHA-256 of what it holds, and a preview: the first 1,024 bytes of its text or of its paths, cut at a character boundary. The file keeps when each row was captured too. Whether a text is a link or an email comes from its whole text when it arrives. The database keeps the kind, the hash, and the preview beside the bytes, so a load reads no text and derives nothing again. Before it reads a row, it checks that each column it reads holds the type of value that the daemon writes there, an integer or text, and measures each row's kind, type, hash, and preview, so it holds no more of a row in memory than its integers, a kind of 5 bytes, a type of 65,535, a hash of 64, and a preview of 1,024. It checks too that an image holds no text and no preview, and that a row of every other kind holds both. The full text lives only in the database, and an image's bytes live in `$XDG_DATA_HOME/lyncast/blobs/<sha256 hex>`, one file per distinct image.

The daemon never waits on the disk, as with `ranking.json`. One writer thread owns `clips.sqlite3` and `blobs/`, and each change to a row goes to it in order. A change that fails prints `lyncast: <path>: not saved: <error>`. A pin, an unpin, or a promotion that fails leaves the row as it is in memory, and a delete that fails leaves the row in the file for the next daemon, unless a capture of the same kind and bytes comes first. The writer then removes the old row in the transaction that saves the new one, so the next daemon does not list those bytes twice, and an image's blob stays, named by the new row. A new row that fails, its blob or its row, goes from the history too, and its warning ends with `; the clip was not recorded`, so the history never lists a row whose bytes are nowhere. An image whose row fails after its blob was written takes the blob with it, unless a saved row names it, so the disk keeps no image that the history cannot list or delete. A capture of the same bytes after the failure records it again, and one that comes while the row still waits for the writer only promotes the row and goes with it. A new row brings its bytes. For an image, the writer writes the blob through a temporary file named with the process ID, and then the row, and a blob that fails to write keeps its row out of the database. A delete removes the row first and then the blob, once no saved row names it, so a crash between the two leaves a blob that no row names, never a row whose blob is gone. A copy reads a row's text or blob through the writer, so a row that the writer has not written yet reads too. The data directory and `blobs/` are created with mode 0700, and `clips.sqlite3` and each blob with mode 0600, because the history holds whatever the user copied, secrets among them. Without an absolute data home, the rows and what they hold live in memory only.

Entries wait with their bytes from the capture thread on, for the owner loop and then, as new rows, for the writer. Each takes its share of 64 MiB before the capture thread hands it over, and gives it back once the writer has applied its row, or once it goes without one, as a promotion does. An entry whose bytes would bring what waits past 64 MiB is not recorded, and the daemon prints `lyncast: <path>: not saved: 64 MiB of clips wait for the disk; the clip was not recorded`, so neither a stalled disk nor a busy owner loop can grow the daemon without bound. With the rows in memory, nothing bounds them, because the history holds every row's bytes. Each change waits for the writer too, and once 100 changes wait, the one the writer has in hand among them, a new change is refused before it changes the history. A pin, an unpin, or a delete then fails with `not saved: 100 changes wait for the disk`, and a capture or a recorded write is not recorded, with `lyncast: <path>: not saved: 100 changes wait for the disk; the clip was not recorded`. A capture that adds a row also removes the rows that have expired, which is a second change, so it is not recorded unless both fit.

The writer thread opens and loads `clips.sqlite3`, and the start waits 5 s at most for it. The writer reads the file only when it is a regular file after symlinks, and creates it empty, with mode 0600, when there is none, so a FIFO or a device there fails at once. The load itself writes nothing to the file. It takes SQLite's exclusive lock with a transaction that it rolls back, and on an empty file it keeps that transaction's journal in memory, so no journal appears beside the file either. Then it reads, and an empty file holds no row. The writer creates the table and sets lyncast's application id in one transaction before its first change, so the file stays empty until the daemon has something to save. The daemon holds the lock until its writer ends, so a second daemon with the same data home finds the database locked and keeps its rows in memory.

Until the daemon's first change, the file keeps its bytes unless SQLite rolls a crashed writer's journal back into it, and of the files that SQLite keeps beside it, the journal and the `-wal` change only as this list says, in rollback mode and in WAL mode alike. With a missing file that the writer creates, that is all that changes, whether the daemon uses the file or not.

- Beside a file in rollback mode that is not empty, SQLite takes any file at the journal's path whose first byte is not 0 for the journal of a writer that crashed, at the first read, as any SQLite client does. It rolls back what it can read of that journal, which restores the file to its last commit, and removes it. So a file there goes even when it is no journal, and the database then stays as it was.
- Beside an empty file, SQLite removes a journal or a `-wal` that holds anything at the first read, since neither can hold a commit of an empty file.
- Beside a file in WAL mode, an empty `-wal` goes as the load's connection closes, at once for a file that the daemon does not use and as the writer ends for one that it does. When there is none, as SQLite leaves a file that it closed cleanly, SQLite creates an empty one at the first read. A `-wal` that holds anything keeps its bytes, and its frames stay out of the file. The load turns off the checkpoint that SQLite makes as a connection closes unless the `-wal` is empty, and it looks only once it holds the lock, which keeps every other connection from writing to the `-wal`.

Each of these leaves the rows in memory only, and nothing changes but a missing file that the writer created empty and what SQLite rolls back or removes for the load. It prints `lyncast: <path>: <why>; the clips stay in memory`, with `<why>` as given:

- A file that is not a regular file after symlinks: `not a regular file`.
- A symlink to nothing: `No such file or directory (os error 2)`.
- A data directory or a file that the daemon cannot look up or create: the system's error, such as `Not a directory (os error 20)` when the data directory is a file, or `File exists (os error 17)` when it is a symlink to nothing.
- A file that SQLite cannot read, or that another daemon holds: SQLite's error, such as `file is not a database` or `database is locked`.
- A file that goes after the daemon's check, or turns into a symlink to nothing, before SQLite opens it: SQLite's `unable to open database file: <path>`, as SQLite opens the file without creating it. No block can time that, so `the_open_creates_no_file` in `crates/lyncast/src/clip/load.rs` opens a symlink to nothing.
- A file whose text encoding is not UTF-8, the one that the daemon writes: `a text encoding of UTF-16le, which the daemon does not write`, with SQLite's name of the encoding in its place.
- A file whose application id, the number in its header that names the program that made it, is not lyncast's: `an application id of 0, which the daemon does not write`, with the file's id in its place. SQLite leaves it 0 unless a program sets it.
- A file that holds a table, an index, a view, or a trigger besides those that `schema.sql` makes, the `clips` table and SQLite's count of used ids in `sqlite_sequence`: `a table that the daemon does not make`, or `an index`, and so on, in its place.
- A `clips` table whose SQL is not that of `schema.sql`, as when it lacks `STRICT`, `AUTOINCREMENT`, or a column, or a column compares text without regard to case: `a clips table that the daemon does not make`.
- A table of `schema.sql` at another root page, the page where SQLite begins the table, as when an edit of `sqlite_master` moves `sqlite_sequence` to the page of `clips`, so that both tables would keep their rows in one place: `a sqlite_sequence table that the daemon does not make`, with the table's name in its place.
- A file without a table that `schema.sql` makes: `no clips table`, with the table's name in its place.
- A `sqlite_sequence` with more than one row: `sqlite_sequence: more than one row`. The load would read the count of used ids from one of them, and another could hold a larger one.
- A row of `sqlite_sequence` that is not named `clips`: `sqlite_sequence: a row not named clips`.
- A count of used ids that is not an integer, which the daemon never writes: `sqlite_sequence: a count of used ids that is not an integer`.
- A row whose id is below 1: `clip <id>: an id below 1`. The daemon numbers its rows from 1, and a `clip:<id>` key takes no sign, so no key could name a row whose id is negative.
- A row where a column that the load reads holds a type of value that the daemon does not write there: `clip <id>: a last use that is not an integer`, with `a size`, `a pin`, or `a sequence number` in its place, or `clip <id>: a type that is not text`, with `a kind`, `a hash`, or `a preview` in its place.
- A row whose kind holds more than 5 bytes: `clip <id>: a kind over 5 bytes`.
- A row whose kind is none of `text`, `link`, `email`, `image`, and `files`: `clip <id>: an unknown kind`.
- A row whose type holds more than 65,535 bytes: `clip <id>: a type over 65535 bytes`.
- A row whose size is below 0: `clip <id>: a negative size`.
- An image that holds a text, or a row of another kind that holds none, which the table's checks refuse: `clip <id>: an image with a text`, or `clip <id>: no text`.
- A row whose hash does not hold 64 bytes: `clip <id>: a hash that is not 64 bytes`.
- A row whose hash holds a byte that is not a lowercase hex digit: `clip <id>: a hash that is not lowercase hex`.
- A row whose preview holds more than 1,024 bytes: `clip <id>: a preview over 1024 bytes`.
- An image that holds a preview, or a row of another kind that holds none, which the table's checks refuse too: `clip <id>: an image with a preview`, or `clip <id>: no preview`.
- A row whose sequence number is below 0: `clip <id>: a negative sequence number`.
- A row whose pin is neither 0 nor 1, the two that the daemon writes: `clip <id>: a pin that is not 0 or 1`.
- A row whose type or preview is text that is not UTF-8: rusqlite's error, such as `invalid utf-8 sequence of 1 bytes from index 0 at index 2`.
- A file whose ids have run out: `no clip id is left after <id>`.
- A file whose sequence numbers have run out: `no sequence number is left after <n>`.
- A load that has not ended in 5 s: `not loaded within 5 s`.

Before it reads any row, the load compares the file with one that the writer makes, in this order: its text encoding is UTF-8, its application id is lyncast's, 1819897443, which spells `lync` in ASCII, `sqlite_master` holds the objects that `schema.sql` makes, each with the same table, root page, and SQL, and nothing else, and `sqlite_sequence` holds at most one row, named `clips`, whose count of used ids is an integer. The first check that fails gives the warning. The load compares nothing else in the file's header, such as the page size, the bytes reserved at the end of each page, or the user version, which change neither the rows that the load reads nor the tables that the writer writes to. So whatever another program writes, the checks of the rows meet only the table of `schema.sql`, whose `id` is SQLite's row id, an integer that no other row holds. The load then checks every row column by column, in the order of `schema.sql`. For each column that it reads, it checks the type of value with `typeof`, then the length of a text with SQLite's `octet_length`, and then the value where the daemon limits it. The daemon writes an id of at least 1, one of the five kinds, a size not below 0, a hash whose bytes are all lowercase hex digits, a pin of 0 or 1, and a sequence number not below 0. It writes a text and a preview in a row of every kind but an image, and neither in an image, as the table's two checks say, so the load checks with `typeof` that the text, which it does not read, and the preview are there or not as the row's kind says. `typeof` and `octet_length` read only a value's header, so the load reads no value whole before it has bounded it, and a long value costs no memory. In a row, the first check that fails gives the warning. Once every check passes, it reads every row without an error, unless the row's type or preview is text that is not UTF-8. SQLite checks the type of a value as it writes the value into a `STRICT` table, and the table's checks unless they are turned off, but not as it reads the value, so a file whose schema was edited around a row, or whose bytes changed, can hold a value that the daemon does not write. A Wayland message gives its size in 16 bits, so no type that a capture reads holds 65,535 bytes, and a row that the daemon wrote always loads. SQLite opens the file, and the journal beside it, by path after the daemon's check, and an open of a FIFO waits for good, so the start waits 5 s at most. A load that the start has stopped waiting for writes no more than one that it waits for, because its thread gets no change.

The daemon gives each new row the next id itself, one above every id the database has used, so a reply can name the row before the database has it, and the id of a deleted row never comes back. Neither does the id of a new row that the file did not save: the writer raises the file's count of used ids to that id at once, and when that fails too, once more before it ends. So an id comes back only when the file can take it neither time, as on a full disk, or when the daemon exits before the writer has finished with the row, as when Quit stops waiting for the writer after its 5 s. A database whose ids leave no next one is one the daemon cannot use, and once the ids run out in memory, a new row is not recorded and the daemon prints `lyncast: no clip id is left; the clip was not recorded`. A capture or a recorded write whose kind and bytes match a row promotes that row: it moves to the front and keeps its id. A pinned row stays where it is. The `Watcher`'s first event is the current selection, so without this every daemon start would add the clipboard again (measured, 23 of 23).

The screen order is pins first, in the order they were pinned, then the rest by last use, newest first. A pin is a use. Every insert, promotion, and pin takes the next number in one sequence, so two in the same millisecond keep their order. Unpinning keeps the row's number, so the row goes back where its pin put it. The numbers end at the largest that SQLite stores, 9,223,372,036,854,775,807. Once that one is taken, a pin fails with `no sequence number is left`, and a capture is not recorded, with `lyncast: no sequence number is left; the clip was not recorded`, and a file whose rows hold it is one the daemon cannot use.

Unpinned rows expire 90 days after their last use, and pins never expire. Unpinning starts a row's age again, so a row pinned for longer than that does not expire as soon as it is unpinned. `[clipboard] retention-days` in `$XDG_CONFIG_HOME/lyncast/config.toml` sets the days, and 0 keeps rows forever. The daemon prunes once its start has succeeded, so a start that fails changes nothing in the file, and after each insert. The writer removes the rows that a prune takes in one transaction, and after it their blobs that no remaining row names, which it finds with one query. The daemon reads `config.toml` only when it is a regular file of at most 1 MiB after symlinks, so a FIFO there cannot hold up the start. A `config.toml` that cannot be read, does not parse, or gives `retention-days` a value that is not a whole number from 0 to 4,294,967,295 prints `lyncast: <path>: <error>; using the defaults` and leaves the defaults. The daemon leaves every other key alone.

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
- Back-to-back seeding races the watcher. A Tier 2 test waits for each seed's row, or for the read of a `seed-read`, before it seeds again.
- A daemon restart empties the clipboard. The daemon does not offer the newest row again at start.

## The fixtures

The tests in `crates/lyncast/tests/clipboard.rs` read the `env`, `session`, and `clipboard` blocks below, with the harness of [daemon.md](daemon.md#the-fixtures): a fresh `$TREE` and `$RUNTIME` for each block, with the `file` blocks of this doc written in `$TREE`, the `env` block below, `session` lines, `stderr:`, `exit:`, and `record:` lines, shell lines through `/bin/sh`, and the `stall-stderr`, `drain-stderr`, `wait-exit`, `freeze-thread`, and `thaw-thread` lines. `crates/lyncast-core/tests/clip.rs` reads the `retention`, `kind`, and `uris` blocks, and the unit tests in `crates/lyncast/src/clip/mod.rs` read the `store` blocks.

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

A `session` block runs with no compositor, as Tier 1 does. Its shell lines add rows to `clips.sqlite3` with `sqlite3` while no daemon runs, because a running daemon holds the file's lock. A daemon that has made no change leaves the file empty, so a block first creates the table from `schema.sql`, the schema that the writer creates before its first change, with lyncast's application id, which a unit test in `crates/lyncast/src/clip/schema.rs` checks against the writer's own. A row's columns are its id, kind, type, size, text, the SHA-256 of what it holds in hex, preview, creation time, last use, pin, and sequence number.

```file schema.sql
PRAGMA application_id = 1819897443;
CREATE TABLE IF NOT EXISTS clips (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('text', 'link', 'email', 'image', 'files')),
    mime TEXT NOT NULL,
    bytes INTEGER NOT NULL CHECK (bytes >= 0),
    body TEXT,
    hash TEXT NOT NULL,
    preview TEXT,
    created INTEGER NOT NULL,
    used INTEGER NOT NULL,
    pinned INTEGER NOT NULL CHECK (pinned IN (0, 1)),
    seq INTEGER NOT NULL CHECK (seq >= 0),
    CHECK ((kind = 'image') = (body IS NULL)),
    CHECK ((kind = 'image') = (preview IS NULL))
) STRICT;
```

A `clipboard` block runs inside its own `scripts/test-env.sh start` environment. `$RUNTIME` is the compositor's `XDG_RUNTIME_DIR`, and the block's daemons also get its `WAYLAND_DISPLAY`, so the daemon and the helpers below share one anvil. A helper is the test binary run again with only those two variables. A `clipboard` block may also use these lines:

- `$ seed <type>[,<type>...] <text>` offers the text under each type, and nothing else, until another selection replaces it. `$ seed-file <type>[,<type>...] <path>` offers a file's bytes the same way, and `$ seed-each <type> <path> [<type> <path> ...]` offers each file's bytes under the type before it. Each returns once the compositor offers its types: the helper watches the clipboard itself, and it is ready when it sees the change that its selection makes.
- `$ seed-read <type>[,<type>...] <path>` offers a file's bytes as `seed-file` does, for one paste only, and returns once that paste has read them, so in a block where nothing else reads the clipboard, the capture thread has read the offer. Its source then goes, and the selection with it.
- `$ wait-clips <n>` waits until `lyncast clip list --json` prints `n` rows, and `$ wait-first <id>` waits until the first row has that id. A seed whose row is still on its way can make the next seed's read come back empty (measured), so a block waits for each row, or for the read of a `seed-read`, before it seeds again.
- `$ stop-seed` stops the newest seeder with SIGSTOP and returns once every thread of it has stopped, so it answers no read from then on.
- `$ cont-seed-at-quit` returns at once, and once Quit has removed the daemon's socket, it continues the newest seeder with SIGCONT.
- `$ wait-read` waits until the newest daemon's capture thread waits in `poll` or `ppoll` on one descriptor, which it does only while it reads an offer. The thread is named `capture`, as the `THREAD` constant in `crates/lyncast/src/clip/capture.rs` spells it, and the wait looks at no other thread.
- `$ wait-stderr <line>` waits until `$RUNTIME/daemon.stderr` holds the line, for an entry that adds no row.
- `$ stop-compositor` stops the block's compositor, and `$RUNTIME` goes with it.
- `$ fake-compositor <name> [<interface> <version> ...]` serves a socket named `<name>` in `$RUNTIME` that offers each interface at its version as a global, answers each roundtrip, and does nothing else, until the block ends. A daemon whose `WAYLAND_DISPLAY` names it finds a compositor that offers only those globals.

### Without a compositor the history still answers

With no `WAYLAND_DISPLAY`, no capture thread runs and a copy fails, but the stored rows load and change. The first daemon creates the database empty, so that only the user may open it, in a directory that only the user may list, and it writes nothing to the file, not even a journal beside it, since it has nothing to save. The rows load as the database keeps them, with their kinds and previews: a link, an email, a file list's paths, and an image without a preview. Pins come first in the order they were pinned, and an unpinned row goes back to the place its pin gave it. A pin, an unpin, and a delete close the summon and record no visit, so no `ranking.json` appears. The changes reach the database, so the next daemon shows the same list.

```session
$ lyncast daemon --headless
$ ls data/lyncast
clips.sqlite3
$ lyncast clip list --json
$ lyncast activate clip:1 --verb pin
stderr: lyncast: no clip 1
exit: 1
$ lyncast activate cmd:quit
ran cmd:quit
$ stat -c %a data/lyncast data/lyncast/clips.sqlite3
700
600
$ stat -c %s data/lyncast/clips.sqlite3
0
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
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

Row 1 was last used 91 days ago, row 2 89 days ago, and row 3 is a pin last used in 1970. With `retention-days = 0` every row stays. A config that does not parse warns, and the default of 90 days removes row 1. A number of days past 4,294,967,295 warns too, and 4,294,967,295 itself does not. Unpinning row 3 starts its age again, so with `retention-days = 1` the next start removes row 2 and keeps row 3.

```session
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
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
$ printf '[clipboard]\nretention-days = 4294967296\n' > config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ printf '[clipboard]\nretention-days = 4294967295\n' > config/lyncast/config.toml
$ lyncast daemon --headless
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
lyncast: $TREE/config/lyncast/config.toml: invalid value: integer `4294967296`, expected u32 at line 2 column 18; using the defaults
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

The writer removes the rows that a prune takes in one transaction, and after it their blobs that no remaining row names, which it finds with one query, so a start that finds many rows expired, as after a long absence or a lower `retention-days`, does not hold the writer up. Here 5,000 images last used in 1970 go with their blobs at the first start, beside 20,000 texts that stay, before a Quit right after it replies. With a transaction for each row, Quit gave up after its 5 s with 3,930 and 3,915 of the images left in two runs, and with one transaction but a query for each blob, with 3,406 and 3,422 of the blobs left. With one transaction and one query, the daemon exited 244 to 251 ms after the Quit in three runs. The daemon drops the expired rows from memory by looking each row up in a set of their ids. With a list of the ids in place of the set, the daemon reads the whole list for each text that stays. While the start pruned before it bound the socket, the first start with the list bound it after 630 to 653 ms in six runs, against 197 to 204 ms with the set. Beside 40,000 texts it took 1,167 and 1,204 ms with the list, against 359 and 366 ms with the set. Each ran in a debug build on the dev box's NVMe disk (measured).

```session
$ mkdir -p data/lyncast/blobs && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "with recursive n (i) as (select 1 union all select i + 1 from n where i < 5000) insert into clips select i, 'image', 'image/png', 3, null, printf('%064x', i), null, 0, 0, 0, i from n"
$ sqlite3 data/lyncast/clips.sqlite3 "select writefile('data/lyncast/blobs/' || hash, 'old') from clips" > /dev/null
$ sqlite3 data/lyncast/clips.sqlite3 "with recursive n (i) as (select 5001 union all select i + 1 from n where i < 25000) insert into clips select i, 'text', 'text/plain;charset=utf-8', 3, 'new', '11507a0e2f5e69d5dfa40a62a1bd7b6ee57e6bcd85c67c9b8431b36fff21c437', 'new', 0, unixepoch() * 1000, 0, i from n"
$ ls data/lyncast/blobs | wc -l
5000
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select kind, count(*) from clips group by kind"
text|20000
$ ls data/lyncast/blobs
```

The daemon prunes only once its start has succeeded, so a start that fails changes nothing in the file. Here the socket's path is a directory, so each start fails at its bind, after the load, and the row last used in 1970 stays, with no journal beside the file. The second daemon's stderr does not drain, so it waits 1 s for its last line before it exits, which would give a prune that the load began the time to reach the file. Once the directory goes, the next start removes the row.

```session
$ mkdir -p data/lyncast && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 3, 'old', 'cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4', 'old', 0, 0, 0, 1)"
$ cp data/lyncast/clips.sqlite3 before.sqlite3 && mkdir $RUNTIME/lyncast.sock
$ lyncast daemon --headless
stderr: lyncast: $RUNTIME/lyncast.sock: Is a directory (os error 21)
exit: 1
$ date +%s%N > started
$ stall-stderr
$ lyncast daemon --headless
exit: 1
$ [ $(( $(date +%s%N) - $(cat started) )) -ge 1000000000 ] && echo 1 s or more
1 s or more
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ ls data/lyncast
clips.sqlite3
$ rmdir $RUNTIME/lyncast.sock
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select count(*) from clips"
0
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

A session cannot let days pass, and whether a seed was read before the next one replaced it does not show in a block. So the unit tests in `crates/lyncast/src/clip/mod.rs` run each `store` block on a history in memory, with the default retention of 90 days and a clock that they move. `capture <text>` records a capture of the text, `pin <text>` and `unpin <text>` change the row that holds it, `wait <n>d` and `wait <n>ms` move the clock, and `=> <text>, ...` gives the previews in screen order. A capture of a pinned row's bytes changes nothing, and unpinning the row puts it back where its pin put it:

```store
capture alpha
capture beta
pin alpha
capture gamma
capture alpha
=> alpha, gamma, beta
unpin alpha
=> gamma, alpha, beta
```

A capture prunes the rows that have expired since start:

```store
capture old
capture pinned
pin pinned
wait 90d
capture new
=> pinned, new, old
wait 1ms
capture newer
=> pinned, newer, new
```

### A database the daemon cannot use stays as it is

The four blocks below start daemons on databases that the daemon cannot use, beside a few that it can. A daemon that cannot use its database keeps its rows in memory and prints the warning that the [list above](#clips-keeps-the-rows-in-memory-and-in-sqlite) gives. It writes nothing to the file, and SQLite changes nothing beside it but what the list of side files there says.

The daemon looks the file up after symlinks, and creates it only where nothing is. Each of these leaves the rows in memory:

- A data directory that is a file, which leaves the visits in memory too.
- A data directory that is a symlink to nothing, whose target the daemon does not create.
- A file that SQLite cannot read, which keeps its bytes.
- A FIFO in the file's place, which the daemon never opens, so the start goes on.
- A symlink to nothing in the file's place, whose target the daemon does not create.

```session
$ mkdir -p data && echo not a directory > data/lyncast
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ rm data/lyncast && ln -s missing data/lyncast
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ ls data
lyncast
$ rm data/lyncast
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
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/ranking.json: Not a directory (os error 20); not moved to ranking.json.bad: Not a directory (os error 20); the visits stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: Not a directory (os error 20); the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: File exists (os error 17); the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: file is not a database; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: not a regular file; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: No such file or directory (os error 2); the clips stay in memory
```

The load takes a file that holds anything only when it matches one that `schema.sql` makes in all that the load compares. Each of these files differs from such a file, and stays byte for byte as it was:

- Another program's file, which has no application id, with a table whose count in `sqlite_sequence`, the table where SQLite counts the ids it has used, is text.
- A file from `schema.sql` whose application id is then set to 1, and a file in UTF-16 from `schema.sql`, which holds lyncast's application id and, as SQLite reads it, the same SQL.
- A file from `schema.sql` with an index beside `clips`, then with another table in the index's place, and then with neither that table nor `clips`, which leaves `sqlite_sequence`.
- A file from `schema.sql` whose `sqlite_sequence` an edit of `sqlite_master` moves to the root page of `clips`.
- A file from `schema.sql` with a second row in `sqlite_sequence` for `clips`, whose count is the largest id, and one whose only row is another table's.
- A `clips` table without `STRICT` and with a column named `Used`, a name that SQLite matches without regard to case, one whose `kind` compares without regard to case and holds `Text`, and one without `AUTOINCREMENT`, which makes no `sqlite_sequence`. Each holds lyncast's application id and a row.

A file from `schema.sql` with a page size of 65,536 bytes, 64 bytes reserved at the end of each page, and a user version of 7 loads, since the load compares nothing else in the header, and the next daemon finds the pin that the first one saved.

```session
$ mkdir -p data/lyncast && sqlite3 data/lyncast/clips.sqlite3 "create table t (id integer primary key autoincrement); insert into t default values; update sqlite_sequence set seq = 'one'"
$ cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "pragma application_id = 1" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 "pragma encoding = 'UTF-16le'" ".read schema.sql" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma encoding" "pragma application_id"
UTF-16le
1819897443
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "create index used on clips (used)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "drop index used; create table t (x)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "drop table t; drop table clips" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "select name from sqlite_master"
sqlite_sequence
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "pragma writable_schema = on; update sqlite_master set rootpage = (select rootpage from sqlite_master where name = 'clips') where name = 'sqlite_sequence'" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1); insert into sqlite_sequence values ('clips', 9223372036854775807)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "select * from sqlite_sequence"
clips|1
clips|9223372036854775807
$ sqlite3 data/lyncast/clips.sqlite3 "delete from sqlite_sequence; insert into sqlite_sequence values ('t', 1)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 "pragma application_id = 1819897443; create table clips (id integer primary key autoincrement, kind text, mime text, bytes integer, body text, hash text, preview text, created integer, Used integer, pinned integer, seq integer); insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sed 's/kind TEXT NOT NULL/kind TEXT NOT NULL COLLATE NOCASE/' schema.sql | sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'Text', 'text/plain', 3, 'abc', 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad', 'abc', 0, 0, 0, 1)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sed 's/ AUTOINCREMENT//' schema.sql | sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 "pragma page_size = 65536" ".filectrl reserve_bytes 64" ".read schema.sql" "pragma user_version = 7" "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1)"
64
$ lyncast daemon --headless
$ lyncast activate clip:1 --verb pin
pinned clip 1
$ lyncast activate cmd:quit
ran cmd:quit
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":1,"kind":"text","pinned":true,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: an application id of 0, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an application id of 1, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: a text encoding of UTF-16le, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an index that the daemon does not make; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: a table that the daemon does not make; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: no clips table; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: a sqlite_sequence table that the daemon does not make; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: sqlite_sequence: more than one row; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: sqlite_sequence: a row not named clips; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: a clips table that the daemon does not make; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: a clips table that the daemon does not make; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: a clips table that the daemon does not make; the clips stay in memory
```

Of the files beside the database, the journal and the `-wal` change only as the list of side files says, whether the daemon refuses the database or loads it:

- Beside the file with the index, in rollback mode, a file at the journal's path whose first byte is 0 stays. One that holds text goes, as SQLite takes it for the journal of a writer that crashed, and the database stays as it was.
- Another program's file in WAL mode stays as it was, and its `-wal` keeps its frames. So it goes when a symlink names that file, since SQLite keeps the `-wal` beside the file that the symlink leads to.
- Such a file with no `-wal`, as SQLite leaves a file that it closed cleanly, or with an empty one, stays as it was too, and once the daemon has refused it, no `-wal` is left beside it.
- A lyncast file in WAL mode that the daemon loads stays as it was, and the empty `-wal` that SQLite creates beside it at the load's first read is there while the daemon runs and goes once it quits.
- A writer that crashes in a transaction leaves a journal beside the file, and SQLite rolls it back at the load's first read, so the file holds its last commit and the journal goes. Here a copy of a file and its journal, taken in the middle of a transaction, stands for such a crash.
- Beside an empty file, a `-wal` and a journal go at the load's first read, and the file stays empty.
- SQLite opens the journal beside the file itself, after the daemon's check, so a FIFO there holds the load. After 5 s the daemon starts with its rows in memory, and the file keeps its row.

```session
$ mkdir -p data/lyncast && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "create index used on clips (used)" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ printf '\0stale' > data/lyncast/clips.sqlite3-journal && cp data/lyncast/clips.sqlite3-journal before.sqlite3-journal
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3 && cmp before.sqlite3-journal data/lyncast/clips.sqlite3-journal
$ echo stale > data/lyncast/clips.sqlite3-journal
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ ls data/lyncast
clips.sqlite3
$ rm data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 ".dbconfig no_ckpt_on_close on" "pragma journal_mode = wal" "create table t (v)" "insert into t values (1)"
   no_ckpt_on_close on
wal
$ cp data/lyncast/clips.sqlite3 before.sqlite3 && cp data/lyncast/clips.sqlite3-wal before.sqlite3-wal
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3 && cmp before.sqlite3-wal data/lyncast/clips.sqlite3-wal
$ ls data/lyncast
clips.sqlite3
clips.sqlite3-shm
clips.sqlite3-wal
$ mkdir elsewhere && mv data/lyncast/clips.sqlite3* elsewhere && ln -s ../../elsewhere/clips.sqlite3 data/lyncast/clips.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 elsewhere/clips.sqlite3 && cmp before.sqlite3-wal elsewhere/clips.sqlite3-wal
$ ls data/lyncast
clips.sqlite3
$ rm data/lyncast/clips.sqlite3*
$ sqlite3 data/lyncast/clips.sqlite3 "pragma journal_mode = wal" "create table t (v)" "insert into t values (1)" && cp data/lyncast/clips.sqlite3 before.sqlite3
wal
$ ls data/lyncast
clips.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ ls data/lyncast
clips.sqlite3
$ touch data/lyncast/clips.sqlite3-wal
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ ls data/lyncast
clips.sqlite3
$ rm data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma journal_mode = wal" ".read schema.sql" "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1)" && cp data/lyncast/clips.sqlite3 before.sqlite3
wal
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ ls data/lyncast
clips.sqlite3
clips.sqlite3-wal
$ stat -c %s data/lyncast/clips.sqlite3-wal
0
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ ls data/lyncast
clips.sqlite3
$ rm data/lyncast/clips.sqlite3*
$ sqlite3 t.sqlite3 "create table t (v); insert into t values (1)" && cp t.sqlite3 before.sqlite3
$ sqlite3 t.sqlite3 "pragma cache_size = 1" "begin" "insert into t select zeroblob(4096) from (select 1 union all select 2 union all select 3 union all select 4 union all select 5 union all select 6 union all select 7 union all select 8)" ".shell cp t.sqlite3 data/lyncast/clips.sqlite3 && cp t.sqlite3-journal data/lyncast/clips.sqlite3-journal" "rollback"
$ cmp -s before.sqlite3 data/lyncast/clips.sqlite3 || echo the writer left its change
the writer left its change
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ ls data/lyncast
clips.sqlite3
$ rm data/lyncast/clips.sqlite3
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 w.sqlite3 ".dbconfig no_ckpt_on_close on" "pragma journal_mode = wal" "create table t (v)"
   no_ckpt_on_close on
wal
$ cp w.sqlite3-wal data/lyncast/clips.sqlite3-wal && echo stale > data/lyncast/clips.sqlite3-journal
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ stat -c %s data/lyncast/clips.sqlite3
0
$ ls data/lyncast
clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1)" && mkfifo data/lyncast/clips.sqlite3-journal
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ rm data/lyncast/clips.sqlite3-journal
$ sqlite3 data/lyncast/clips.sqlite3 "select id, preview from clips"
1|alpha
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: an index that the daemon does not make; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an index that the daemon does not make; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an application id of 0, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an application id of 0, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an application id of 0, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an application id of 0, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: an application id of 0, which the daemon does not write; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: not loaded within 5 s; the clips stay in memory
```

The load checks each value that it reads before it reads any row. Each of these leaves the rows in memory:

- A file whose largest id leaves no next one.
- A preview of 1,025 bytes, where one of 1,024 loads, and a type of 65,536 bytes, where one of 65,535 loads.
- A hash of 63 bytes, one of 64 in capitals, and one with a NUL byte at its end.
- A kind of 6 bytes, which only a file that ignores the table's checks can hold.
- A sequence number that is the largest that SQLite stores. One below it loads, and a pin takes the last number, so the next pin fails.
- An id of -1, which the table takes but no `clip:<id>` key can name. The file stays byte for byte as it was.
- A value of a type that `STRICT` refuses, which a table holds once its schema has been edited around the row. `loose.sql` here takes `STRICT` off `clips`, and turns its checks off, while a line writes the row, and `strict.sql` puts `STRICT` back, so the file holds the SQL of `schema.sql` again. Each of these leaves the rows in memory:
  - A last use that holds 32 MiB of text.
  - A kind, type, hash, or preview that is a blob.
  - A size, pin, or sequence number that is text.
- A count of used ids that is text, which `sqlite_sequence` takes, since its columns take any type.
- With the table's checks turned off, a kind of `texts`, which names no kind, a size or a sequence number below 0, and a pin of 2. The file stays byte for byte as it was after the pin.
- With the table's checks turned off, a text row without its text, then without its preview, a link without its text, an email without its preview, and then the row as an image with a text, and as one with a preview, which the table's two checks refuse. The file stays byte for byte as it was after each.
- A type that is text that is not UTF-8.

```file loose.sql
PRAGMA ignore_check_constraints = ON;
PRAGMA writable_schema = ON;
UPDATE sqlite_master SET sql = replace(sql, ') STRICT', ')') WHERE name = 'clips';
PRAGMA writable_schema = RESET;
```

```file strict.sql
PRAGMA writable_schema = ON;
UPDATE sqlite_master SET sql = sql || ' STRICT' WHERE name = 'clips';
```

```session
$ mkdir -p data/lyncast && sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (9223372036854775807, 'text', 'text/plain;charset=utf-8', 3, 'max', '9baf3a40312f39849f46dad1040f2f039f1cffa1238c41e9db675315cfad39b6', 'max', 0, unixepoch() * 1000, 0, 1)"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "delete from clips; delete from sqlite_sequence; insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', replace(hex(zeroblob(512)), '0', 'a'), 0, unixepoch() * 1000, 0, 1)"
$ lyncast daemon --headless
$ lyncast activate clip:1 --verb pin
pinned clip 1
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set preview = preview || 'a'"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, pinned, length(preview) from clips"
1|1|1025
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set preview = 'alpha', mime = 'text/' || replace(hex(zeroblob(32765)), '0', 'a')"
$ lyncast daemon --headless
$ lyncast activate clip:1 --verb unpin
unpinned clip 1
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set mime = mime || 'a'"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set mime = 'text/plain;charset=utf-8', hash = substr(hash, 2)"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set hash = upper('8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8')"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set hash = substr('8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 1, 63) || char(0)"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set hash = '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', kind = 'images'"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, pinned, kind, length(preview), length(mime), length(hash) from clips"
1|0|images|5|24|64
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set kind = 'text', seq = 9223372036854775806"
$ lyncast daemon --headless
$ lyncast activate clip:1 --verb pin
pinned clip 1
$ lyncast activate clip:1 --verb unpin
unpinned clip 1
$ lyncast activate clip:1 --verb pin
stderr: lyncast: no sequence number is left
exit: 1
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, pinned, seq from clips"
1|0|9223372036854775807
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set seq = 1, id = -1" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set id = 1, used = replace(hex(zeroblob(16777216)), '0', 'a')" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set used = unixepoch() * 1000, mime = cast(mime as blob)" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set mime = cast(mime as text), hash = cast(hash as blob)" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set hash = cast(hash as text), kind = cast(kind as blob)" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set kind = cast(kind as text), bytes = 'five'" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set bytes = 5, preview = cast(preview as blob)" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set preview = cast(preview as text), pinned = 'no'" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set pinned = 1, seq = 'one'" ".read strict.sql"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 ".read loose.sql" "update clips set seq = 2" ".read strict.sql" "update sqlite_sequence set seq = 'one'"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, pinned, typeof(used), typeof(mime), typeof(hash), (select typeof(seq) from sqlite_sequence) from clips"
1|1|integer|text|text|text
$ sqlite3 data/lyncast/clips.sqlite3 "update sqlite_sequence set seq = 1; pragma ignore_check_constraints = 1; update clips set kind = 'texts'"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set kind = 'text', bytes = -1"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set bytes = 5, seq = -1"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set seq = 1, pinned = 2" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set pinned = 1, body = null" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set body = 'alpha', preview = null" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set kind = 'link', body = null, preview = 'alpha'" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set kind = 'email', body = 'alpha', preview = null" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set kind = 'image', mime = 'image/png'" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "pragma ignore_check_constraints = 1; update clips set body = null, preview = 'not-written-by-daemon'" && cp data/lyncast/clips.sqlite3 before.sqlite3
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ sqlite3 data/lyncast/clips.sqlite3 "update clips set kind = 'text', body = 'alpha', preview = 'alpha', mime = cast(x'ff' as text)"
$ lyncast daemon --headless
$ lyncast clip list --json
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, kind, bytes, seq, hex(mime), (select seq from sqlite_sequence) from clips"
1|text|5|1|FF|1
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: no clip id is left after 9223372036854775807; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a preview over 1024 bytes; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a type over 65535 bytes; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a hash that is not 64 bytes; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a hash that is not lowercase hex; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a hash that is not lowercase hex; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a kind over 5 bytes; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: no sequence number is left after 9223372036854775807; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip -1: an id below 1; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a last use that is not an integer; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a type that is not text; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a hash that is not text; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a kind that is not text; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a size that is not an integer; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a preview that is not text; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a pin that is not an integer; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a sequence number that is not an integer; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: sqlite_sequence: a count of used ids that is not an integer; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: an unknown kind; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a negative size; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a negative sequence number; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: a pin that is not 0 or 1; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: no text; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: no preview; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: no text; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: no preview; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: an image with a text; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: clip 1: an image with a preview; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: invalid utf-8 sequence of 1 bytes from index 0 at index 2; the clips stay in memory
```

A load that the start has stopped waiting for changes no more than a load that the start waits for, which is at most what SQLite rolls back or removes for it. Here a FIFO at the journal's path holds the load's open of a database that holds no table past the 5 s. Once the daemon has started with its rows in memory, the test holds the load's thread still, removes the FIFO, and lets the thread go on, so its open starts again and finds no journal, and the load ends. The test waits until the daemon holds no descriptor of the file, looking 1,000 times at most, and `ls` may find a descriptor gone that it listed, so the wait drops its errors. Once the load has closed the file, the file is as it was. So it goes for a file in WAL mode that gets a `-wal` with frames while the thread is held, before the load holds the lock. The test copies that `-wal` from a connection to a copy of the file. The load looks at the `-wal` only once it holds the lock, so the `-wal` keeps its frames, and the file its bytes.

```session
$ mkdir -p data/lyncast && sqlite3 data/lyncast/clips.sqlite3 "create table t (x); drop table t"
$ cp data/lyncast/clips.sqlite3 before.sqlite3 && mkfifo data/lyncast/clips.sqlite3-journal
$ lyncast daemon --headless
$ freeze-thread clips
$ rm data/lyncast/clips.sqlite3-journal
$ thaw-thread
$ i=0; while ls -l /proc/$DAEMON/fd 2>/dev/null | grep -q clips.sqlite3; do [ $((i += 1)) -lt 1000 ] || exit 1; sleep 0.01; done
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3
$ ls data/lyncast
clips.sqlite3
$ rm data/lyncast/clips.sqlite3 && sqlite3 data/lyncast/clips.sqlite3 "pragma journal_mode = wal" "create table t (x)" "drop table t"
wal
$ cp data/lyncast/clips.sqlite3 before.sqlite3 && cp data/lyncast/clips.sqlite3 w.sqlite3
$ sqlite3 w.sqlite3 ".dbconfig no_ckpt_on_close on" "create table t (x)" "drop table t"
   no_ckpt_on_close on
$ mkfifo data/lyncast/clips.sqlite3-journal
$ lyncast daemon --headless
$ freeze-thread clips
$ cp w.sqlite3-wal data/lyncast/clips.sqlite3-wal && rm data/lyncast/clips.sqlite3-journal
$ thaw-thread
$ i=0; while ls -l /proc/$DAEMON/fd 2>/dev/null | grep -q clips.sqlite3; do [ $((i += 1)) -lt 1000 ] || exit 1; sleep 0.01; done
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp before.sqlite3 data/lyncast/clips.sqlite3 && cmp w.sqlite3-wal data/lyncast/clips.sqlite3-wal
$ ls data/lyncast
clips.sqlite3
clips.sqlite3-wal
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: not loaded within 5 s; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: not loaded within 5 s; the clips stay in memory
```

With a compositor, a daemon whose database stays as it is keeps what it captures in memory.

```clipboard
$ mkdir -p data/lyncast && echo not a database > data/lyncast/clips.sqlite3
$ lyncast daemon --headless
$ seed text/plain;charset=utf-8 alpha
$ wait-clips 1
$ lyncast activate cmd:quit
ran cmd:quit
$ cat data/lyncast/clips.sqlite3
not a database
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: file is not a database; the clips stay in memory
```

### A second daemon finds the history locked

The first daemon holds SQLite's exclusive lock from its load on, so `sqlite3` cannot read the database while it runs, and a second daemon with its own `XDG_RUNTIME_DIR` and the same data home keeps its clips in memory. Once the first daemon quits, the next one loads the database. So it goes for a file that holds a row: the daemon that loads it lists the row, `sqlite3` cannot read the file, and a second daemon lists nothing.

```session
$ lyncast daemon --headless
$ sqlite3 data/lyncast/clips.sqlite3 "select count(*) from sqlite_master"
stderr: Error: in prepare, database is locked (5)
exit: 5
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
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', '8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8', 'alpha', 0, unixepoch() * 1000, 0, 1)"
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ sqlite3 data/lyncast/clips.sqlite3 "select count(*) from clips"
stderr: Error: in prepare, database is locked (5)
exit: 5
$ XDG_RUNTIME_DIR=$RUNTIME/two lyncast daemon --headless
$ XDG_RUNTIME_DIR=$RUNTIME/two lyncast clip list --json
$ XDG_RUNTIME_DIR=$RUNTIME/two lyncast activate cmd:quit
ran cmd:quit
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: database is locked; the clips stay in memory
lyncast: $TREE/data/lyncast/clips.sqlite3: database is locked; the clips stay in memory
```

### The filters keep own writes, passwords, and large entries out

A seed that carries the marker or the password hint adds no row, and neither does one over 25 MiB, empty text, text that is not UTF-8, or an empty image. An image of exactly 25 MiB is kept. A plain text follows each seed that adds no row, and the `Watcher` hands the offers over in order, so once the text's row is there, the capture thread has taken the seed's offer. The thread never reads an offer with the marker or the hint, so nothing replaces such an offer too soon. Each of the others is a `seed-read`, which returns once the thread has read it.

```clipboard
$ lyncast daemon --headless
$ seed text/plain;charset=utf-8,application/x-lyncast-own;copy=1 mine
$ seed text/plain;charset=utf-8 one
$ wait-clips 1
$ seed text/plain;charset=utf-8,x-kde-passwordManagerHint hunter2
$ seed text/plain;charset=utf-8 two
$ wait-clips 2
$ head -c 26214401 /dev/zero | tr '\0' a > big.txt
$ seed-read text/plain;charset=utf-8 big.txt
$ seed text/plain;charset=utf-8 three
$ wait-clips 3
$ : > empty.txt
$ seed-read text/plain;charset=utf-8 empty.txt
$ seed text/plain;charset=utf-8 four
$ wait-clips 4
$ seed-read image/png empty.txt
$ seed text/plain;charset=utf-8 five
$ wait-clips 5
$ printf '\377' > bad.txt
$ seed-read text/plain;charset=utf-8 bad.txt
$ seed text/plain;charset=utf-8 six
$ wait-clips 6
$ head -c 26214400 /dev/zero > big.png
$ seed-file image/png big.png
$ wait-clips 7
$ lyncast clip list --json
{"id":7,"kind":"image","pinned":false,"mime":"image/png","bytes":26214400,"preview":null}
{"id":6,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":3,"preview":"six"}
{"id":5,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":4,"preview":"five"}
{"id":4,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":4,"preview":"four"}
{"id":3,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"three"}
{"id":2,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":3,"preview":"two"}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":3,"preview":"one"}
```

A source that never answers holds a read for 2 s at most. Here the seeder stops before the daemon starts, so the daemon's first event is its offer. The next seed's row comes once that read gives up.

```clipboard
$ seed text/plain;charset=utf-8 stuck
$ stop-seed
$ lyncast daemon --headless
$ seed text/plain;charset=utf-8 kept
$ wait-clips 1
$ lyncast clip list --json
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":4,"preview":"kept"}
```

### An offer gives files, else an image, else text

Each seed here offers more than one representation, and the row shows which one the thread read. An image comes before text, and files before an image. `image/png` comes before another image type, which the thread takes when no PNG is offered, and `text/uri-list` comes before the GNOME list. A list's lines may end in CRLF, and a file entry keeps each local file that its list names, in order, without the comment and the other host. The last seed's list is not UTF-8, so the thread reads its text. A list whose one path is longer than the preview shows its first 1,024 bytes, cut at a character boundary: here 1,021, because the character of 4 bytes after them would end past 1,024.

```clipboard
$ lyncast daemon --headless
$ seed image/png,text/plain;charset=utf-8 alpha
$ wait-clips 1
$ seed image/webp,image/png beta
$ wait-clips 2
$ seed image/webp gamma
$ wait-clips 3
$ seed text/uri-list,x-special/gnome-copied-files file:///etc/hostname
$ wait-clips 4
$ seed text/uri-list,image/png file:///etc/hosts
$ wait-clips 5
$ printf 'file:///etc/hostname\r\n# a comment\r\nfile://example.org/x\r\nfile:///tmp/a%%20b\r\n' > list.txt
$ seed-file text/uri-list,text/plain;charset=utf-8 list.txt
$ wait-clips 6
$ printf 'file:///tmp/caf\351' > latin1.txt && printf hello > hello.txt
$ seed-each text/uri-list latin1.txt text/plain;charset=utf-8 hello.txt
$ wait-clips 7
$ lyncast clip list --json
{"id":7,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"hello"}
{"id":6,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":38,"preview":"/etc/hostname\n/tmp/a b"}
{"id":5,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":17,"preview":"/etc/hosts"}
{"id":4,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":20,"preview":"/etc/hostname"}
{"id":3,"kind":"image","pinned":false,"mime":"image/webp","bytes":5,"preview":null}
{"id":2,"kind":"image","pinned":false,"mime":"image/png","bytes":4,"preview":null}
{"id":1,"kind":"image","pinned":false,"mime":"image/png","bytes":5,"preview":null}
$ { printf file:///; head -c 1020 /dev/zero | tr '\0' a; printf %%F0%%9F%%98%%80%%F0%%9F%%98%%80; } > long.txt
$ seed-file text/uri-list long.txt
$ wait-clips 8
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select kind, bytes, length(cast(preview as blob)), substr(preview, -2) from clips where id = 8"
files|1052|1021|aa
```

### The same bytes promote a row, and pins stay first

A seed of text that a row holds moves that row to the front and keeps its id. A pin stays first, and unpinning it puts it back where its pin put it. A file list that names a local file is a file entry, and one that names only a link is read as text. The rows persist, in a file in UTF-8 with lyncast's application id, which the writer set as it created the table, so the next daemon loads them. The file keeps when each row was captured, and a promotion, a pin, and an unpin leave that as it was. So only the rows that one of them used again show a last use after it, and the first two rows show a capture before the promotion of the first, though the second was pinned and unpinned after it. The next daemon's first event, the selection already there, promotes the row that holds it, and the daemon after that keeps the promotion and gives a new row the next id.

```clipboard
$ lyncast daemon --headless
$ seed text/plain;charset=utf-8 alpha
$ wait-clips 1
$ seed text/plain;charset=utf-8 beta
$ wait-clips 2
$ seed text/plain;charset=utf-8 alpha
$ wait-first 1
$ lyncast activate clip:2 --verb pin
pinned clip 2
$ seed text/uri-list,text/plain;charset=utf-8 https://example.org/a
$ wait-clips 3
$ printf 'copy\nfile:///etc/hostname' > gnome.txt
$ seed-file x-special/gnome-copied-files gnome.txt
$ wait-clips 4
$ seed text/plain;charset=utf-8 me@example.org
$ wait-clips 5
$ lyncast clip list --json
{"id":2,"kind":"text","pinned":true,"mime":"text/plain;charset=utf-8","bytes":4,"preview":"beta"}
{"id":5,"kind":"email","pinned":false,"mime":"text/plain;charset=utf-8","bytes":14,"preview":"me@example.org"}
{"id":4,"kind":"files","pinned":false,"mime":"x-special/gnome-copied-files","bytes":20,"preview":"/etc/hostname"}
{"id":3,"kind":"link","pinned":false,"mime":"text/plain;charset=utf-8","bytes":21,"preview":"https://example.org/a"}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ lyncast activate clip:2 --verb unpin
unpinned clip 2
$ lyncast activate clip:3 --verb delete
deleted clip 3
$ lyncast activate clip:3
stderr: lyncast: no clip 3
exit: 1
$ lyncast clip list --json
{"id":5,"kind":"email","pinned":false,"mime":"text/plain;charset=utf-8","bytes":14,"preview":"me@example.org"}
{"id":4,"kind":"files","pinned":false,"mime":"x-special/gnome-copied-files","bytes":20,"preview":"/etc/hostname"}
{"id":2,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":4,"preview":"beta"}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "pragma encoding" "pragma application_id"
UTF-8
1819897443
$ sqlite3 data/lyncast/clips.sqlite3 "select id from clips where created < used"
1
2
$ sqlite3 data/lyncast/clips.sqlite3 "select id from clips where created < (select used from clips where id = 1)"
1
2
$ seed text/plain;charset=utf-8 beta
$ lyncast daemon --headless
$ wait-first 2
$ lyncast activate cmd:quit
ran cmd:quit
$ seed text/plain;charset=utf-8 gamma
$ lyncast daemon --headless
$ wait-clips 5
$ lyncast clip list --json
{"id":6,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"gamma"}
{"id":2,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":4,"preview":"beta"}
{"id":5,"kind":"email","pinned":false,"mime":"text/plain;charset=utf-8","bytes":14,"preview":"me@example.org"}
{"id":4,"kind":"files","pinned":false,"mime":"x-special/gnome-copied-files","bytes":20,"preview":"/etc/hostname"}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
```

A capture of a pinned row's bytes changes nothing, which the first `store` block shows, because whether a seed was read before the next one replaced it does not show here.

### An image keeps its blob until its row goes

The writer writes an image's blob before its row, and the blob goes with its last row. Only the user may open a blob or list `blobs/`. The writer writes after the row appears, so the block reads the blob once Quit has waited for the writer. The id of a deleted row never comes back, even the newest one across a restart, whose first event here adds the selection again as a new row.

```clipboard
$ printf '\211PNG\015\012\032\012\000\000\000\015IHDR\000\000\000\001\000\000\000\001\010\002\000\000\000\220wS\336\000\000\000\014IDATx\332c\370\317\300\000\000\003\001\001\000\367\003AC\000\000\000\000IEND\256B`\202' > one.png
$ lyncast daemon --headless
$ seed-file image/png one.png
$ wait-clips 1
$ lyncast clip list --json
{"id":1,"kind":"image","pinned":false,"mime":"image/png","bytes":69,"preview":null}
$ seed text/plain;charset=utf-8 alpha
$ wait-clips 2
$ lyncast activate cmd:quit
ran cmd:quit
$ cmp data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20 one.png
$ stat -c %a data/lyncast/blobs data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20
700
600
$ lyncast daemon --headless
$ lyncast activate clip:1 --verb delete
deleted clip 1
$ lyncast activate clip:2 --verb delete
deleted clip 2
$ lyncast activate cmd:quit
ran cmd:quit
$ ls data/lyncast/blobs
$ lyncast daemon --headless
$ wait-clips 1
$ lyncast clip list --json
{"id":3,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
```

Two rows here name one blob, which stays until the second goes. The delete of a row comes first and its blob's after it, so while the journal's path is a directory and SQLite cannot delete the row, the blob stays too, and the next daemon lists the row and deletes it. A delete whose rows are gone but whose commit fails keeps their blobs as well. No block can make the commit fail once the rows are gone, so `a_blob_stays_until_the_delete_of_its_row_is_saved` in `crates/lyncast/src/clip/db.rs` fails one with a foreign key that SQLite checks only at the commit.

```session
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ mkdir data/lyncast/blobs && printf '\211PNG\015\012\032\012\000\000\000\015IHDR\000\000\000\001\000\000\000\001\010\002\000\000\000\220wS\336\000\000\000\014IDATx\332c\370\317\300\000\000\003\001\001\000\367\003AC\000\000\000\000IEND\256B`\202' > data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'image', 'image/png', 69, null, '2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20', null, 0, unixepoch() * 1000, 0, 1)"
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (2, 'image', 'image/png', 69, null, '2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20', null, 0, unixepoch() * 1000, 0, 2)"
$ lyncast daemon --headless
$ lyncast activate clip:1 --verb delete
deleted clip 1
$ lyncast activate cmd:quit
ran cmd:quit
$ ls data/lyncast/blobs
2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20
$ lyncast daemon --headless
$ mkdir data/lyncast/clips.sqlite3-journal
$ lyncast activate clip:2 --verb delete
deleted clip 2
$ lyncast activate cmd:quit
ran cmd:quit
$ rmdir data/lyncast/clips.sqlite3-journal
$ ls data/lyncast/blobs
2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":2,"kind":"image","pinned":false,"mime":"image/png","bytes":69,"preview":null}
$ lyncast activate clip:2 --verb delete
deleted clip 2
$ lyncast activate cmd:quit
ran cmd:quit
$ ls data/lyncast/blobs
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: unable to open database file
```

### A capture replaces the row of a delete that the file did not save

A delete that the file does not save leaves the row in the file, though the history no longer holds it, so a capture of the same bytes adds a new row. The writer removes the old row in the transaction that saves the new one, and only a row of the new row's kind and bytes, so the next daemon lists those bytes once and still lists every other row. Here SQLite saves neither the delete of a text nor that of an image while the journal's path is a directory, and the same text and image captured once the path is free leave one row each. A file list that holds the text's bytes stays beside them, and so does a text of other bytes. The image's two rows named one blob, which stays for the new row. A daemon that has written keeps its journal beside the file until it ends (measured), so the block makes the journal's path a directory in a daemon that has not written yet, and seeds with `seed-read`, whose selection goes once it is read, so that no daemon's first event captures it again. No block can make the insert fail once the old row is gone, so `an_insert_that_is_not_saved_keeps_the_row_it_replaces` in `crates/lyncast/src/clip/db.rs` fails one on an id that another row holds and finds the old row still there, which the same insert on a free id then replaces.

```clipboard
$ printf '\211PNG\015\012\032\012\000\000\000\015IHDR\000\000\000\001\000\000\000\001\010\002\000\000\000\220wS\336\000\000\000\014IDATx\332c\370\317\300\000\000\003\001\001\000\367\003AC\000\000\000\000IEND\256B`\202' > one.png
$ printf file:///etc/hostname > uri.txt
$ printf alpha > alpha.txt
$ lyncast daemon --headless
$ seed-read text/plain;charset=utf-8 uri.txt
$ wait-clips 1
$ seed-read text/uri-list uri.txt
$ wait-clips 2
$ seed-read text/plain;charset=utf-8 alpha.txt
$ wait-clips 3
$ seed-read image/png one.png
$ wait-clips 4
$ lyncast activate cmd:quit
ran cmd:quit
$ lyncast daemon --headless
$ mkdir data/lyncast/clips.sqlite3-journal
$ lyncast activate clip:1 --verb delete
deleted clip 1
$ lyncast activate clip:4 --verb delete
deleted clip 4
$ until [ $(grep -c 'not saved: unable to open database file' $RUNTIME/daemon.stderr) = 2 ]; do sleep 0.1; done
$ rmdir data/lyncast/clips.sqlite3-journal
$ seed-read text/plain;charset=utf-8 uri.txt
$ wait-clips 3
$ seed-read image/png one.png
$ wait-clips 4
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, kind, preview from clips"
2|files|/etc/hostname
3|text|alpha
5|text|file:///etc/hostname
6|image|
$ cmp data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20 one.png
$ lyncast daemon --headless
$ lyncast clip list --json
{"id":6,"kind":"image","pinned":false,"mime":"image/png","bytes":69,"preview":null}
{"id":5,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":20,"preview":"file:///etc/hostname"}
{"id":3,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
{"id":2,"kind":"files","pinned":false,"mime":"text/uri-list","bytes":20,"preview":"/etc/hostname"}
$ lyncast activate cmd:quit
ran cmd:quit
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: unable to open database file
lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: unable to open database file
```

### A stalled disk drops new rows past 64 MiB

Here the test holds the writer still, as a disk that stops answering would, so the image's row and the rows after it wait with their bytes. Two texts of 25 MiB wait beside the image, and a third would bring what waits past 64 MiB, so it is not recorded. A small text still is. The image captured again while its row waits only promotes the row. A directory then stands where the blob's temporary file goes, so once the writer goes on, the blob fails to write, and the image's row leaves the history with both captures, while the rows that waited reach the database. Quit waits 5 s at most for the writer, and a writer still at work after that keeps the database until the daemon exits, so the test waits for the exit before it reads the database.

```clipboard
$ printf '\211PNG\015\012\032\012\000\000\000\015IHDR\000\000\000\001\000\000\000\001\010\002\000\000\000\220wS\336\000\000\000\014IDATx\332c\370\317\300\000\000\003\001\001\000\367\003AC\000\000\000\000IEND\256B`\202' > one.png
$ head -c 26214400 /dev/zero | tr '\0' a > a.txt
$ head -c 26214400 /dev/zero | tr '\0' b > b.txt
$ head -c 26214400 /dev/zero | tr '\0' c > c.txt
$ lyncast daemon --headless
$ freeze-thread clips
$ seed-file image/png one.png
$ wait-clips 1
$ seed-file text/plain;charset=utf-8 a.txt
$ wait-clips 2
$ seed-file text/plain;charset=utf-8 b.txt
$ wait-clips 3
$ seed-file text/plain;charset=utf-8 c.txt
$ wait-stderr lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: 64 MiB of clips wait for the disk; the clip was not recorded
$ seed text/plain;charset=utf-8 kept
$ wait-clips 4
$ seed-file image/png one.png
$ wait-first 1
$ mkdir -p data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20.$DAEMON.tmp
$ thaw-thread
$ wait-clips 3
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
stderr: lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: 64 MiB of clips wait for the disk; the clip was not recorded
stderr: lyncast: $TREE/data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20: not saved: Is a directory (os error 21); the clip was not recorded
$ sqlite3 data/lyncast/clips.sqlite3 "select id, bytes from clips"
2|26214400
3|26214400
4|4
```

### A held owner loop drops captures past 64 MiB

Here the test holds the daemon's main thread still, which runs the owner loop, so each entry that the capture thread hands over waits with its bytes, as it would behind a busy owner loop. Each `seed-read` returns once the capture thread has read the offer. The capture thread takes each entry's share of the 64 MiB before it hands the entry over, so two texts of 25 MiB wait, and a third would bring what waits past 64 MiB, so it is not recorded. A small text and an image still are. Once the owner loop goes on, the rows that waited go to the writer, which gives back each row's share as it applies the row. A directory stands where the image's blob goes, so the image's row fails, and it fails last, so once its warning is out, every row before it has given its share back, and the third text is recorded.

```clipboard
$ printf '\211PNG\015\012\032\012\000\000\000\015IHDR\000\000\000\001\000\000\000\001\010\002\000\000\000\220wS\336\000\000\000\014IDATx\332c\370\317\300\000\000\003\001\001\000\367\003AC\000\000\000\000IEND\256B`\202' > one.png
$ head -c 26214400 /dev/zero | tr '\0' a > a.txt
$ head -c 26214400 /dev/zero | tr '\0' b > b.txt
$ head -c 26214400 /dev/zero | tr '\0' c > c.txt
$ printf kept > kept.txt
$ lyncast daemon --headless
$ mkdir -p data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20.$DAEMON.tmp
$ freeze-thread main
$ seed-read text/plain;charset=utf-8 a.txt
$ seed-read text/plain;charset=utf-8 b.txt
$ seed-read text/plain;charset=utf-8 c.txt
$ wait-stderr lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: 64 MiB of clips wait for the disk; the clip was not recorded
$ seed-read text/plain;charset=utf-8 kept.txt
$ seed-read image/png one.png
$ thaw-thread
$ wait-stderr lyncast: $TREE/data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20: not saved: Is a directory (os error 21); the clip was not recorded
$ wait-clips 3
$ seed-read text/plain;charset=utf-8 c.txt
$ wait-clips 4
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, bytes from clips"
1|26214400
2|26214400
3|4
5|26214400
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: 64 MiB of clips wait for the disk; the clip was not recorded
lyncast: $TREE/data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20: not saved: Is a directory (os error 21); the clip was not recorded
```

### A stalled disk refuses changes past 100

Here the test holds the writer still, as above, so each change waits for it. Row 1 was last used a day before a time 20 s after the shell line stores it, and `retention-days` is 1, so it expires then, once the daemon has started. The shell line pins and unpins the image's row 49 times, so 98 changes wait beside the image's row, which makes 99, and the line after it waits until row 1 has expired. A capture then would add a row and remove row 1, two changes, so it is not recorded, and row 1 stays. A pin still fits, which makes 100, so the unpin after it fails and changes nothing. A capture of row 1's text then would promote the row, so it is not recorded either, and the shell line waits until its warning has reached stderr. Once a directory stands where the blob's temporary file goes and the writer goes on, the image's row goes and the changes that waited drain, so a capture is recorded and saved again, and row 1 goes with it. The test holds the writer before any change reaches it, so all 100 changes here wait in its queue and none in its hand. No block can time a hold while the writer has a change in hand, so `a_change_in_hand_keeps_its_place` in `crates/lyncast/src/clip/db.rs` holds it as it reports a new row that it could not save, and finds that 99 more changes fit beside that one, and 100 only once the writer has finished with it.

```clipboard
$ printf '\211PNG\015\012\032\012\000\000\000\015IHDR\000\000\000\001\000\000\000\001\010\002\000\000\000\220wS\336\000\000\000\014IDATx\332c\370\317\300\000\000\003\001\001\000\367\003AC\000\000\000\000IEND\256B`\202' > one.png
$ mkdir -p config/lyncast && printf '[clipboard]\nretention-days = 1\n' > config/lyncast/config.toml
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ echo $(( $(date +%s) + 20 )) > expires
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 3, 'old', 'cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4', 'old', 0, ($(cat expires) - 86400) * 1000, 0, 1)"
$ lyncast daemon --headless
$ freeze-thread clips
$ seed-file image/png one.png
$ wait-clips 2
$ export XDG_RUNTIME_DIR=$RUNTIME; for i in $(seq 49); do $LYNCAST activate clip:2 --verb pin && $LYNCAST activate clip:2 --verb unpin || exit 1; done > /dev/null
$ until [ $(date +%s) -gt $(cat expires) ]; do sleep 0.1; done
$ seed text/plain;charset=utf-8 alpha
$ wait-stderr lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: 100 changes wait for the disk; the clip was not recorded
$ lyncast activate clip:2 --verb pin
pinned clip 2
$ lyncast activate clip:2 --verb unpin
stderr: lyncast: not saved: 100 changes wait for the disk
exit: 1
$ seed text/plain;charset=utf-8 old
$ until [ $(grep -c '100 changes wait for the disk; the clip was not recorded' $RUNTIME/daemon.stderr) = 2 ]; do sleep 0.1; done
$ lyncast clip list --json
{"id":2,"kind":"image","pinned":true,"mime":"image/png","bytes":69,"preview":null}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":3,"preview":"old"}
$ mkdir -p data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20.$DAEMON.tmp
$ thaw-thread
$ wait-clips 1
$ seed text/plain;charset=utf-8 beta
$ wait-first 3
$ lyncast clip list --json
{"id":3,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":4,"preview":"beta"}
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, preview from clips"
3|beta
```

### A new row that the file does not save leaves the history

While the journal's path is a directory, SQLite cannot save a row, so the new row goes from the history, and its id cannot reach the file's count of used ids either. An image's row goes the same way, after its blob was written, and the writer removes the blob, which no saved row names, so `blobs/` is left empty. Both warnings read the same, so a shell line counts them. Once the directory has gone, the writer puts the newer id in the count as it ends at Quit, so the next daemon gives the text, captured again as its first event, the next id. An image whose blob does not reach the disk goes the same way, and the writer puts its id in the count at once: here a directory stands where its blob's temporary file goes. The test then holds the writer still while a second image's row waits for it, so Quit stops waiting for the writer after its 5 s, and the first image's id is in the count all the same. The second image's row never reached the file, so the next daemon, whose first event captures that image again, gives it the same id, 5: an id comes back only for a row that the writer has not finished with when the daemon exits.

```clipboard
$ printf '\211PNG\015\012\032\012\000\000\000\015IHDR\000\000\000\001\000\000\000\001\010\002\000\000\000\220wS\336\000\000\000\014IDATx\332c\370\317\300\000\000\003\001\001\000\367\003AC\000\000\000\000IEND\256B`\202' > one.png
$ printf two > two.png
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ lyncast daemon --headless
$ mkdir data/lyncast/clips.sqlite3-journal
$ seed-file image/png one.png
$ wait-stderr lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: unable to open database file; the clip was not recorded
$ wait-clips 0
$ ls data/lyncast/blobs
$ seed text/plain;charset=utf-8 alpha
$ until [ $(grep -c 'clips.sqlite3: not saved' $RUNTIME/daemon.stderr) = 2 ]; do sleep 0.1; done
$ wait-clips 0
$ rmdir data/lyncast/clips.sqlite3-journal
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select seq from sqlite_sequence"
2
$ lyncast daemon --headless
$ wait-clips 1
$ lyncast clip list --json
{"id":3,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ mkdir -p data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20.$DAEMON.tmp
$ seed-file image/png one.png
$ wait-stderr lyncast: $TREE/data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20: not saved: Is a directory (os error 21); the clip was not recorded
$ wait-clips 1
$ freeze-thread clips
$ seed-file image/png two.png
$ wait-clips 2
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
stderr: lyncast: $TREE/data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20: not saved: Is a directory (os error 21); the clip was not recorded
$ sqlite3 data/lyncast/clips.sqlite3 "select id, preview from clips"
3|alpha
$ sqlite3 data/lyncast/clips.sqlite3 "select seq from sqlite_sequence"
4
$ lyncast daemon --headless
$ wait-clips 2
$ lyncast clip list --json
{"id":5,"kind":"image","pinned":false,"mime":"image/png","bytes":3,"preview":null}
{"id":3,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
$ cat $RUNTIME/daemon.stderr
lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: unable to open database file; the clip was not recorded
lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: unable to open database file; the clip was not recorded
lyncast: $TREE/data/lyncast/blobs/2e9b06dc65a4dec84a3eb3124553ec93ca27c78221e64ab2177d0f1412cfcb20: not saved: Is a directory (os error 21); the clip was not recorded
```

A file with no table gets it with the first change, so while the journal's path is a directory, the first capture is not recorded, and its id cannot reach the count. As the writer ends, it creates the table and puts the id in the count.

```clipboard
$ lyncast daemon --headless
$ mkdir data/lyncast/clips.sqlite3-journal
$ seed text/plain;charset=utf-8 alpha
$ wait-stderr lyncast: $TREE/data/lyncast/clips.sqlite3: not saved: unable to open database file; the clip was not recorded
$ wait-clips 0
$ rmdir data/lyncast/clips.sqlite3-journal
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select seq from sqlite_sequence"
1
```

### The ids run out

A row with the second largest id leaves one id, which the first capture takes. The next capture then adds no row.

```clipboard
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (9223372036854775806, 'text', 'text/plain;charset=utf-8', 3, 'old', 'cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4', 'old', 0, unixepoch() * 1000, 0, 1)"
$ seed text/plain;charset=utf-8 alpha
$ lyncast daemon --headless
$ wait-clips 2
$ seed text/plain;charset=utf-8 beta
$ wait-stderr lyncast: no clip id is left; the clip was not recorded
$ lyncast clip list --json
{"id":9223372036854775807,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
{"id":9223372036854775806,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":3,"preview":"old"}
```

### The sequence runs out

A row whose sequence number is one below the largest that SQLite stores leaves one number, which the first capture takes. The next capture then adds no row, and a capture of row 1's text does not promote it. The shell line waits until both warnings have reached stderr.

```clipboard
$ lyncast daemon --headless
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 < schema.sql
$ sqlite3 data/lyncast/clips.sqlite3 "insert into clips values (1, 'text', 'text/plain;charset=utf-8', 3, 'old', 'cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4', 'old', 0, unixepoch() * 1000, 0, 9223372036854775806)"
$ seed text/plain;charset=utf-8 alpha
$ lyncast daemon --headless
$ wait-clips 2
$ seed text/plain;charset=utf-8 beta
$ wait-stderr lyncast: no sequence number is left; the clip was not recorded
$ seed text/plain;charset=utf-8 old
$ until [ $(grep -cx 'lyncast: no sequence number is left; the clip was not recorded' $RUNTIME/daemon.stderr) = 2 ]; do sleep 0.1; done
$ lyncast clip list --json
{"id":2,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":5,"preview":"alpha"}
{"id":1,"kind":"text","pinned":false,"mime":"text/plain;charset=utf-8","bytes":3,"preview":"old"}
$ lyncast activate cmd:quit
ran cmd:quit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, seq from clips"
1|9223372036854775806
2|9223372036854775807
```

### An offer read when Quit begins is recorded

Here the daemon's first event is the offer of a stopped seeder, and the block waits until the capture thread reads it. The seeder goes on only once Quit has cancelled the `Watcher`, so the thread hands its entry over after Quit has begun. The daemon closes the writer of the clips only once the thread has ended, so the row reaches the database.

```clipboard
$ seed text/plain;charset=utf-8 late
$ stop-seed
$ lyncast daemon --headless
$ wait-read
$ cont-seed-at-quit
$ lyncast activate cmd:quit
ran cmd:quit
$ wait-exit
$ sqlite3 data/lyncast/clips.sqlite3 "select id, body from clips"
1|late
```

### The daemon needs its compositor

A daemon with `WAYLAND_SOCKET` set, even empty or not UTF-8, does not start, whether it has the block's `WAYLAND_DISPLAY` or none. A daemon whose `WAYLAND_DISPLAY` names no compositor does not start either, and neither does one whose compositor offers wlr data-control but no seat, or a seat but no data-control. When the compositor goes, the daemon stops as Quit does and exits 1. A block cannot time a Quit, so `quit_reads_no_offer_queued_behind_the_one_in_hand` in `crates/lyncast/tests/clipboard.rs` expects a daemon with a compositor to exit within 2.5 s of a Quit, half of Quit's 5 s, while the read of a stopped seeder's offer holds the capture thread and the offers of two more wait behind it. The thread reads neither of those. The test holds the capture thread still while it seeds each offer and stops the seeder, so the thread asks for no offer before its seeder has stopped, and it expects no row before the Quit. Here the test holds the writer of the visits still, as in [daemon.md's](daemon.md#a-slow-write-holds-up-nothing-but-quit) slow write, and lets it go on a second after the compositor has gone, and `ranking.json` holds Lock Screen's visit once the daemon has exited, so the daemon waited for the write.

```clipboard
$ WAYLAND_SOCKET=3 lyncast daemon --headless
stderr: lyncast: not started: WAYLAND_SOCKET is set; lyncast connects through WAYLAND_DISPLAY
exit: 1
$ env -u WAYLAND_DISPLAY WAYLAND_SOCKET= lyncast daemon --headless
stderr: lyncast: not started: WAYLAND_SOCKET is set; lyncast connects through WAYLAND_DISPLAY
exit: 1
$ WAYLAND_SOCKET=\x{FF} lyncast daemon --headless
stderr: lyncast: not started: WAYLAND_SOCKET is set; lyncast connects through WAYLAND_DISPLAY
exit: 1
$ WAYLAND_DISPLAY=nowhere lyncast daemon --headless
stderr: lyncast: not started: Couldn't connect to the Wayland compositor
exit: 1
$ fake-compositor seatless zwlr_data_control_manager_v1 2
$ WAYLAND_DISPLAY=seatless lyncast daemon --headless
stderr: lyncast: not started: There are no seats
exit: 1
$ fake-compositor bare wl_seat 9
$ WAYLAND_DISPLAY=bare lyncast daemon --headless
stderr: lyncast: not started: A required Wayland protocol (ext-data-control, or wlr-data-control version 1) is not supported by the compositor
exit: 1
$ lyncast daemon --headless
$ seed text/plain;charset=utf-8 alpha
$ wait-clips 1
$ freeze-thread visits
$ lyncast activate cmd:lock-screen
ran cmd:lock-screen
record: ["loginctl","lock-session"]
$ thaw-thread 1s
$ stop-compositor
$ wait-exit
stderr: lyncast: the Wayland connection closed
exit: 1
$ sed -E 's/[0-9]+/N/g' data/lyncast/ranking.json
{"cmd:lock-screen":{"anchor":N,"last_opened":N,"terms":[]}}
$ sqlite3 data/lyncast/clips.sqlite3 "select id, body from clips"
1|alpha
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
