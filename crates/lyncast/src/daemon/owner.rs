//! The owner of the daemon, which every host runs it with. It holds the [`Daemon`], the inbox,
//! and the claim, hands the daemon every event, and owns Quit's order. No [`Daemon`] method
//! blocks, and only the host waits: the headless host on the channel, the palette's on its bell.

use std::fs;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use futures::channel::oneshot;

use super::face::{Face, IsOpen, Windowless, apply};
use super::runner::Runner;
use super::serve::{Claim, Event, Events, ReplyTo, Socket, WAYLAND_CLOSED, now, spawn_with};
use super::{Answer, Daemon, Done, Handled, Job, QUIT_WAIT, Success, Task, Work, quitting};
use crate::clip::{Copier, Payload, Seen};
use crate::view;
use crate::warning;
use crate::wire::{Reply, Request, Summon};

/// Quit's wait for its warnings to reach stderr before it replies.
const WARNINGS_BEFORE_REPLY: Duration = Duration::from_secs(1);

/// Quit's wait for the replies to be written, after its own goes out, before the process exits,
/// which would cut them off.
const REPLIES_BEFORE_EXIT: Duration = Duration::from_secs(1);

/// The replies that went to connection threads and may not be written yet. Quit waits for
/// them, because the process exit would cut them off.
#[derive(Default)]
struct Unwritten(Vec<Receiver<()>>);

impl Unwritten {
    fn send(&mut self, socket: Socket, reply: Reply) {
        self.0
            .retain(|written| written.try_recv() == Err(TryRecvError::Empty));
        // The send fails only when the connection's thread is gone, with no one to tell.
        if socket.reply.send(reply).is_ok() {
            self.0.push(socket.written);
        }
    }

    /// Waits until each reply is written, or its thread gives up, or `deadline` passes.
    fn wait(self, deadline: Instant) {
        for written in self.0 {
            let _ = written.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        }
    }

    fn deliver(&mut self, reply_to: ReplyTo, answer: Answer, face: &mut dyn Face) {
        match reply_to {
            ReplyTo::Socket(socket) => {
                let summon = apply(face, &answer);
                self.send(socket, view::reply(answer, summon));
            }
            ReplyTo::Palette(answer_to) => {
                let _ = answer_to.send(answer);
            }
        }
    }

    /// Answers an event that is still queued once Quit stops waiting, without handing it to the
    /// daemon. A job's result is not taken either, so its client hears that the daemon is
    /// quitting, even when its program started.
    fn refuse(&mut self, event: Event, face: &mut dyn Face) {
        if let Event::Request(_, reply_to) | Event::Finished(_, _, reply_to) = event {
            self.deliver(reply_to, quitting(), face);
        }
    }
}

/// The daemon and what a host needs around it: the inbox, the claim, and how far the stop has
/// gone. It is the one owner of the dispatch and of Quit's order, for every host. Each event
/// reaches the daemon through it, and it begins the stop when an event asks for one, so no host
/// chooses between serving and stopping.
pub(crate) struct Owner {
    daemon: Daemon,
    inbox: Receiver<Event>,
    claim: Claim,
    phase: Phase,
    /// A sender for the worker threads and the copier's results, and the host's bell.
    events: Events,
    runner: Runner,
    copier: Option<Copier>,
    unwritten: Unwritten,
}

enum Phase {
    Serving,
    Stopping(Stopping),
}

enum Stop {
    Quit(Answer, ReplyTo),
    Closed,
}

/// A stop that has begun: why, and when Quit's 5 s run out.
struct Stopping {
    stop: Stop,
    deadline: Instant,
}

impl Stopping {
    /// How long the host may wait for the next event, or `None` once the daemon has stopped or
    /// the 5 s are up.
    fn wait(&self, daemon: &Daemon) -> Option<Duration> {
        if daemon.stopped() {
            return None;
        }
        time_left(self.deadline)
    }
}

/// The time left before `deadline`, or `None` once it has passed. A queued event comes back at
/// once even when the wait is zero, so the host asks before each one.
fn time_left(deadline: Instant) -> Option<Duration> {
    let wait = deadline.saturating_duration_since(Instant::now());
    (!wait.is_zero()).then_some(wait)
}

/// What a host waits for, once the owner has handed the daemon what it may take now.
pub(crate) enum Wait {
    /// The daemon serves: the next event, however long it takes.
    Forever,
    /// The stop has begun: the next event, for this long at most.
    For(Duration),
    /// The stop's wait is over, and [`Owner::finish_blocking`] ends it.
    Over(WaitOver),
}

/// Proof that the stop's wait is over, which [`Owner::finish_blocking`] takes, so no host
/// finishes a stop that has not begun or is still waiting.
pub(crate) struct WaitOver(());

