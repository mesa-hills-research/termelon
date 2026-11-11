//! The writer, the one owner of the terminal's output while a program has it. Frames, out-of-band
//! commands and probe queries reach it as packets, in order, and a thread of its own writes them. A
//! terminal that stops taking output then holds up that thread alone. The program draws its next
//! frame only once the writer has written the last one, so a terminal that takes output again gets
//! the changes since the last frame it took.
//!
//! Giving the terminal back is one step, [`Writer::close`]. It ends the program's output, so
//! nothing sent after it reaches the terminal, and queues the reset once, as a packet whose end in
//! the output is recorded then and never moves. Only the writer's thread marks it written, once its
//! bytes were. What follows (the program's tail) goes after it, and each [`Ticket`] that
//! [`Writer::wait`] waits on is written only when its bytes were, and the reset too where it was
//! given for it. A failure never counts as written. An end that can't count on the writer writes
//! the reset itself ([`Takeover::reset_directly`]), once, from a thread of its own, and those that
//! come meanwhile wait for what became of it. A writer whose thread didn't start fails to start,
//! and one whose thread ended says so ([`Writer::failure`]) and wakes those waiting.
//!
//! Its state is behind one lock, held for no write, no wait but its own condvar's, and no call out.
//! Those waiting are woken after it's let go. The record of the reset is outside it, so an end that
//! can't count on the writer waits on no lock the writer holds.

use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
use std::task::{Context, Poll, Waker};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// What a reset written without the writer starts with. CAN ends an escape sequence a write cut
/// short left open, and ST (`ESC \`) a string.
pub(crate) const CANCEL: &[u8] = b"\x18\x1b\\";

/// How much the writer writes at once, so an end that takes over stops it within a chunk.
const CHUNK: usize = 4096;

/// How long an end that can't count on the writer tries for its lock.
pub(crate) const LOCK_WAIT: Duration = Duration::from_millis(50);

/// How long the thread waits for output before it checks whether any handle is left.
const IDLE_CHECK: Duration = Duration::from_secs(1);

/// How often those waiting for a reset written without the writer look at its record.
const POLL: Duration = Duration::from_millis(5);

/// No one writes the reset without the writer, or the thread to write it didn't start.
const NOT_STARTED: u8 = 0;
/// A thread of its own writes it.
const IN_FLIGHT: u8 = 1;
/// That thread wrote it.
const WRITTEN: u8 = 2;
/// That thread couldn't write it (the terminal is gone), or it broke.
const FAILED: u8 = 3;

/// A handle on the writer. Clones share one queue and one thread, which ends when the last handle
/// is dropped.
#[derive(Clone)]
pub(crate) struct Writer {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = lock(&self.shared.state);
        f.debug_struct("Writer")
            .field("pending", &state.pending.len())
            .field("writing", &state.writing)
            .field("phase", &state.phase)
            .field("failed", &state.failed)
            .finish()
    }
}

struct Shared {
    state: Mutex<State>,
    /// Rings for the writer's thread when there is something to write, and for those waiting for
    /// it to be written.
    changed: Condvar,
    record: Arc<Record>,
    /// The writer's thread, once it runs.
    thread: OnceLock<ThreadId>,
    /// What to call once the writer fails.
    on_failure: Mutex<Option<Notify>>,
}

/// Told that the writer failed.
type Notify = Arc<dyn Fn() + Send + Sync>;

impl Shared {
    /// Tells whoever asked that the writer failed. Called with no lock held.
    fn notify_failure(&self) {
        let notify = lock(&self.on_failure).clone();
        if let Some(notify) = notify {
            notify();
        }
    }
}

/// The reset and what became of it, outside the writer's lock. Restore handles keep it after the
/// writer's thread ends.
struct Record {
    /// What gives the terminal back. Its lock is held only to clone or replace it.
    reset: Mutex<Arc<[u8]>>,
    /// The writer wrote the reset ([`Phase::Reset`]), for those who look without its lock.
    reset_written: AtomicBool,
    /// Where the reset written without the writer is, [`NOT_STARTED`], [`IN_FLIGHT`],
    /// [`WRITTEN`] or [`FAILED`].
    direct: AtomicU8,
}

impl Record {
    fn reset(&self) -> Arc<[u8]> {
        lock(&self.reset).clone()
    }
}

/// Marks the reset written without the writer failed, when the thread writing it breaks before it
/// says what became of it.
struct Unwound(Arc<Record>);

impl Drop for Unwound {
    fn drop(&mut self) {
        let _ =
            self.0
                .direct
                .compare_exchange(IN_FLIGHT, FAILED, Ordering::SeqCst, Ordering::SeqCst);
    }
}

struct State {
    /// What waits to be written.
    pending: Vec<u8>,
    /// A written packet's buffer, emptied, for the next frame to fill.
    spare: Vec<u8>,
    /// How many bytes the write going on writes (none when it's zero).
    writing: usize,
    /// When the write going on started.
    started: Option<Instant>,
    /// How many bytes were queued so far, written or not. It only grows.
    queued: u64,
    /// How many of those were written, or dropped as the program's when it closed. The rest are
    /// being written or wait.
    settled: u64,
    phase: Phase,
    /// An end that can't count on the writer writes the reset itself, so the writer writes nothing
    /// more.
    taken_over: bool,
    failed: Option<Failure>,
    /// The tasks waiting in [`Writer::written`].
    wakers: Vec<Waker>,
    /// Out-of-band commands that go with the next packet the writer takes, one of each kind.
    held: Vec<(Hold, Vec<u8>)>,
    /// What the writer wrote, in order, while it keeps a record.
    record: Option<Vec<u8>>,
}

/// An out-of-band command that waits for the next packet, where a later one of its kind replaces
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    Title,
    Copy,
    Bell,
}

impl State {
    fn queue(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        self.queued += bytes.len() as u64;
    }

