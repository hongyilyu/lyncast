use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};
use std::mem;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::ptr;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

const QUEUED: usize = 8;

struct Queue {
    lines: VecDeque<String>,
    dropped: u64,
    writing: bool,
}

struct Warnings {
    queue: Mutex<Queue>,
    changed: Condvar,
}

static WARNINGS: Warnings = Warnings::new();

impl Warnings {
    const fn new() -> Warnings {
        Warnings {
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

    fn warn(&self, line: fmt::Arguments<'_>) {
        let mut queue = self.queue();
        if queue.lines.len() < QUEUED {
            queue.lines.push_back(format!("lyncast: {line}\n"));
        } else {
            queue.dropped += 1;
        }
        self.changed.notify_all();
    }

    fn last(&self, line: fmt::Arguments<'_>) {
        let mut queue = self.queue();
        if let Some(count) = dropped(mem::take(&mut queue.dropped)) {
            queue.lines.push_back(count);
        }
        queue.lines.push_back(format!("lyncast: {line}\n"));
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
                    None => dropped(mem::take(&mut queue.dropped))
                        .expect("the wait ends on a line or a count"),
                };
                queue.writing = true;
                line
            };
            // eprintln! panics when stderr fails, which would take the daemon down.
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

fn dropped(dropped: u64) -> Option<String> {
    match dropped {
        0 => None,
        1 => Some("lyncast: 1 warning was dropped while stderr was full\n".to_owned()),
        dropped => Some(format!(
            "lyncast: {dropped} warnings were dropped while stderr was full\n"
        )),
    }
}

/// Queues `lyncast: <line>` for stderr, or counts it as dropped when [`QUEUED`] warnings wait.
/// It never waits for stderr.
pub fn warn(line: fmt::Arguments<'_>) {
    WARNINGS.warn(line);
}

/// Queues the daemon's last line, `lyncast: <line>`, after its warnings. It is never dropped.
pub fn last(line: fmt::Arguments<'_>) {
    WARNINGS.last(line);
}

/// Starts the thread that writes the warnings, those queued before it first. Fails only when the
/// thread cannot start.
pub fn start() -> io::Result<()> {
    thread::Builder::new().spawn(|| WARNINGS.write(&mut io::stderr()))?;
    Ok(())
}

/// Waits until the thread has written every line queued so far, or until `deadline`.
pub fn flush(deadline: Instant) {
    WARNINGS.flush(deadline);
}

/// Writes `lyncast: <line>` to stderr without the thread, for a daemon whose thread did not
/// start: one write of at most `PIPE_BUF` bytes, which a pipe takes whole or not at all, and which
/// gives up once `wait` has passed. A stderr that does not drain gets nothing, so it keeps no
/// daemon from exiting.
pub fn write_alone(line: fmt::Arguments<'_>, wait: Duration) {
    let line = format!("lyncast: {line}\n");
    let bytes = &line.as_bytes()[..line.len().min(libc::PIPE_BUF)];
    let _ = write_within(io::stderr().as_fd(), bytes, wait);
}

const AGAIN: Duration = Duration::from_millis(10);

/// One write of `bytes` to `fd`, which a timer interrupts once `wait` has passed and every
/// [`AGAIN`] after it, so a write that begins after the first signal still ends. The signal goes
/// to this thread alone, and its handler does nothing and does not restart the write, which then
/// fails with `EINTR`. A process inherits its signal mask across exec, so the thread unblocks the
/// signal while the timer runs.
fn write_within(fd: BorrowedFd<'_>, bytes: &[u8], wait: Duration) -> io::Result<usize> {
    extern "C" fn interrupt(_: libc::c_int) {}
    let at = |after: Duration| libc::timespec {
        tv_sec: after.as_secs().try_into().unwrap_or(libc::time_t::MAX),
        tv_nsec: after.subsec_nanos().into(),
    };
    // A zero time would disarm the timer.
    let times = libc::itimerspec {
        it_value: at(wait.max(Duration::from_nanos(1))),
        it_interval: at(AGAIN),
    };
    // SAFETY: each call gets pointers to values that live through it, and the handler touches
    // nothing. The timer exists only between timer_create and timer_delete.
    unsafe {
        let mut action: libc::sigaction = mem::zeroed();
        action.sa_sigaction = interrupt as *const () as libc::sighandler_t;
        if libc::sigaction(libc::SIGALRM, &action, ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut event: libc::sigevent = mem::zeroed();
        event.sigev_notify = libc::SIGEV_THREAD_ID;
        event.sigev_signo = libc::SIGALRM;
        event.sigev_notify_thread_id = libc::gettid();
        let mut timer: libc::timer_t = ptr::null_mut();
        if libc::timer_create(libc::CLOCK_MONOTONIC, &mut event, &mut timer) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut alarm: libc::sigset_t = mem::zeroed();
        libc::sigemptyset(&mut alarm);
        libc::sigaddset(&mut alarm, libc::SIGALRM);
        let mut mask: libc::sigset_t = mem::zeroed();
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &alarm, &mut mask);
        let written = if libc::timer_settime(timer, 0, &times, ptr::null_mut()) == 0 {
            match libc::write(fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) {
                count if count < 0 => Err(io::Error::last_os_error()),
                count => Ok(count as usize),
            }
        } else {
            Err(io::Error::last_os_error())
        };
        libc::timer_delete(timer);
        libc::pthread_sigmask(libc::SIG_SETMASK, &mask, ptr::null_mut());
        written
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};

    use super::*;

    struct Held {
        began: Sender<()>,
        go: Receiver<()>,
        wrote: Arc<Mutex<Vec<String>>>,
    }

    impl Write for Held {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let _ = self.began.send(());
            let _ = self.go.recv();
            let line = String::from_utf8_lossy(bytes).into_owned();
            self.wrote.lock().unwrap().push(line);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    const HELD: Duration = Duration::from_millis(200);

    fn written(waits: usize) -> Vec<String> {
        let warnings: &'static Warnings = Box::leak(Box::new(Warnings::new()));
        let (began, writing) = mpsc::channel();
        let (go, held) = mpsc::channel();
        let wrote = Arc::new(Mutex::new(Vec::new()));
        let mut stderr = Held {
            began,
            go: held,
            wrote: Arc::clone(&wrote),
        };
        thread::spawn(move || warnings.write(&mut stderr));
        warnings.warn(format_args!("in hand"));
        writing.recv().unwrap();
        let started = Instant::now();
        warnings.flush(started + HELD);
        assert!(
            started.elapsed() >= HELD,
            "a flush waits for the line the thread holds"
        );
        for n in 1..=waits {
            warnings.warn(format_args!("waits {n}"));
        }
        warnings.last(format_args!("the daemon stopped"));
        drop(go);
        warnings.flush(Instant::now() + Duration::from_secs(60));
        wrote.lock().unwrap().clone()
    }

    fn want(waits: usize, count: &str) -> Vec<String> {
        ["in hand".to_owned()]
            .into_iter()
            .chain((1..=waits.min(QUEUED)).map(|n| format!("waits {n}")))
            .chain([count.to_owned(), "the daemon stopped".to_owned()])
            .map(|line| format!("lyncast: {line}\n"))
            .collect()
    }

    #[test]
    fn the_thread_writes_in_order_and_a_flush_waits_for_it() {
        assert_eq!(
            written(11),
            want(11, "3 warnings were dropped while stderr was full")
        );
    }

    const WAIT: Duration = Duration::from_millis(200);

    #[test]
    fn a_write_to_a_full_pipe_gives_up_at_its_wait() {
        let (reader, mut writer) = io::pipe().unwrap();
        crate::pipe::fill(&mut writer, b'x');
        let (wrote, written) = mpsc::channel();
        let started = Instant::now();
        thread::spawn(move || wrote.send(write_within(writer.as_fd(), b"lyncast: x\n", WAIT)));
        let result = written
            .recv_timeout(Duration::from_secs(60))
            .expect("the write gives up");
        assert_eq!(
            result.map_err(|error| error.kind()),
            Err(io::ErrorKind::Interrupted)
        );
        assert!(
            started.elapsed() >= WAIT,
            "the write gave up after {:?}",
            started.elapsed()
        );
        drop(reader);
    }
}
