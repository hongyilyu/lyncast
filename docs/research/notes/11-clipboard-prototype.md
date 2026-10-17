# 11. Clipboard prototype: nine measurements for plan step 6

An agent report from 2026-10-17, kept close to verbatim. A throwaway probe (`p6`, one mode per item) built on wl-clipboard-rs 0.9.4 ran under `scripts/test-env.sh run --` on the dev box. The box has 32 cores, and other sessions kept the load average between 21 and 46 throughout. "Idle" means the probe added no load, and "load" means it added one `yes` per core. Source facts are read in wl-clipboard-rs 0.9.4 and wayland-client 0.31.15.

## 1. A read right after copy_multi can see the old selection

VERIFIED under load, never seen idle.

- Without a wait: idle gave 0 stale reads in 3,000 (3 runs of 1,000). Load gave 173 bad reads in 3,000 (53, 98, and 22 per run): 169 one value behind, 3 two values behind, and 1 with 0 bytes.
- With a wait for the copier's own Changed event that carries the marker: 0 stale reads in 6,000 (3,000 idle, 3,000 under load). No wait hit the 2 s timeout. The first marked event after each call was that call's write, 0 skipped in 6,000, so the daemon can match its write by taking the next marked event.
- Wait length, from the start of the `copy_multi` call to the watcher's event: idle median 0.31 to 0.35 ms, max 3.3 ms; load median 3.0 to 3.8 ms, max 194 ms. With the channel hop and the marker read, the worst case under load was 237 ms. A 500 ms limit covered all 6,000.
- Cause, read in source: `copy_multi`'s thread reports Ok (`src/copy.rs:1054`) before `serve()` (`:1057`) flushes the queued `set_selection` (`:956`). Under load, `copy_multi`'s own return time rose from a 0.26 ms median to 2.3 to 3.1 ms, max 249 ms, which fits the serving thread waiting for a core (inferred).

## 2. Back-to-back seeds lose no event, but reads come back empty

VERIFIED. Every run delivered 200 of 200 Changed events. The failures were 0-byte reads, because a replaced source destroys itself when cancelled (`src/copy.rs:361`).

| Condition | Runs | Empty reads |
|---|---|---|
| Idle, 200 ms between pairs | 6 × 100 pairs | 9 of 1,200 seeds |
| Idle, no gap | 6 × 100 pairs | 6 of 1,200 |
| Load, 200 ms between pairs | 4 × 100 pairs | 31 of 800 |
| Load, no gap | 4 × 100 pairs | 179 of 800 |

In 6 of 400 load pairs, alpha arrived after beta and stayed on the clipboard: each `copy_multi` call flushes from its own thread on its own connection, so call order is not arrival order (cause inferred).

## 3. A Tier 2 harness that re-runs the test binary as a helper

All three shapes VERIFIED; the recommendation is (b). Each variant had 4 seed-and-read tests plus one `#[should_panic]` test; each test started its own environment with `test-env.sh start`, parsed its `export` and `unset` lines, and ran `stop DIR` in Drop.

| Variant | Passed (20 runs, `--test-threads=4`) | Wall median |
|---|---|---|
| (a1) static Mutex for the whole test, `unsafe set_var` | 20 of 20 | 1,524 ms, serial |
| (a2) static Mutex per wl-clipboard-rs call | 20 of 20 | 577 ms |
| (b) the test binary re-runs itself with `Command::env` | 20 of 20 | 577 ms |

Across 375 environments nothing was left behind. (b) needs no `unsafe`, changes no process environment, and uses the same `Command::env` path a daemon child needs. Under (a2), any wl-clipboard-rs call made outside the lock silently uses another test's anvil. With (b), every wl-clipboard-rs call must stay in the helper, because the test process keeps the developer's real `WAYLAND_DISPLAY`.

## 4. The 25 MiB cap is cheap, and a re-copied image costs two copies

- A poll-based reader capped at 25 MiB stopped a 26 MiB offer after 25.04 MiB in a median 20.9 ms (20.6 to 30.0, 7 runs), and read a 24 MiB offer in 17.9 ms (17.1 to 37.8).
- Pss_Anon of the probe (watcher plus copier): 0.2 MiB at baseline, 24.9 MiB holding the captured bytes, 0.3 MiB after the blob write, 49.5 MiB serving the re-copy with the same-bytes marker in 6 of 9 runs and 98.7 MiB in 3 of 9, and 25.0 MiB (6 runs) or 98.7 MiB (3 runs) after a foreign copy replaced it.
- Control: with `MALLOC_MMAP_THRESHOLD_=131072`, serving was 49.5 MiB and fell to 0.38 MiB after the replacement, in 6 of 6 runs. The built-in cost is two `Arc` copies of the payload (`src/copy.rs:593-616`); the rest is glibc keeping freed 24 MiB blocks in an arena (inferred from the control).

## 5. A stalled source is recovered by a poll deadline

VERIFIED. In 10 of 10 runs the copier was stopped (state T), and the read returned 0 bytes at 2,000.1 to 2,002.1 ms against a 2,000 ms deadline. The next copy read correctly in 10 of 10, in 0.08 to 0.19 ms, five times after SIGCONT and five while the copier was still stopped. `Offer::receive` returns a blocking pipe (`src/watch.rs:399-408`).

## 6. When the compositor exits, next_event returns an error

VERIFIED. In 10 of 10 runs the blocked call returned `Err(WaylandCommunication(Backend(Io(BrokenPipe))))` 18 to 29 ms after `stop` began. CPU while blocked was 0 ticks. Five more calls returned the same error in about 1.5 µs each, so a loop that retried would spin. The cancel handle stopped a blocked call with `Ok(None)` in 20 of 20 (median 0.037 ms), and a later call also returned `Ok(None)` at once, so cancellation is permanent (`src/watch.rs:148-150`, `:371-374`).

## 7. The first event is the selection already there

VERIFIED. In 23 of 23 cases the first event was Changed with the seeded bytes, a median 0.201 ms (max 0.218) after `Watcher::new` started; the constructor's roundtrip queues it (`src/watch.rs:263-265`). A text seed from wl-clipboard-rs offered five types (`text/plain;charset=utf-8`, `text/plain`, `UTF8_STRING`, `STRING`, `TEXT`), and an `image/png` seed offered only `image/png`. An empty clipboard gave CLEARED at 0.32 ms.

## 8. An image round trip gives the same PNG

VERIFIED with a 64×48 RGB PNG of 8,455 bytes, in 20 of 20 rounds: the capture equalled the file, the watcher saw the marked re-copy and skipped it, the seeder exited when the re-copy replaced it, and a `MimeType::Any` read and an `image/png` read from another process both returned all 8,455 bytes. `Any` reported `image/png` 14 times and the marker type 6 times, always the first offered type (`src/paste.rs:435-440`).

## 9. The clipboard is empty after the copier exits

VERIFIED. In 40 of 40 runs the value was readable from another process before exit. Right after waitpid the read gave `ClipboardEmpty`, for a return from main (20) and for SIGKILL (20), and it was still empty 100 ms later.