    /// Whether what was sent waits to be written, or is being written (the terminal hasn't taken
    /// it yet).
    fn busy(&self) -> bool {
        self.failed.is_none()
            && (self.writing > 0 || !self.pending.is_empty() || !self.held.is_empty())
    }

    /// Keeps `buffer`, emptied, for [`Writer::spare_buffer`], when it's bigger than the one kept.
    fn keep_spare(&mut self, mut buffer: Vec<u8>) {
        if buffer.capacity() > self.spare.capacity() {
            buffer.clear();
            self.spare = buffer;
        }
    }
}

/// Where giving the terminal back is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The program writes.
    Open,
    /// The program closed, and the reset ends at this many bytes queued.
    Closing(u64),
    /// The writer wrote the reset.
    Reset,
}

/// Why the writer writes no more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The terminal refused a write, usually because it closed.
    Gone(io::ErrorKind),
    /// The writer's thread ended while the program had the terminal.
    Died,
}

impl From<Failure> for io::Error {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Gone(kind) => io::Error::new(kind, "the terminal stopped taking output"),
            Failure::Died => io::Error::other("the terminal's writer stopped"),
        }
    }
}

/// What [`Writer::close`] or [`Writer::ticket`] gave, for [`Writer::wait`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    /// The bytes queued up to its end.
    end: u64,
    /// Whether it waits for the reset too.
    reset: bool,
}

/// What became of a [`Ticket`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Finish {
    /// The writer wrote it.
    Written,
    /// It won't be, as the writer failed or an end that couldn't count on it took over.
    Failed,
    /// Not yet.
    TimedOut,
}

/// What became of a reset written without the writer ([`Takeover::reset_directly`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direct {
    /// None was needed, as the writer wrote the reset.
    Unneeded,
    /// It was written.
    Written,
    /// It wasn't, because the terminal is gone, or the thread to write it didn't start (and a later
    /// call tries again).
    Failed,
    /// It's being written, and wasn't by the deadline.
    TimedOut,
}

/// Starts a thread for the writer or for a reset written without it.
pub(crate) type Spawn<'a> = &'a mut dyn FnMut(Box<dyn FnOnce() + Send>) -> io::Result<()>;

/// Opens what a reset written without the writer goes to.
pub(crate) type Open = Box<dyn FnOnce() -> Box<dyn Write + Send> + Send>;

/// Starts a thread named `name`.
pub(crate) fn spawn_named(name: &str, run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(run)
        .map(drop)
}

impl Writer {
    /// Starts the writer's thread, which writes to `out`. `reset` gives the terminal back. Fails
    /// when the thread doesn't start.
    pub(crate) fn spawn(out: impl Write + Send + 'static, reset: Vec<u8>) -> io::Result<Self> {
        Self::spawn_with(out, reset, &mut |run| spawn_named("tuiframe-writer", run))
    }

