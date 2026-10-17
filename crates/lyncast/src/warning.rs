//! The daemon's warnings. One thread writes them to stderr, so a stderr that stops draining, such
//! as a pipe that no one reads, holds up that thread and no other. At most [`QUEUED`] warnings
//! wait for it, and one that does not fit is dropped and counted. docs/design/daemon.md describes
//! them.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};
use std::mem;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

/// How many warnings may wait while stderr does not drain.
const QUEUED: usize = 8;

/// The warnings that wait for the thread.
struct Queue {
    lines: VecDeque<String>,
    /// The warnings that did not fit since the thread last wrote their count.
    dropped: u64,
    /// The thread has taken a line and not written all of it yet.
    writing: bool,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue {
    lines: VecDeque::new(),
    dropped: 0,
    writing: false,
});

/// Wakes the thread when a warning comes, and [`flush`] when the thread has written one.
static CHANGED: Condvar = Condvar::new();

fn queue() -> MutexGuard<'static, Queue> {
    // The queue holds plain values, so a panic while it was locked leaves it whole.
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queues `lyncast: <line>` for stderr, or counts it as dropped when [`QUEUED`] warnings wait.
/// It never waits for stderr.
pub fn warn(line: fmt::Arguments<'_>) {
    let mut queue = queue();
    if queue.lines.len() < QUEUED {
        queue.lines.push_back(format!("lyncast: {line}\n"));
    } else {
        queue.dropped += 1;
    }
    CHANGED.notify_all();
}

/// Starts the thread that writes the warnings, those queued before it first. Fails only when the
/// thread cannot start.
pub fn start() -> io::Result<()> {
    thread::Builder::new().spawn(write)?;
    Ok(())
}

/// Writes each warning in the order it came, and after the warnings that waited, how many did
/// not fit.
fn write() {
    loop {
        let line =
            {
                let mut queue = CHANGED
                    .wait_while(queue(), |queue| {
                        queue.lines.is_empty() && queue.dropped == 0
                    })
                    .unwrap_or_else(PoisonError::into_inner);
                let line = queue.lines.pop_front().unwrap_or_else(|| {
                    match mem::take(&mut queue.dropped) {
                        1 => "lyncast: 1 warning was dropped while stderr was full\n".to_owned(),
                        dropped => {
                            format!(
                                "lyncast: {dropped} warnings were dropped while stderr was full\n"
                            )
                        }
                    }
                });
                queue.writing = true;
                line
            };
        // eprintln! panics when stderr fails, which would take the daemon down.
        let _ = io::stderr().write_all(line.as_bytes());
        queue().writing = false;
        CHANGED.notify_all();
    }
}

/// Waits until the thread has written every warning queued so far, or until `deadline`.
pub fn flush(deadline: Instant) {
    let wait = deadline.saturating_duration_since(Instant::now());
    let _ = CHANGED.wait_timeout_while(queue(), wait, |queue| {
        queue.writing || !queue.lines.is_empty() || queue.dropped > 0
    });
}