impl Owner {
    pub(super) fn new(
        daemon: Daemon,
        inbox: Receiver<Event>,
        claim: Claim,
        events: Events,
        runner: Runner,
        copier: Option<Copier>,
    ) -> Owner {
        Owner {
            daemon,
            inbox,
            claim,
            phase: Phase::Serving,
            events,
            runner,
            copier,
            unwritten: Unwritten::default(),
        }
    }

    /// The daemon, for reads that change nothing.
    #[cfg(test)]
    pub(crate) fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    /// The palette's own request, which skips the inbox. Its answer is in the receiver at once
    /// unless the request starts a job, and then it comes when the job ends. A Quit begins the
    /// stop here and rings the bell, so the owner task starts its wait, and the Quit's answer
    /// comes once the stop has finished.
    pub(crate) fn ask(
        &mut self,
        face: &mut dyn Face,
        request: Request,
    ) -> oneshot::Receiver<Answer> {
        let (answer_to, answer) = oneshot::channel();
        let event = Event::Request(request, ReplyTo::Palette(answer_to));
        if let Some(stop) = self.hand(face, event) {
            self.stop(stop, face);
            self.events.ring();
        }
        answer
    }

    /// Hands the daemon each queued event it may take now, without blocking, and gives what the
    /// host waits for next. Once the stop has begun, the time is checked before each event.
    pub(crate) fn take_queued(&mut self, face: &mut dyn Face) -> Wait {
        loop {
            let wait = self.wait();
            if let Wait::Over(_) = wait {
                return wait;
            }
            let Ok(event) = self.inbox.try_recv() else {
                return wait;
            };
            if let Some(stop) = self.hand(face, event) {
                self.stop(stop, face);
            }
        }
    }

    /// Ends the stop. Each event still queued fails, the lock goes only if the daemon has
    /// stopped, the warnings get up to [`WARNINGS_BEFORE_REPLY`] to reach stderr before Quit's
    /// answer goes out, and the replies get up to [`REPLIES_BEFORE_EXIT`] to be written, so it
    /// blocks for 2 s at most.
    pub(crate) fn finish_blocking(
        mut self,
        _: WaitOver,
        face: &mut dyn Face,
    ) -> Result<(), String> {
        for event in self.inbox.try_iter() {
            self.unwritten.refuse(event, face);
        }
        let Owner {
            daemon,
            claim,
            phase,
            mut unwritten,
            ..
        } = self;
        let Phase::Stopping(Stopping { stop, .. }) = phase else {
            unreachable!("only a stop that has begun gives a WaitOver");
        };
        claim.give_up(daemon.stopped());
        let stopped = match stop {
            Stop::Quit(answer, reply_to) => {
                warning::flush(Instant::now() + WARNINGS_BEFORE_REPLY);
                unwritten.deliver(reply_to, answer, face);
                Ok(())
            }
            Stop::Closed => Err(WAYLAND_CLOSED.to_owned()),
        };
        unwritten.wait(Instant::now() + REPLIES_BEFORE_EXIT);
        stopped
    }

    /// Stops a daemon whose host's loop ended before its stop finished, as the palette's does
    /// once the compositor goes, blocking on the inbox as the headless host does. A stop that has
    /// begun goes on. While the daemon still serves, it takes events for up to Quit's 5 s, for the
    /// capture thread to see the compositor go, and begins that stop itself if none comes. The
    /// windows went with the loop, so the summon is closed. It gives nothing, because the
    /// compositor's end is the exit even when the stop answers a Quit.
    pub(crate) fn stop_blocking(mut self) {
        let mut face = Windowless(Summon::Closed);
        let deadline = Instant::now() + QUIT_WAIT;
        while let Phase::Serving = self.phase {
            let event = time_left(deadline).and_then(|wait| self.inbox.recv_timeout(wait).ok());
            let Some(event) = event else {
                self.daemon.quit_now();
                self.stop(Stop::Closed, &mut face);
                break;
            };
            if let Some(stop) = self.hand(&mut face, event) {
                self.stop(stop, &mut face);
            }
        }
        let _ = self.serve_blocking(&mut face);
    }

    /// A host on a std thread: each event as it comes, blocking on the inbox, the stop's wait,
    /// and the finish.
    pub(super) fn serve_blocking(mut self, face: &mut dyn Face) -> Result<(), String> {
        let over = loop {
            let event = match self.wait() {
                Wait::Forever => self.inbox.recv().expect("the owner holds a sender"),
                Wait::For(wait) => match self.inbox.recv_timeout(wait) {
                    Ok(event) => event,
                    Err(_) => break WaitOver(()),
                },
                Wait::Over(over) => break over,
            };
            if let Some(stop) = self.hand(face, event) {
                self.stop(stop, face);
            }
        };
        self.finish_blocking(over, face)
    }