    /// [`spawn`](Self::spawn), with the thread started by `spawn`.
    pub(crate) fn spawn_with(
        out: impl Write + Send + 'static,
        reset: Vec<u8>,
        spawn: Spawn<'_>,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: Vec::new(),
                spare: Vec::new(),
                writing: 0,
                started: None,
                queued: 0,
                settled: 0,
                phase: Phase::Open,
                taken_over: false,
                failed: None,
                wakers: Vec::new(),
                held: Vec::new(),
                record: None,
            }),
            changed: Condvar::new(),
            record: Arc::new(Record {
                reset: Mutex::new(reset.into()),
                reset_written: AtomicBool::new(false),
                direct: AtomicU8::new(NOT_STARTED),
            }),
            thread: OnceLock::new(),
            on_failure: Mutex::new(None),
        });
        let watched = Arc::downgrade(&shared);
        spawn(Box::new(move || {
            let _ended = Ended(watched.clone());
            write_all(&watched, out);
        }))?;
        Ok(Self { shared })
    }

    /// Whether everything sent was written, or the writer failed. A program draws its next frame
    /// only then.
    pub(crate) fn is_idle(&self) -> bool {
        !lock(&self.shared.state).busy()
    }

    /// Why the writer writes no more, if it doesn't.
    pub(crate) fn failure(&self) -> Option<Failure> {
        lock(&self.shared.state).failed
    }

    /// When the write going on started, if one does.
    pub(crate) fn busy_since(&self) -> Option<Instant> {
        lock(&self.shared.state).started
    }

    /// Calls `notify` once the writer fails, or now if it has. It may be called more than once.
    pub(crate) fn on_failure(&self, notify: impl Fn() + Send + Sync + 'static) {
        let notify: Notify = Arc::new(notify);
        *lock(&self.shared.on_failure) = Some(notify.clone());
        if self.failure().is_some() {
            notify();
        }
    }

    /// The writer's thread, once it runs.
    pub(crate) fn thread(&self) -> Option<ThreadId> {
        self.shared.thread.get().copied()
    }

    /// Completes when everything sent so far was written, or the writer failed. It may complete
    /// early, as a later packet can make the writer busy again, so check
    /// [`is_idle`](Self::is_idle) again.
    pub(crate) fn written(&self) -> Written<'_> {
        Written { writer: self }
    }

    /// Queues `packet` after what was sent before. After [`close`](Self::close), or once an end
    /// that couldn't count on the writer took over, it's dropped. Fails once the writer failed.
    pub(crate) fn send(&self, packet: Vec<u8>) -> Result<(), Failure> {
        if packet.is_empty() {
            return self.failure().map_or(Ok(()), Err);
        }
        {
            let mut state = lock(&self.shared.state);
            if let Some(failure) = state.failed {
                return Err(failure);
            }
            if state.phase != Phase::Open || state.taken_over {
                return Ok(());
            }
            state.queued += packet.len() as u64;
            if state.pending.is_empty() {
                let empty = std::mem::replace(&mut state.pending, packet);
                state.keep_spare(empty);
            } else {
                state.pending.extend_from_slice(&packet);
                state.keep_spare(packet);
            }
        }
        self.shared.changed.notify_all();
        Ok(())
    }

    /// Holds `command` for the next packet the writer takes, which is at once while it's idle. A
    /// later command of the same kind replaces it. Dropped after [`close`](Self::close), and fails
    /// once the writer failed.
    pub(crate) fn hold(&self, kind: Hold, command: Vec<u8>) -> Result<(), Failure> {
        {
            let mut state = lock(&self.shared.state);
            if let Some(failure) = state.failed {
                return Err(failure);
            }
            if state.phase != Phase::Open || state.taken_over {
                return Ok(());
            }
            state.held.retain(|(held, _)| *held != kind);
            state.held.push((kind, command));
        }
        self.shared.changed.notify_all();
        Ok(())
    }

    /// Keeps a record of what the writer writes from now on, or stops keeping one.
    pub(crate) fn record(&self, on: bool) {
        let mut state = lock(&self.shared.state);
        if on {
            state.record.get_or_insert_with(Vec::new);
        } else {
            state.record = None;
        }
    }

    /// What the writer wrote since the record started or was last taken, empty while it keeps
    /// none.
    pub(crate) fn take_recorded(&self) -> Vec<u8> {
        lock(&self.shared.state)
            .record
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// A written packet's buffer, emptied, or a new one, so a frame reuses it rather than
    /// allocating.
    pub(crate) fn spare_buffer(&self) -> Vec<u8> {
        std::mem::take(&mut lock(&self.shared.state).spare)
    }

    /// What gives the terminal back.
    #[cfg_attr(
        all(windows, not(test)),
        expect(
            dead_code,
            reason = "only suspending sends the reset alone, and Windows can't suspend"
        )
    )]
    pub(crate) fn reset(&self) -> Arc<[u8]> {
        self.shared.record.reset()
    }

    /// Replaces the reset, which [`close`](Self::close) and a reset written without the writer
    /// send. Once the program closed, the reset already queued stays.
    pub(crate) fn set_reset(&self, reset: Vec<u8>) {
        *lock(&self.shared.record.reset) = reset.into();
    }

    /// The ticket for everything sent so far, which [`wait`](Self::wait) waits on.
    pub(crate) fn ticket(&self) -> Ticket {
        Ticket {
            end: lock(&self.shared.state).queued,
            reset: false,
        }
    }

    /// Ends the program's output and gives the terminal back. What was sent and no write has
    /// started on is dropped, and the reset is queued, with where it ends recorded for good. Then
    /// `tail`. Nothing sent from now on is written. Once more, it queues only `tail`. Returns the
    /// ticket for this call's bytes, and whether this call ended the output.
    pub(crate) fn close(&self, tail: &[u8]) -> (Ticket, bool) {
        let reset = self.shared.record.reset();
        let (ticket, ended) = {
            let mut state = lock(&self.shared.state);
            let ended = state.phase == Phase::Open;
            if ended {
                // (What no write started on counts as settled.)
                state.settled += state.pending.len() as u64;
                state.pending.clear();
                state.queue(&reset);
                state.phase = Phase::Closing(state.queued);
                // (A copy and a bell still go out, after the reset. A title would undo the one the
                // reset put back.)
                for (kind, command) in std::mem::take(&mut state.held) {
                    if kind != Hold::Title {
                        state.queue(&command);
                    }
                }
            }
            state.queue(tail);
            let ticket = Ticket {
                end: state.queued,
                reset: true,
            };
            (ticket, ended)
        };
        self.shared.changed.notify_all();
        (ticket, ended)
    }

    /// Waits at most `timeout` for the writer to write what `ticket` was given for, and the reset
    /// too for a ticket from [`close`](Self::close).
    pub(crate) fn wait(&self, ticket: Ticket, timeout: Duration) -> Finish {
        let until = Instant::now() + timeout;
        let mut state = lock(&self.shared.state);
        loop {
            if state.settled >= ticket.end && (!ticket.reset || state.phase == Phase::Reset) {
                return Finish::Written;
            }
            if state.failed.is_some() || state.taken_over {
                return Finish::Failed;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Finish::TimedOut;
            }
            state = self.wait_for_change(state, left);
        }
    }

    /// Waits at most `timeout` until everything sent is written, or the writer failed. Whether it
    /// was.
    #[cfg(test)]
    pub(crate) fn wait_idle(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        let mut state = lock(&self.shared.state);
        while state.busy() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self.wait_for_change(state, left);
        }
        true
    }

    /// What writes the reset without the writer, for restore handles.
    pub(crate) fn takeover(&self) -> Takeover {
        Takeover {
            writer: Arc::downgrade(&self.shared),
            record: self.shared.record.clone(),
        }
    }

    /// [`Takeover::reset_directly`] for this writer.
    #[cfg(test)]
    fn reset_directly(&self, wait: Duration, out: Open, spawn: Spawn<'_>) -> Direct {
        self.takeover().reset_directly(wait, out, spawn)
    }

    fn wait_for_change<'a>(
        &self,
        state: MutexGuard<'a, State>,
        left: Duration,
    ) -> MutexGuard<'a, State> {
        self.shared
            .changed
            .wait_timeout(state, left)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0
    }
}

/// The future of [`Writer::written`].
#[derive(Debug)]
#[must_use = "futures do nothing unless polled"]
pub(crate) struct Written<'a> {
    writer: &'a Writer,
}

impl Future for Written<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = lock(&self.writer.shared.state);
        if !state.busy() {
            return Poll::Ready(());
        }
        // (Every pending poll's waker is kept, so no task waits for another to be woken.)
        if !state.wakers.iter().any(|kept| kept.will_wake(cx.waker())) {
            state.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// Writes the reset without the writer. Restore handles keep one, which holds the writer's thread
/// up no longer than the writer's own handles do.
#[derive(Clone)]
pub(crate) struct Takeover {
    writer: Weak<Shared>,
    record: Arc<Record>,
}

impl std::fmt::Debug for Takeover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Takeover")
            .field(
                "reset_written",
                &self.record.reset_written.load(Ordering::SeqCst),
            )
            .field("direct", &self.record.direct.load(Ordering::SeqCst))
            .finish()
    }
}

