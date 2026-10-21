use std::collections::VecDeque;
use std::io::{self, Write};
use std::mem;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

const QUEUED: usize = 64;

struct Queue {
    lines: VecDeque<String>,
    dropped: u64,
    writing: bool,
}

struct Lines {
    queue: Mutex<Queue>,
    changed: Condvar,
}

static LINES: Lines = Lines::new();

impl Lines {
    const fn new() -> Lines {
        Lines {
            queue: Mutex::new(Queue {
                lines: VecDeque::new(),
                dropped: 0,
                writing: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, line: String) {
        let mut queue = self.queue();
        if queue.lines.len() < QUEUED {
            queue.lines.push_back(line);
        } else {
            queue.dropped += 1;
        }
        self.changed.notify_all();
    }

    fn write(&self, out: &mut impl Write) {
        loop {
            let line = {
                let mut queue = self
                    .changed
                    .wait_while(self.queue(), |queue| {
                        queue.lines.is_empty() && queue.dropped == 0
                    })
                    .unwrap_or_else(PoisonError::into_inner);
                let line = match queue.lines.pop_front() {
                    Some(line) => line,
                    None => format!("PROBE dropped={}\n", mem::take(&mut queue.dropped)),
                };
                queue.writing = true;
                line
            };
            let _ = out.write_all(line.as_bytes());
            self.queue().writing = false;
            self.changed.notify_all();
        }
    }

    fn flush(&self, deadline: Instant) {
        let wait = deadline.saturating_duration_since(Instant::now());
        let _ = self
            .changed
            .wait_timeout_while(self.queue(), wait, |queue| {
                queue.writing || !queue.lines.is_empty() || queue.dropped > 0
            });
    }
}

const RENDERER: &str = "gpui_wgpu";

/// The log lines of GPUI's renderer at info and above, such as the GPU adapter it picks, as
/// `<LEVEL> <target>: <message>` lines among the PROBE lines.
struct Renderer;

impl log::Log for Renderer {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
            && metadata.target().split("::").next() == Some(RENDERER)
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let (level, target) = (record.level(), record.target());
            let message = record.args().to_string().replace('\n', " ");
            LINES.push(format!("{level} {target}: {message}\n"));
        }
    }

    fn flush(&self) {}
}

/// Whether the daemon prints PROBE lines, which one thread writes to stdout.
#[derive(Clone, Copy)]
pub struct Trace(bool);

impl Trace {
    pub const OFF: Trace = Trace(false);

    /// Starts the thread that writes the PROBE lines, and passes it the renderer's log lines.
    /// Fails only when the thread cannot start.
    pub fn start() -> io::Result<Trace> {
        thread::Builder::new()
            .name("trace".to_owned())
            .spawn(|| LINES.write(&mut io::stdout()))?;
        if log::set_logger(&Renderer).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
        Ok(Trace(true))
    }

    /// Queues `PROBE seq=<n> ev=<name> mono_ns=<CLOCK_MONOTONIC ns>`, read now, or counts it
    /// as dropped when `QUEUED` lines wait. Summon 0 is the app.
    pub fn event(self, seq: u32, name: &str) {
        if self.0 {
            LINES.push(format!("PROBE seq={seq} ev={name} mono_ns={}\n", mono_ns()));
        }
    }

    /// Waits until the thread has written every line queued so far, or until `deadline`.
    pub fn flush(self, deadline: Instant) {
        if self.0 {
            LINES.flush(deadline);
        }
    }
}

fn mono_ns() -> u64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes the timespec it is given, which lives through the call.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
    seconds * 1_000_000_000 + u64::try_from(now.tv_nsec).unwrap_or(0)
}