    fn wait(&self) -> Wait {
        match &self.phase {
            Phase::Serving => Wait::Forever,
            Phase::Stopping(stopping) => match stopping.wait(&self.daemon) {
                Some(wait) => Wait::For(wait),
                None => Wait::Over(WaitOver(())),
            },
        }
    }

    /// Every event reaches the daemon here. While the daemon serves, it gives the stop an event
    /// asks for, which its caller begins. Once the stop has begun, no event gives another.
    #[must_use]
    fn hand(&mut self, face: &mut dyn Face, event: Event) -> Option<Stop> {
        match self.phase {
            Phase::Serving => self.dispatch(face, event),
            Phase::Stopping(_) => {
                self.drain(face, event);
                None
            }
        }
    }

    /// Hands one event to the daemon while it serves, and sends the answer it gives, or starts
    /// the job it hands back. Gives back the answer to a Quit, which waits, or the compositor's
    /// end.
    #[must_use]
    fn dispatch(&mut self, face: &mut dyn Face, event: Event) -> Option<Stop> {
        match event {
            Event::Request(request, reply_to) => match self.daemon.handle(request, now()) {
                Handled::Quit(answer) => return Some(Stop::Quit(answer, reply_to)),
                handled => self.answer(handled, reply_to, face),
            },
            Event::Seen(Seen::Closed) => {
                self.daemon.compositor_closed();
                return Some(Stop::Closed);
            }
            event => self.drain(face, event),
        }
        None
    }

    /// Hands one event to the daemon, and no event here begins a stop. The stop hands it every
    /// event, since the daemon then fails every request and the compositor's end only ends the
    /// capture thread, and [`Owner::dispatch`] hands it the events that never begin one.
    fn drain(&mut self, face: &mut dyn Face, event: Event) {
        match event {
            Event::Request(request, reply_to) => {
                let handled = self.daemon.handle(request, now());
                self.answer(handled, reply_to, face);
            }
            Event::Seen(Seen::Entry(entry)) => self.daemon.captured(entry, now()),
            Event::Seen(Seen::Ended | Seen::Closed) => self.daemon.capture_ended(),
            Event::Finished(success, done, reply_to) => {
                let answer = self.daemon.finish(success, done, now());
                self.unwritten.deliver(reply_to, answer, face);
            }
            Event::WriterEnded(writer) => self.daemon.writer_ended(writer),
            Event::Unsaved(id) => self.daemon.unsaved(id),
        }
    }

    /// Sends the answer that a request came to, or starts its job. A Quit gets the answer of any
    /// request once the stop has begun, because [`Owner::dispatch`] takes the Quit that begins it.
    fn answer(&mut self, handled: Handled, reply_to: ReplyTo, face: &mut dyn Face) {
        match handled {
            Handled::Answer(answer) => self.unwritten.deliver(reply_to, answer, face),
            Handled::Job(job) => self.start(job, reply_to, face),
            Handled::Quit(_) => self.unwritten.deliver(reply_to, quitting(), face),
        }
    }

    /// Begins the stop: the socket goes, so no new client connects, the summon closes, and
    /// Quit's 5 s start.
    fn stop(&mut self, stop: Stop, face: &mut dyn Face) {
        let _ = fs::remove_file(&self.claim.socket);
        if let Some(open) = IsOpen::of(face) {
            face.close(open);
        }
        self.phase = Phase::Stopping(Stopping {
            stop,
            deadline: Instant::now() + QUIT_WAIT,
        });
    }

    /// Runs the job's task on a worker thread, or hands its copy to the copier, and its result
    /// comes back as an event.
    fn start(&mut self, job: Job, reply_to: ReplyTo, face: &mut dyn Face) {
        let Job { work, success } = job;
        match work {
            Work::Task(task) => self.run(task, success, reply_to, face),
            Work::Copy(payload) => self.copy(payload, success, reply_to),
        }
    }

    fn copy(&self, payload: Payload, success: Success, reply_to: ReplyTo) {
        let copier = self
            .copier
            .as_ref()
            .expect("the daemon hands out a copy only with a compositor");
        let events = self.events.clone();
        copier.copy(payload, move |copied| {
            let done = copied.map(|()| Done::Copied);
            events.send_or_drop(Event::Finished(success, done, reply_to));
        });
    }

    /// A task whose thread cannot start finishes at once, failed.
    fn run(&mut self, task: Task, success: Success, reply_to: ReplyTo, face: &mut dyn Face) {
        let (events, runner) = (self.events.clone(), self.runner.clone());
        let started = spawn_with(
            (task, success, reply_to),
            move |(task, success, reply_to)| {
                let done = task.run(&runner);
                events.send_or_drop(Event::Finished(success, done, reply_to));
            },
        );
        if let Err(((_, success, reply_to), error)) = started {
            let done = Err(format!("not run: {error}"));
            let answer = self.daemon.finish(success, done, now());
            self.unwritten.deliver(reply_to, answer, face);
        }
    }
}