impl Takeover {
    /// The writer, while a handle on it is left.
    pub(crate) fn writer(&self) -> Option<Writer> {
        self.writer.upgrade().map(|shared| Writer { shared })
    }

    /// For an end that can't count on the writer (a stuck program, a failed writer), writes the
    /// reset without it, unless the writer wrote it, and waits at most `wait` for what became of
    /// it. Its record is outside the writer's lock, which this takes for no wait. The first call
    /// (or the first after a thread that didn't start) claims it, stops the writer if it gets the
    /// writer's lock within [`LOCK_WAIT`], and starts a thread with `spawn` that writes [`CANCEL`] and
    /// the reset to what `out` opens. All calls then look at the record every few milliseconds until
    /// their deadline.
    pub(crate) fn reset_directly(&self, wait: Duration, out: Open, spawn: Spawn<'_>) -> Direct {
        let until = Instant::now() + wait;
        let mut out = Some(out);
        loop {
            if self.record.reset_written.load(Ordering::SeqCst) {
                return Direct::Unneeded;
            }
            match self.record.direct.load(Ordering::SeqCst) {
                WRITTEN => return Direct::Written,
                FAILED => return Direct::Failed,
                NOT_STARTED => {
                    // (This call's own thread didn't start.)
                    let Some(open) = out.take() else {
                        return Direct::Failed;
                    };
                    let claimed = self.record.direct.compare_exchange(
                        NOT_STARTED,
                        IN_FLIGHT,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );
                    if claimed.is_err() {
                        out = Some(open);
                        continue;
                    }
                    let left = until.saturating_duration_since(Instant::now());
                    self.stop_writer(left.min(LOCK_WAIT));
                    let mut bytes = CANCEL.to_vec();
                    bytes.extend_from_slice(&self.record.reset());
                    let record = self.record.clone();
                    let started = spawn(Box::new(move || {
                        let _unwound = Unwound(record.clone());
                        let mut out = open();
                        let wrote = out.write_all(&bytes).and_then(|()| out.flush());
                        let outcome = if wrote.is_ok() { WRITTEN } else { FAILED };
                        record.direct.store(outcome, Ordering::SeqCst);
                    }));
                    if started.is_err() {
                        self.record.direct.store(NOT_STARTED, Ordering::SeqCst);
                        return Direct::Failed;
                    }
                }
                _ => {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Direct::TimedOut;
                    }
                    std::thread::sleep(left.min(POLL));
                }
            }
        }
    }

    /// Makes the writer write nothing more, if a handle on it is left and its lock can be had
    /// within `within`, and wakes those waiting for it.
    fn stop_writer(&self, within: Duration) {
        let Some(shared) = self.writer.upgrade() else {
            return;
        };
        let wakers = {
            let Some(mut state) = try_lock_for(&shared.state, within) else {
                return;
            };
            state.taken_over = true;
            state.pending.clear();
            if state.busy() {
                Vec::new()
            } else {
                std::mem::take(&mut state.wakers)
            }
        };
        shared.changed.notify_all();
        wakers.into_iter().for_each(Waker::wake);
    }
}

/// Marks the writer failed, and wakes those waiting for it, when its thread ends while a handle is
/// left (a panic). Once no handle is left, no one waits.
struct Ended(Weak<Shared>);

impl Drop for Ended {
    fn drop(&mut self) {
        let Some(shared) = self.0.upgrade() else {
            return;
        };
        let (wakers, failed) = {
            let mut state = lock(&shared.state);
            let failed = state.failed.is_none();
            state.failed.get_or_insert(Failure::Died);
            state.writing = 0;
            state.started = None;
            state.pending.clear();
            (std::mem::take(&mut state.wakers), failed)
        };
        shared.changed.notify_all();
        wakers.into_iter().for_each(Waker::wake);
        if failed {
            shared.notify_failure();
        }
    }
}

/// The writer's thread. It writes what is sent to `out`, all that waits at once, until no handle
/// is left.
fn write_all(shared: &Weak<Shared>, mut out: impl Write) {
    if let Some(shared) = shared.upgrade() {
        let _ = shared.thread.set(std::thread::current().id());
    }
    loop {
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let bytes = {
            let mut state = lock(&shared.state);
            if (state.pending.is_empty() && state.held.is_empty())
                || state.failed.is_some()
                || state.taken_over
            {
                state.pending.clear();
                state.held.clear();
                // (Let go of the writer meanwhile, so the last handle dropped ends the thread.)
                let state = shared
                    .changed
                    .wait_timeout(state, IDLE_CHECK)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
                drop(state);
                continue;
            }
            let mut bytes = std::mem::take(&mut state.pending);
            for (_, command) in std::mem::take(&mut state.held) {
                state.queued += command.len() as u64;
                bytes.extend_from_slice(&command);
            }
            state.writing = bytes.len();
            state.started = Some(Instant::now());
            bytes
        };
        let wrote = write_chunks(&shared, &mut out, &bytes);
        let failed = wrote.is_err();
        let wakers = {
            let mut state = lock(&shared.state);
            state.writing = 0;
            state.started = None;
            match wrote {
                Ok(()) => {
                    state.settled += bytes.len() as u64;
                    // (Only here does the reset count as written.)
                    if let Phase::Closing(end) = state.phase
                        && state.settled >= end
                    {
                        state.phase = Phase::Reset;
                        shared.record.reset_written.store(true, Ordering::SeqCst);
                    }
                }
                Err(error) => {
                    state.failed.get_or_insert(Failure::Gone(error.kind()));
                    state.pending.clear();
                }
            }
            state.keep_spare(bytes);
            if state.busy() {
                Vec::new()
            } else {
                std::mem::take(&mut state.wakers)
            }
        };
        // (Woken with the state let go, as a task's waker may take locks of its executor's.)
        shared.changed.notify_all();
        wakers.into_iter().for_each(Waker::wake);
        if failed {
            shared.notify_failure();
        }
    }
}

/// Writes `bytes` to `out` a chunk at a time, and stops between chunks once an end that couldn't
/// count on the writer took over, so the reset it writes follows no more of the packet.
fn write_chunks(shared: &Shared, out: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    for chunk in bytes.chunks(CHUNK) {
        out.write_all(chunk)?;
        let mut state = lock(&shared.state);
        if let Some(record) = &mut state.record {
            record.extend_from_slice(chunk);
        }
        if state.taken_over {
            break;
        }
    }
    out.flush()
}

/// The lock of `mutex`, also when a thread panicked while holding it. No lock in the library is
/// held across a step that could leave its state half changed, so the state is whole either way.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The lock of `mutex`, if it can be had within `within`. std's `Mutex` has no timed lock, so it
/// tries again every millisecond.
pub(crate) fn try_lock_for<T>(mutex: &Mutex<T>, within: Duration) -> Option<MutexGuard<'_, T>> {
    let until = Instant::now() + within;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::task::Wake;

    /// A pipe whose reading end nobody reads, already full, so a write to it waits until the test
    /// reads it. Returns its two ends and how many bytes fill it.
    #[cfg(unix)]
    pub(crate) fn full_pipe() -> (std::fs::File, std::fs::File, usize) {
        use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
        let (read, mut write) = io::pipe().unwrap();
        let flags = fcntl_getfl(&write).unwrap();
        fcntl_setfl(&write, flags | OFlags::NONBLOCK).unwrap();
        let mut filled = 0;
        while let Ok(count) = write.write(&[b'.'; 4096]) {
            filled += count;
        }
        fcntl_setfl(&write, flags).unwrap();
        (
            std::fs::File::from(std::os::fd::OwnedFd::from(read)),
            std::fs::File::from(std::os::fd::OwnedFd::from(write)),
            filled,
        )
    }

    /// A terminal for tests, which keeps what it takes and takes nothing while it is held. A write
    /// waits meanwhile, as one to a terminal that doesn't read does.
    #[derive(Clone, Default)]
    pub(crate) struct Gate {
        taken: Arc<Mutex<Vec<u8>>>,
        held: Arc<(Mutex<bool>, Condvar)>,
        waiting: Arc<Mutex<usize>>,
    }

    impl Gate {
        /// Stops taking writes, or takes them again.
        pub(crate) fn hold(&self, held: bool) {
            *lock(&self.held.0) = held;
            self.held.1.notify_all();
        }

        /// What it took so far.
        pub(crate) fn taken(&self) -> Vec<u8> {
            lock(&self.taken).clone()
        }

        /// How many writes wait for it to take them.
        pub(crate) fn waiting(&self) -> usize {
            *lock(&self.waiting)
        }
    }

    impl Write for Gate {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            *lock(&self.waiting) += 1;
            let mut held = lock(&self.held.0);
            while *held {
                held = self
                    .held
                    .1
                    .wait(held)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            drop(held);
            lock(&self.taken).extend_from_slice(bytes);
            *lock(&self.waiting) -= 1;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Starts a thread for a reset written without the writer.
    pub(crate) fn spawn(run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        std::thread::Builder::new().spawn(run).map(drop)
    }

    /// Writes a reset without the writer to `gate`, waiting `wait` at most.
    fn reset_to(writer: &Writer, gate: &Gate, wait: Duration) -> Direct {
        let gate = gate.clone();
        writer.reset_directly(wait, Box::new(move || Box::new(gate)), &mut spawn)
    }

    /// A terminal whose writer breaks on the first write.
    pub(crate) struct Breaks;

    impl Write for Breaks {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            panic!("the terminal's writer breaks")
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A terminal that refuses every write, as one that closed does.
    struct Refuses;

    impl Write for Refuses {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Reads `count` bytes from `read`.
    #[cfg(unix)]
    fn take(read: &mut std::fs::File, count: usize) -> Vec<u8> {
        use std::io::Read;
        let mut bytes = vec![0; count];
        read.read_exact(&mut bytes).unwrap();
        bytes
    }

    /// Waits until the writer's thread took what was sent and waits for the terminal.
    pub(crate) fn until_writing(writer: &Writer) {
        let started = Instant::now();
        while writer.busy_since().is_none() {
            assert!(started.elapsed() < LONG, "the writer never started writing");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Wakes the thread that waits in [`block_on`].
    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    /// Runs `future` on this thread until it completes, or for `timeout` at most. Whether it
    /// completed.
    pub(crate) fn block_on(future: impl Future<Output = ()>, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if future.as_mut().poll(&mut cx).is_ready() {
                return true;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            std::thread::park_timeout(left);
        }
    }

    const LONG: Duration = Duration::from_secs(5);
    const SHORT: Duration = Duration::from_millis(100);

    /// A packet sent to a terminal that doesn't read returns at once, and the writer is busy until
    /// the terminal takes it. Meanwhile other threads go on, since only the writer's waits.
    #[cfg(unix)]
    #[test]
    fn a_terminal_that_doesnt_read_holds_up_no_other_thread() {
        let (mut read, write, filled) = full_pipe();
        let writer = Writer::spawn(write, Vec::new()).unwrap();
        let program = {
            let writer = writer.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                for frame in 0..50 {
                    writer.send(format!("frame {frame} ").into_bytes()).unwrap();
                }
                started.elapsed()
            })
        };
        assert!(program.join().unwrap() < Duration::from_secs(1));
        assert!(!writer.is_idle());
        // (A program's watchdog tells how long it has waited, from when the writer's thread took
        // the first frame.)
        until_writing(&writer);
        std::thread::sleep(SHORT);
        let since = writer.busy_since().unwrap();
        assert!(since.elapsed() >= SHORT);
        // The written() future waits meanwhile, and completes once the terminal takes it all, in
        // order.
        assert!(!block_on(writer.written(), Duration::from_millis(50)));
        let reader = std::thread::spawn(move || {
            let mut expected = Vec::new();
            for frame in 0..50 {
                write!(expected, "frame {frame} ").unwrap();
            }
            assert_eq!(take(&mut read, filled).len(), filled);
            assert_eq!(take(&mut read, expected.len()), expected);
            read
        });
        assert!(block_on(writer.written(), LONG));
        assert!(writer.is_idle());
        drop(reader.join().unwrap());
    }

    /// Once the terminal closed, what is sent is dropped, the writer isn't busy, and it says why.
    #[cfg(unix)]
    #[test]
    fn a_closed_terminal_drops_the_rest() {
        let (read, write, _) = full_pipe();
        drop(read);
        let writer = Writer::spawn(write, Vec::new()).unwrap();
        let (told, telling) = mpsc::channel();
        writer.on_failure(move || {
            let _ = told.send(());
        });
        writer.send(b"frame".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        telling.recv_timeout(LONG).unwrap();
        assert_eq!(
            writer.failure(),
            Some(Failure::Gone(io::ErrorKind::BrokenPipe))
        );
        assert_eq!(
            writer.send(b"another".to_vec()),
            Err(Failure::Gone(io::ErrorKind::BrokenPipe))
        );
        assert!(writer.is_idle());
        assert!(block_on(writer.written(), LONG));
    }

    /// What was sent after the program closed never reaches the terminal, nor what waited
    /// unwritten as it closed. Only the reset and the tail do, after the write that went on.
    #[test]
    fn nothing_sent_after_close_is_written() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        gate.hold(true);
        writer.send(b"[taken]".to_vec()).unwrap();
        until_writing(&writer);
        writer.send(b"[waiting]".to_vec()).unwrap();
        // A signal's end gives the terminal back meanwhile.
        let (ticket, ended) = writer.close(b"[tail]");
        assert!(ended);
        writer.send(b"[a frame after]".to_vec()).unwrap();
        gate.hold(false);
        assert_eq!(writer.wait(ticket, LONG), Finish::Written);
        writer.send(b"[another]".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        assert_eq!(
            String::from_utf8(gate.taken()).unwrap(),
            "[taken][reset][tail]"
        );
    }

    /// Ends that give the terminal back at once share one reset, which one of them queued. Until
    /// the writer wrote it the terminal isn't given back, and an end that can't wait writes the
    /// reset itself. Once it was written, that end has nothing to do.
    #[test]
    fn the_terminal_is_given_back_once_the_reset_is_written() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        gate.hold(true);
        writer.send(b"[frame]".to_vec()).unwrap();
        until_writing(&writer);
        let closers: Vec<_> = (0..4)
            .map(|_| {
                let writer = writer.clone();
                std::thread::spawn(move || {
                    let (ticket, ended) = writer.close(b"");
                    (ended, writer.wait(ticket, Duration::from_millis(200)))
                })
            })
            .collect();
        let results: Vec<_> = closers
            .into_iter()
            .map(|closer| closer.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|(ended, _)| *ended).count(), 1);
        assert!(results.iter().all(|(_, wait)| *wait == Finish::TimedOut));
        gate.hold(false);
        let (ticket, ended) = writer.close(b"");
        assert!(!ended);
        assert_eq!(writer.wait(ticket, LONG), Finish::Written);
        assert_eq!(reset_to(&writer, &gate, LONG), Direct::Unneeded);
        assert_eq!(String::from_utf8(gate.taken()).unwrap(), "[frame][reset]");

        // One that the terminal doesn't take is the forced end's to write.
        let stuck = Gate::default();
        let writer = Writer::spawn(stuck.clone(), b"[reset]".to_vec()).unwrap();
        stuck.hold(true);
        let (ticket, _) = writer.close(b"");
        assert_eq!(writer.wait(ticket, SHORT), Finish::TimedOut);
        let direct = Gate::default();
        assert_eq!(reset_to(&writer, &direct, LONG), Direct::Written);
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");
        assert_eq!(writer.wait(ticket, LONG), Finish::Failed);
        stuck.hold(false);
    }

    /// A writer whose thread breaks says so, isn't busy, and wakes those waiting, and the reset is
    /// left for an end that writes it itself. One whose thread doesn't start fails to start.
    #[test]
    fn a_writer_that_breaks_or_never_starts_says_so() {
        let writer = Writer::spawn(Breaks, b"[reset]".to_vec()).unwrap();
        writer.send(b"[frame]".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        assert_eq!(writer.failure(), Some(Failure::Died));
        // One told of a failure after it came hears of it at once.
        let (told, telling) = mpsc::channel();
        writer.on_failure(move || {
            let _ = told.send(());
        });
        telling.recv_timeout(LONG).unwrap();
        assert!(writer.is_idle());
        assert_eq!(writer.send(b"[frame]".to_vec()), Err(Failure::Died));
        let (ticket, _) = writer.close(b"");
        assert_eq!(writer.wait(ticket, LONG), Finish::Failed);
        let direct = Gate::default();
        assert_eq!(reset_to(&writer, &direct, LONG), Direct::Written);
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");

        let refused = Writer::spawn_with(Vec::new(), Vec::new(), &mut |_| {
            Err(io::Error::other("no thread"))
        });
        assert!(refused.is_err());
    }

    /// A writer that dies on the reset never counts it as written, also when the terminal is given
    /// back again with nothing after it, and the reset is left for an end that writes it itself.
    #[test]
    fn a_second_close_after_the_writer_died_on_the_reset_isnt_written() {
        let writer = Writer::spawn(Breaks, b"[reset]".to_vec()).unwrap();
        let (first, ended) = writer.close(b"");
        assert!(ended);
        assert_eq!(writer.wait(first, LONG), Finish::Failed);
        let (second, ended) = writer.close(b"");
        assert!(!ended);
        assert_eq!(
            writer.wait(second, Duration::from_millis(200)),
            Finish::Failed
        );
        let direct = Gate::default();
        assert_eq!(reset_to(&writer, &direct, LONG), Direct::Written);
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");
    }

    /// A reset the writer wrote stays written while the tail waits for the terminal, so an end that
    /// can't wait doesn't write another.
    #[test]
    fn a_written_reset_stays_so_while_a_tail_waits() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        let (reset, _) = writer.close(b"");
        assert_eq!(writer.wait(reset, LONG), Finish::Written);
        gate.hold(true);
        let (tail, ended) = writer.close(b"[tail]");
        assert!(!ended);
        until_writing(&writer);
        assert_eq!(writer.wait(tail, SHORT), Finish::TimedOut);
        let direct = Gate::default();
        assert_eq!(reset_to(&writer, &direct, LONG), Direct::Unneeded);
        assert!(direct.taken().is_empty());
        gate.hold(false);
    }

    /// The writer's reset and the one written without it go through each of their states. The
    /// writer's goes from open to closing, then to reset when the writer wrote it, or to failed when
    /// the writer dies, which never turns into written. The direct one goes from not started to in
    /// flight, then to written, or to failed when the terminal refuses it, or back to not started
    /// when its thread doesn't start, so a later call tries again. While it's in flight, others
    /// wait for it, and then all get what it came to.
    #[test]
    fn the_resets_go_through_each_of_their_states() {
        // Open, closing, reset.
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        writer.send(b"[open]".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        gate.hold(true);
        let (ticket, ended) = writer.close(b"");
        assert!(ended);
        writer.send(b"[sealed]".to_vec()).unwrap();
        assert_eq!(writer.wait(ticket, SHORT), Finish::TimedOut);
        gate.hold(false);
        assert_eq!(writer.wait(ticket, LONG), Finish::Written);
        assert_eq!(reset_to(&writer, &Gate::default(), LONG), Direct::Unneeded);
        assert_eq!(String::from_utf8(gate.taken()).unwrap(), "[open][reset]");

        // Closing, failed, also when given back again.
        let dies = Writer::spawn(Breaks, b"[reset]".to_vec()).unwrap();
        let (ticket, _) = dies.close(b"");
        assert_eq!(dies.wait(ticket, LONG), Finish::Failed);
        let (again, _) = dies.close(b"[tail]");
        assert_eq!(dies.wait(again, SHORT), Finish::Failed);

        // Not started, in flight, not started again when its thread doesn't start.
        let direct = Gate::default();
        let refused = {
            let direct = direct.clone();
            dies.reset_directly(LONG, Box::new(move || Box::new(direct)), &mut |_| {
                Err(io::Error::other("no thread"))
            })
        };
        assert_eq!(refused, Direct::Failed);
        assert!(direct.taken().is_empty());

        // Not started, in flight, written, with two at once and one more that gives up waiting.
        direct.hold(true);
        let closers: Vec<_> = (0..2)
            .map(|_| {
                let (writer, direct) = (dies.clone(), direct.clone());
                std::thread::spawn(move || reset_to(&writer, &direct, LONG))
            })
            .collect();
        let started = Instant::now();
        while direct.waiting() == 0 {
            assert!(started.elapsed() < LONG);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(reset_to(&dies, &direct, SHORT), Direct::TimedOut);
        direct.hold(false);
        for closer in closers {
            assert_eq!(closer.join().unwrap(), Direct::Written);
        }
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");
        // Written stays so, with nothing more written.
        assert_eq!(reset_to(&dies, &direct, LONG), Direct::Written);
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");

        // In flight, failed, when the terminal refuses it, and no other try.
        let refusing = Writer::spawn(Breaks, b"[reset]".to_vec()).unwrap();
        let (ticket, _) = refusing.close(b"");
        assert_eq!(refusing.wait(ticket, LONG), Finish::Failed);
        assert_eq!(
            refusing.reset_directly(LONG, Box::new(|| Box::new(Refuses)), &mut spawn),
            Direct::Failed
        );
        let unused = Gate::default();
        assert_eq!(reset_to(&refusing, &unused, LONG), Direct::Failed);
        assert!(unused.taken().is_empty());
    }

    /// A reset whose thread didn't start while something else held the writer's lock is left for
    /// the next end, which writes it.
    #[test]
    fn a_reset_whose_thread_didnt_start_while_the_lock_was_held_is_tried_again() {
        let writer = Writer::spawn(io::sink(), b"[reset]".to_vec()).unwrap();
        let shared = writer.shared.clone();
        let (held, holding) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let released = Arc::new(Mutex::new(released));
        let mut holder = None;
        let first = writer.reset_directly(
            Duration::from_millis(10),
            Box::new(|| Box::new(io::sink())),
            &mut |_| {
                let (shared, held, released) = (shared.clone(), held.clone(), released.clone());
                holder = Some(std::thread::spawn(move || {
                    let state = lock(&shared.state);
                    held.send(()).unwrap();
                    let _ = lock(&released).recv();
                    drop(state);
                }));
                holding.recv().unwrap();
                Err(io::Error::other("no thread"))
            },
        );
        release.send(()).unwrap();
        holder.unwrap().join().unwrap();
        assert_ne!(first, Direct::Written);
        let direct = Gate::default();
        assert_eq!(reset_to(&writer, &direct, LONG), Direct::Written);
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");
    }

    /// A thread writing the reset that breaks leaves it failed, which those waiting learn at once,
    /// not at their deadline.
    #[test]
    fn a_reset_whose_thread_breaks_counts_as_failed() {
        let writer = Writer::spawn(io::sink(), b"[reset]".to_vec()).unwrap();
        let started = Instant::now();
        let first = writer.reset_directly(
            Duration::from_secs(1),
            Box::new(|| -> Box<dyn Write + Send> { panic!("the reset's thread breaks") }),
            &mut spawn,
        );
        assert_eq!(first, Direct::Failed);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(reset_to(&writer, &Gate::default(), LONG), Direct::Failed);
    }

    /// One waiting for a reset in flight returns by its deadline, also while something else holds
    /// the writer's lock.
    #[test]
    fn waiting_for_a_reset_ends_at_its_deadline_while_the_lock_is_held() {
        let writer = Writer::spawn(io::sink(), b"[reset]".to_vec()).unwrap();
        let held = Gate::default();
        held.hold(true);
        let first = {
            let (writer, held) = (writer.clone(), held.clone());
            std::thread::spawn(move || reset_to(&writer, &held, LONG))
        };
        let started = Instant::now();
        while held.waiting() == 0 {
            assert!(started.elapsed() < LONG);
            std::thread::sleep(Duration::from_millis(1));
        }
        let waiter = {
            let writer = writer.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let direct = writer.reset_directly(
                    Duration::from_millis(200),
                    Box::new(|| Box::new(io::sink())),
                    &mut spawn,
                );
                (direct, started.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        let state = lock(&writer.shared.state);
        std::thread::sleep(Duration::from_millis(600));
        drop(state);
        let (direct, waited) = waiter.join().unwrap();
        held.hold(false);
        assert_eq!(first.join().unwrap(), Direct::Written);
        assert_eq!(direct, Direct::TimedOut);
        assert!(waited < Duration::from_millis(400), "it waited {waited:?}");
    }

    /// The record holds what was written, in order, once it's on, and nothing while it's off.
    #[test]
    fn the_record_holds_what_was_written() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        writer.send(b"[before]".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        assert!(writer.take_recorded().is_empty());
        writer.record(true);
        writer.send(b"[frame]".to_vec()).unwrap();
        writer.hold(Hold::Bell, b"[bell]".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        assert_eq!(writer.take_recorded(), b"[frame][bell]");
        let (ticket, _) = writer.close(b"[tail]");
        assert_eq!(writer.wait(ticket, LONG), Finish::Written);
        assert_eq!(writer.take_recorded(), b"[reset][tail]");
        writer.record(false);
        assert!(writer.take_recorded().is_empty());
    }

    /// Out-of-band commands held while the terminal is busy go with the next packet the writer
    /// takes, the last of each kind, and at close a copy and a bell follow the reset.
    #[test]
    fn held_commands_go_with_the_next_packet() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        writer.hold(Hold::Bell, b"[bell]".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        assert_eq!(gate.taken(), b"[bell]");
        gate.hold(true);
        writer.send(b"[frame 1]".to_vec()).unwrap();
        until_writing(&writer);
        for title in ["[a]", "[b]"] {
            writer.hold(Hold::Title, title.as_bytes().to_vec()).unwrap();
            writer.hold(Hold::Bell, b"[bell]".to_vec()).unwrap();
        }
        writer.send(b"[frame 2]".to_vec()).unwrap();
        assert!(!writer.is_idle());
        gate.hold(false);
        assert!(writer.wait_idle(LONG));
        assert_eq!(
            String::from_utf8(gate.taken()).unwrap(),
            "[bell][frame 1][frame 2][b][bell]"
        );
        gate.hold(true);
        writer.send(b"[frame 3]".to_vec()).unwrap();
        until_writing(&writer);
        writer.hold(Hold::Title, b"[title]".to_vec()).unwrap();
        writer.hold(Hold::Copy, b"[copy]".to_vec()).unwrap();
        let (ticket, _) = writer.close(b"[tail]");
        gate.hold(false);
        assert_eq!(writer.wait(ticket, LONG), Finish::Written);
        assert!(
            String::from_utf8(gate.taken())
                .unwrap()
                .ends_with("[frame 3][reset][copy][tail]")
        );
    }

    /// An end that takes over stops the writer within the chunk it's writing, so the reset doesn't
    /// wait behind the rest of a long packet.
    #[test]
    fn a_takeover_stops_the_writer_between_chunks() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        gate.hold(true);
        writer.send(vec![b'.'; 3 * CHUNK + 10]).unwrap();
        until_writing(&writer);
        let direct = Gate::default();
        assert_eq!(reset_to(&writer, &direct, LONG), Direct::Written);
        gate.hold(false);
        std::thread::sleep(SHORT);
        assert_eq!(gate.taken().len(), CHUNK);
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");
    }

    /// Every task waiting for the writer is woken when the terminal takes the frame, not only the
    /// first, and one that polls once the writer is idle completes at once.
    #[test]
    fn written_wakes_every_task_waiting() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), Vec::new()).unwrap();
        gate.hold(true);
        writer.send(b"[frame]".to_vec()).unwrap();
        until_writing(&writer);
        let tasks: Vec<_> = (0..3)
            .map(|_| {
                let writer = writer.clone();
                std::thread::spawn(move || block_on(writer.written(), LONG))
            })
            .collect();
        std::thread::sleep(Duration::from_millis(50));
        gate.hold(false);
        for task in tasks {
            assert!(task.join().unwrap());
        }
        assert!(block_on(writer.written(), Duration::ZERO));
    }

    /// A written packet's buffer comes back for the next frame, and a ticket for what was sent is
    /// written with it, without the reset.
    #[test]
    fn written_buffers_come_back_and_tickets_follow_packets() {
        let gate = Gate::default();
        let writer = Writer::spawn(gate.clone(), b"[reset]".to_vec()).unwrap();
        assert_eq!(writer.spare_buffer().capacity(), 0);
        let mut packet = Vec::with_capacity(4096);
        packet.extend_from_slice(b"[frame]");
        writer.send(packet).unwrap();
        let ticket = writer.ticket();
        assert_eq!(writer.wait(ticket, LONG), Finish::Written);
        let spare = writer.spare_buffer();
        assert!(spare.is_empty());
        assert!(spare.capacity() >= 4096);
        // A new reset replaces the one given at the start.
        writer.set_reset(b"[new reset]".to_vec());
        assert_eq!(&*writer.reset(), b"[new reset]");
        let (ticket, _) = writer.close(b"");
        assert_eq!(writer.wait(ticket, LONG), Finish::Written);
        assert_eq!(gate.taken(), b"[frame][new reset]");
        assert!(writer.thread().is_some());
    }
}
