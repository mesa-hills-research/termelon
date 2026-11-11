//! The terminal on Unix. Finding it, raw mode, its size, and the reader thread, which waits on the
//! terminal's input, a pipe for each signal it handles and a wake pipe.

use std::ffi::c_int;
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustix::event::Timespec;
use rustix::termios::{self, OptionalActions, SpecialCodeIndex, Termios};

use rustix::io::Errno;

use super::Shared;
use crate::event::Event;
use crate::event::decode::{Decoded, Decoder, Query};
use crate::layout::Size;
use crate::output::writer::lock;

/// How much the reader reads at once.
const READ_SIZE: usize = 4096;

/// How long [`Reader::park`] and [`Reader::stop`] wait for the reader's thread.
const CONTROL_WAIT: Duration = Duration::from_secs(1);

/// How long the input stays silent before the decoder gives up on a sequence, a string or a paste
/// it holds, which a terminal that stopped halfway leaves held for good.
const HELD_WAIT: Duration = Duration::from_secs(1);

/// How long the reader waits for the SIGCONT that ended a suspend, once the program runs again.
const RESUME_WAIT: Duration = Duration::from_millis(100);

/// The terminal, a descriptor of the library's own for each of its ends.
#[derive(Debug)]
pub(crate) struct Tty {
    pub(crate) input: OwnedFd,
    pub(crate) output: OwnedFd,
}

impl Tty {
    /// Standard input when it's a terminal, else `/dev/tty`, for reading, and standard output when
    /// it's a terminal, else `/dev/tty`, for writing. Fails when neither is a terminal and the
    /// process has no controlling terminal.
    pub(crate) fn open() -> io::Result<Self> {
        let mut tty = None;
        let input = own(rustix::stdio::stdin(), &mut tty)?;
        let output = own(rustix::stdio::stdout(), &mut tty)?;
        Ok(Self { input, output })
    }
}

/// A descriptor of `fd`'s own when it's a terminal, else of `/dev/tty`, which opens once into
/// `tty`.
fn own(fd: BorrowedFd<'_>, tty: &mut Option<OwnedFd>) -> io::Result<OwnedFd> {
    use rustix::fs::{Mode, OFlags};
    if termios::isatty(fd) {
        return fd.try_clone_to_owned();
    }
    if let Some(tty) = tty {
        return tty.try_clone();
    }
    let opened = rustix::fs::open(
        "/dev/tty",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let own = opened.try_clone()?;
    *tty = Some(opened);
    Ok(own)
}

/// The terminal's modes as the program found them, and in raw mode.
pub(crate) struct Modes {
    fd: OwnedFd,
    saved: Termios,
    raw: Termios,
}

impl std::fmt::Debug for Modes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Modes").field("fd", &self.fd).finish()
    }
}

impl Modes {
    /// Saves the modes of the terminal `fd` reads from and puts it in raw mode, where each byte
    /// arrives as it's typed (`VMIN` 1, `VTIME` 0), with no echo and no signals from keys. The
    /// change applies at once (`TCSANOW`), so keys typed before it stay to be read. Fails when `fd`
    /// isn't a terminal.
    pub(crate) fn enter(fd: BorrowedFd<'_>) -> io::Result<Self> {
        let fd = fd.try_clone_to_owned()?;
        let saved = termios::tcgetattr(&fd)?;
        let mut raw = saved.clone();
        raw.make_raw();
        raw.special_codes[SpecialCodeIndex::VMIN] = 1;
        raw.special_codes[SpecialCodeIndex::VTIME] = 0;
        termios::tcsetattr(&fd, OptionalActions::Now, &raw)?;
        Ok(Self { fd, saved, raw })
    }

    /// The byte the terminal sends for Backspace, `0x08` where its erase character is Ctrl+H, else
    /// `0x7f`.
    pub(crate) fn erase(&self) -> u8 {
        match self.saved.special_codes[SpecialCodeIndex::VERASE] {
            0x08 => 0x08,
            _ => 0x7f,
        }
    }

    /// Puts the modes back as the program found them, exactly. Input not read yet is dropped
    /// first, such as mouse reports in flight, so the shell doesn't get it.
    pub(crate) fn restore(&self) -> io::Result<()> {
        let _ = termios::tcflush(&self.fd, termios::QueueSelector::IFlush);
        Ok(termios::tcsetattr(
            &self.fd,
            OptionalActions::Now,
            &self.saved,
        )?)
    }

    /// Puts raw mode back, after something else changed the modes.
    pub(crate) fn apply_raw(&self) -> io::Result<()> {
        Ok(termios::tcsetattr(
            &self.fd,
            OptionalActions::Now,
            &self.raw,
        )?)
    }
}

/// The terminal's size, in cells and in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowSize {
    pub(crate) cells: Size,
    /// Zero where the terminal doesn't fill it in.
    pub(crate) pixels: Size,
}

/// The size of the terminal `fd` is open on.
pub(crate) fn window_size(fd: BorrowedFd<'_>) -> io::Result<WindowSize> {
    let size = termios::tcgetwinsize(fd)?;
    Ok(WindowSize {
        cells: Size::new(size.ws_col, size.ws_row),
        pixels: Size::new(size.ws_xpixel, size.ws_ypixel),
    })
}

/// Stops the process as SIGTSTP's default action does, and returns once it runs again. It raises
/// SIGSTOP on this thread, which stops with the rest before the call returns, since the library
/// handles SIGTSTP itself while it has the terminal.
pub(crate) fn stop_now() -> io::Result<()> {
    signal_hook::low_level::emulate_default_handler(signal_hook::consts::SIGTSTP)
}

/// Sends SIGTSTP to the process's group, as Ctrl+Z does in a shell, so the program's children
/// stop with it.
pub(crate) fn stop_group() -> io::Result<()> {
    use rustix::process::{Signal, kill_current_process_group};
    Ok(kill_current_process_group(Signal::TSTP)?)
}

/// The signals that end a program, which the library gives the terminal back on where the program
/// asks it to, before their default action.
pub(crate) const ENDING: [c_int; 4] = [
    signal_hook::consts::SIGTERM,
    signal_hook::consts::SIGHUP,
    signal_hook::consts::SIGINT,
    signal_hook::consts::SIGQUIT,
];

/// The signals whose default action is held off while a terminal holds them, each with the
/// condition that lets it run again and how many hold it. signal-hook leaves its handler in place
/// once installed, so without the condition a signal would do nothing after the terminal closed.
static DEFAULTS: Mutex<Vec<(c_int, Arc<AtomicBool>, usize)>> = Mutex::new(Vec::new());

/// Holds off `signal`'s default action until [`release_default`].
fn hold_default(signal: c_int) -> io::Result<()> {
    let mut defaults = lock(&DEFAULTS);
    if let Some(entry) = defaults.iter_mut().find(|(held, ..)| *held == signal) {
        entry.2 += 1;
        entry.1.store(false, Ordering::SeqCst);
        return Ok(());
    }
    let run_default = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register_conditional_default(signal, run_default.clone())?;
    defaults.push((signal, run_default, 1));
    Ok(())
}

/// Lets `signal`'s default action run again once nothing else holds it off.
fn release_default(signal: c_int) {
    let mut defaults = lock(&DEFAULTS);
    if let Some(entry) = defaults.iter_mut().find(|(held, ..)| *held == signal) {
        entry.2 = entry.2.saturating_sub(1);
        if entry.2 == 0 {
            entry.1.store(true, Ordering::SeqCst);
        }
    }
}

/// The pipes the signal handlers write to, so the reader learns which came.
#[derive(Debug)]
pub(crate) struct Signals {
    resize: PipeReader,
    resume: PipeReader,
    /// SIGTSTP, from Ctrl+Z in another program's raw terminal, `kill`, or the program's suspend.
    stop: PipeReader,
    /// The ending signals, where the program asked for them, with a flag for each that says which
    /// came.
    end: PipeReader,
    ends: Vec<(c_int, Arc<AtomicBool>)>,
    registered: Vec<signal_hook::SigId>,
    /// The signals whose default this holds off.
    held: Vec<c_int>,
    /// The writing ends of pipes no signal writes to, for tests, kept so the pipes stay open.
    #[cfg(test)]
    _kept: Vec<PipeWriter>,
}

impl Signals {
    /// Registers SIGWINCH, SIGCONT and SIGTSTP, whose handlers write to the pipes, and with
    /// `ending` SIGTERM, SIGHUP, SIGINT and SIGQUIT too.
    pub(crate) fn register(ending: bool) -> io::Result<Self> {
        use signal_hook::consts::{SIGCONT, SIGTSTP, SIGWINCH};
        use signal_hook::low_level::pipe;
        let (resize, resize_end) = io::pipe()?;
        let (resume, resume_end) = io::pipe()?;
        let (stop, stop_end) = io::pipe()?;
        let (end, end_end) = io::pipe()?;
        let mut signals = Self {
            resize,
            resume,
            stop,
            end,
            ends: Vec::new(),
            registered: Vec::new(),
            held: Vec::new(),
            #[cfg(test)]
            _kept: Vec::new(),
        };
        // (Each registration owns its pipe's end, and closes it when unregistered. Dropping
        // `signals` on an error unregisters what was registered.)
        signals
            .registered
            .push(pipe::register(SIGWINCH, resize_end)?);
        signals
            .registered
            .push(pipe::register(SIGCONT, resume_end)?);
        hold_default(SIGTSTP)?;
        signals.held.push(SIGTSTP);
        signals.registered.push(pipe::register(SIGTSTP, stop_end)?);
        if ending {
            for signal in ENDING {
                hold_default(signal)?;
                signals.held.push(signal);
                let came = Arc::new(AtomicBool::new(false));
                signals
                    .registered
                    .push(signal_hook::flag::register(signal, came.clone())?);
                signals.ends.push((signal, came));
                signals
                    .registered
                    .push(pipe::register(signal, end_end.try_clone()?)?);
            }
        }
        drop(end_end);
        Ok(signals)
    }

    /// Pipes no signal writes to, with the ends that stand in for SIGWINCH and SIGCONT.
    #[cfg(test)]
    pub(crate) fn unregistered() -> io::Result<(Self, PipeWriter, PipeWriter)> {
        let (resize, resize_end) = io::pipe()?;
        let (resume, resume_end) = io::pipe()?;
        let (stop, stop_end) = io::pipe()?;
        let (end, end_end) = io::pipe()?;
        let signals = Self {
            resize,
            resume,
            stop,
            end,
            ends: Vec::new(),
            registered: Vec::new(),
            held: Vec::new(),
            _kept: vec![stop_end, end_end],
        };
        Ok((signals, resize_end, resume_end))
    }

    /// The ending signal that came, if one did.
    fn ending(&self) -> Option<c_int> {
        self.ends
            .iter()
            .find(|(_, came)| came.swap(false, Ordering::SeqCst))
            .map(|(signal, _)| *signal)
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for id in self.registered.drain(..) {
            signal_hook::low_level::unregister(id);
        }
        for signal in self.held.drain(..) {
            release_default(signal);
        }
    }
}

/// Which of `fds` have input or hung up, waiting at most `timeout`, or with no limit for `None`.
#[cfg(not(target_os = "macos"))]
pub(crate) fn wait<const N: usize>(
    fds: [BorrowedFd<'_>; N],
    timeout: Option<Duration>,
) -> io::Result<[bool; N]> {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut polled = fds.map(|fd| PollFd::from_borrowed_fd(fd, PollFlags::IN));
    let timeout = timeout.map(timespec);
    poll(&mut polled, timeout.as_ref())?;
    Ok(polled.map(|fd| !fd.revents().is_empty()))
}

#[cfg(target_os = "macos")]
pub(crate) use super::macos::wait;

/// `duration` for `poll` and `select`, at most a day.
pub(crate) fn timespec(duration: Duration) -> Timespec {
    const DAY: Duration = Duration::from_secs(86_400);
    Timespec::try_from(duration.min(DAY)).unwrap_or(Timespec {
        tv_sec: 86_400,
        tv_nsec: 0,
    })
}

/// Reads what a pipe that is ready holds, to empty it. Each byte only says the pipe was written.
fn drain(pipe: &PipeReader) {
    let _ = (&*pipe).read(&mut [0; 64]);
}

/// The decoding the reader does on what it reads, which [`Decoder`] does for the library.
pub(crate) trait Decode: Send + 'static {
    /// Decodes `bytes`, appending what it found to `out`, and keeps an incomplete sequence for the
    /// next call.
    fn feed(&mut self, bytes: &[u8], out: &mut Vec<Decoded>);

    /// Whether an incomplete sequence is kept.
    fn has_pending(&self) -> bool;

    /// Decodes the incomplete sequence kept as if no more input follows.
    fn flush_pending(&mut self, out: &mut Vec<Decoded>);

    /// Records a query whose answer is outstanding.
    fn expect(&mut self, query: Query);

    /// Forgets the queries whose answers are outstanding.
    fn cancel_expected(&mut self);

    /// Whether anything at all is kept, a sequence with more of it, a string or a paste included.
    fn has_held(&self) -> bool;

    /// Gives up on what's kept, as the input has been silent too long for it to be finished.
    fn flush_held(&mut self, out: &mut Vec<Decoded>);
}

impl Decode for Decoder {
    fn feed(&mut self, bytes: &[u8], out: &mut Vec<Decoded>) {
        Decoder::feed(self, bytes, out);
    }

    fn has_pending(&self) -> bool {
        Decoder::has_pending(self)
    }

    fn flush_pending(&mut self, out: &mut Vec<Decoded>) {
        Decoder::flush_pending(self, out);
    }

    fn expect(&mut self, query: Query) {
        Decoder::expect(self, query);
    }

    fn cancel_expected(&mut self) {
        Decoder::cancel_expected(self);
    }

    fn has_held(&self) -> bool {
        Decoder::has_held(self)
    }

    fn flush_held(&mut self, out: &mut Vec<Decoded>) {
        Decoder::flush_held(self, out);
    }
}

/// What the program asks of the decoder, which the reader passes on before it decodes more.
#[derive(Debug, Clone, Copy)]
enum Ask {
    Expect(Query),
    Cancel,
}

/// What the reader does on the signals that concern the terminal.
pub(crate) struct Hooks {
    /// On SIGCONT that the reader didn't stop for, before it queues [`Event::Resumed`], to take
    /// the terminal back after something else had it.
    pub(crate) resume: Box<dyn FnMut() + Send>,
    /// On SIGTSTP, to give the terminal back, stop, and take it back once the process runs again.
    pub(crate) stop: Box<dyn FnMut() + Send>,
    /// On an ending signal, to give the terminal back before the signal's default action.
    pub(crate) end: Box<dyn FnMut(c_int) + Send>,
}

impl Hooks {
    /// Hooks that do nothing.
    #[cfg(test)]
    pub(crate) fn none() -> Self {
        Self {
            resume: Box::new(|| {}),
            stop: Box::new(|| {}),
            end: Box::new(|_| {}),
        }
    }
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooks").finish_non_exhaustive()
    }
}

/// The reader thread's handle.
#[derive(Debug)]
pub(crate) struct Reader {
    thread: Option<JoinHandle<()>>,
    control: Arc<Control>,
    wake: PipeWriter,
}

#[derive(Debug, Default)]
struct Control {
    state: Mutex<Requests>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct Requests {
    /// The program asks the reader to hold still.
    park: bool,
    /// The reader holds still.
    parked: bool,
    stop: bool,
    /// The reader is handling a SIGTSTP.
    stopping: bool,
    /// How many SIGTSTPs it handled.
    stops: u64,
    /// The reader's thread ended.
    ended: bool,
    /// Queries whose answers are outstanding, and their cancelling, for the decoder to learn of
    /// before it decodes more.
    asks: Vec<Ask>,
    /// A new escape timeout.
    escape_timeout: Option<Duration>,
}

impl Control {
    fn wait_until(&self, done: impl Fn(&Requests) -> bool) -> bool {
        let until = Instant::now() + CONTROL_WAIT;
        let mut requests = lock(&self.state);
        while !done(&requests) {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            requests = self
                .changed
                .wait_timeout(requests, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

impl Reader {
    /// Starts the reader's thread. It reads from `input`, decodes with `decoder`, queues the events
    /// in `shared` and offers the replies to its reply slot. A sequence the decoder keeps waits `escape_timeout` for the rest, or
    /// decodes at once when it's zero and no more input is ready. On SIGWINCH it reads the size
    /// and queues [`Event::Resize`], and on SIGCONT it calls `resume` and queues
    /// [`Event::Resumed`]. When the input closes or fails, or the thread ends, the queue ends.
    pub(crate) fn spawn(
        input: OwnedFd,
        decoder: impl Decode,
        shared: Arc<Shared>,
        escape_timeout: Duration,
        signals: Signals,
        hooks: Hooks,
    ) -> io::Result<Self> {
        let (wake_end, wake) = io::pipe()?;
        let control = Arc::new(Control::default());
        let run = Loop {
            input: File::from(input),
            decoder,
            escape_timeout,
            signals,
            hooks,
            wake: wake_end,
            decoded: Vec::new(),
            ended: Ended {
                shared,
                control: control.clone(),
            },
        };
        let thread = std::thread::Builder::new()
            .name("tuiframe-reader".into())
            .spawn(move || run.run())?;
        Ok(Self {
            thread: Some(thread),
            control,
            wake,
        })
    }

    /// Tells the decoder of a query whose answer is outstanding, before it decodes what the reader
    /// reads next. Calling it before the query is written is enough.
    pub(crate) fn expect(&self, query: Query) {
        lock(&self.control.state).asks.push(Ask::Expect(query));
    }

    /// Makes a kept sequence wait `timeout` for the rest from now on.
    pub(crate) fn set_escape_timeout(&self, timeout: Duration) {
        lock(&self.control.state).escape_timeout = Some(timeout);
        self.wake();
    }

    /// Tells the decoder to forget the queries whose answers are outstanding, for a probe that
    /// stopped waiting, before it decodes what the reader reads next.
    pub(crate) fn cancel_expected(&self) {
        lock(&self.control.state).asks.push(Ask::Cancel);
    }

    /// Holds the reader still, reading nothing and handling no signal until
    /// [`unpark`](Self::unpark), for while the program has given the terminal back. Returns
    /// whether it held still within a second.
    pub(crate) fn park(&self) -> bool {
        lock(&self.control.state).park = true;
        self.wake();
        self.control
            .wait_until(|requests| requests.parked || requests.ended)
    }

    /// Lets the reader go on.
    pub(crate) fn unpark(&self) {
        lock(&self.control.state).park = false;
        self.control.changed.notify_all();
    }

    /// How many SIGTSTPs the reader handled.
    pub(crate) fn stops(&self) -> u64 {
        lock(&self.control.state).stops
    }

    /// Waits until the reader handled a SIGTSTP after the `since`th, for at most `start` for it
    /// to begin, then as long as it takes, the stop included. Whether one came.
    pub(crate) fn wait_for_stop(&self, since: u64, start: Duration) -> bool {
        let until = Instant::now() + start;
        let mut requests = lock(&self.control.state);
        while requests.stops <= since && !requests.ended {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() && !requests.stopping {
                return false;
            }
            let wait = if requests.stopping {
                CONTROL_WAIT
            } else {
                left
            };
            requests = self
                .control
                .changed
                .wait_timeout(requests, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        requests.stops > since
    }

    /// Ends the reader's thread, waiting a second at most.
    pub(crate) fn stop(mut self) {
        self.end();
    }

    fn end(&mut self) {
        lock(&self.control.state).stop = true;
        self.control.changed.notify_all();
        self.wake();
        let ended = self.control.wait_until(|requests| requests.ended);
        if let Some(thread) = self.thread.take()
            && ended
        {
            let _ = thread.join();
        }
    }

    fn wake(&self) {
        let _ = (&self.wake).write(&[1]);
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.end();
        }
    }
}

/// Closes the queue and says the thread ended, also when it ends by a panic.
struct Ended {
    shared: Arc<Shared>,
    control: Arc<Control>,
}

impl Drop for Ended {
    fn drop(&mut self) {
        self.shared.events.close();
        lock(&self.control.state).ended = true;
        self.control.changed.notify_all();
    }
}

/// The reader's thread.
struct Loop<D> {
    input: File,
    decoder: D,
    escape_timeout: Duration,
    signals: Signals,
    hooks: Hooks,
    wake: PipeReader,
    /// What the decoder found and the reader hasn't routed yet.
    decoded: Vec<Decoded>,
    ended: Ended,
}

impl<D: Decode> Loop<D> {
    fn run(mut self) {
        let mut buffer = vec![0; READ_SIZE];
        let mut events = Vec::new();
        // (When the decoder last kept an incomplete sequence, for the escape timeout.)
        let mut pending_since = None;
        let mut last_read = Instant::now();
        while self.obey() {
            let pending = pending_since
                .map(|since: Instant| self.escape_timeout.saturating_sub(since.elapsed()));
            let held = self
                .decoder
                .has_held()
                .then(|| HELD_WAIT.saturating_sub(last_read.elapsed()));
            let timeout = pending.into_iter().chain(held).min();
            let fds = [
                self.input.as_fd(),
                self.signals.resize.as_fd(),
                self.signals.resume.as_fd(),
                self.signals.stop.as_fd(),
                self.signals.end.as_fd(),
                self.wake.as_fd(),
            ];
            let [input, resize, resume, stop, end, wake] = match wait(fds, timeout) {
                Ok(ready) => ready,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.ended.shared.events.fail(error.kind());
                    break;
                }
            };
            if wake {
                drain(&self.wake);
            }
            if resize {
                drain(&self.signals.resize);
                self.resized(&mut events);
            }
            if end {
                drain(&self.signals.end);
                if let Some(signal) = self.signals.ending() {
                    (self.hooks.end)(signal);
                }
            }
            if stop {
                drain(&self.signals.stop);
                self.stopped(&mut events);
            } else if resume {
                drain(&self.signals.resume);
                (self.hooks.resume)();
                self.ended.shared.request_redraw();
                events.push(Event::Resumed);
            }
            if input {
                match (&self.input).read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        last_read = Instant::now();
                        // (Queries asked for before their answers could have come.)
                        let asks = std::mem::take(&mut lock(&self.ended.control.state).asks);
                        for ask in asks {
                            match ask {
                                Ask::Expect(query) => self.decoder.expect(query),
                                Ask::Cancel => self.decoder.cancel_expected(),
                            }
                        }
                        self.decoder.feed(&buffer[..read], &mut self.decoded);
                        pending_since = None;
                        if self.decoder.has_pending() {
                            if self.escape_timeout.is_zero() && read < buffer.len() {
                                self.decoder.flush_pending(&mut self.decoded);
                            } else {
                                pending_since = Some(Instant::now());
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                        ) => {}
                    // (A terminal that hung up fails reads with EIO.)
                    Err(error) if error.raw_os_error() == Some(Errno::IO.raw_os_error()) => break,
                    Err(error) => {
                        self.ended.shared.events.fail(error.kind());
                        break;
                    }
                }
            } else if !(wake || resize || resume || stop || end) {
                if pending_since.is_some_and(|since| since.elapsed() >= self.escape_timeout) {
                    // (The escape timeout passed with nothing more.)
                    pending_since = None;
                    self.decoder.flush_pending(&mut self.decoded);
                }
                if self.decoder.has_held() && last_read.elapsed() >= HELD_WAIT {
                    self.decoder.flush_held(&mut self.decoded);
                }
            }
            self.route(&mut events);
        }
        self.decoder.flush_pending(&mut self.decoded);
        self.route(&mut events);
    }

    /// Queues the events the decoder found, after `events`, and offers the replies to the reply
    /// slot.
    fn route(&mut self, events: &mut Vec<Event>) {
        let shared = &self.ended.shared;
        for decoded in self.decoded.drain(..) {
            match decoded {
                Decoded::Event(event) => events.push(event),
                Decoded::Reply(reply) => shared.replies.offer(reply),
            }
        }
        shared.events.push(events);
    }

    /// Follows the program's requests. Holds still while it's parked, and returns whether to go
    /// on.
    fn obey(&mut self) -> bool {
        let control = self.ended.control.clone();
        let mut requests = lock(&control.state);
        if let Some(timeout) = requests.escape_timeout.take() {
            self.escape_timeout = timeout;
        }
        if !requests.park {
            return !requests.stop;
        }
        requests.parked = true;
        control.changed.notify_all();
        while requests.park && !requests.stop {
            requests = control
                .changed
                .wait(requests)
                .unwrap_or_else(PoisonError::into_inner);
        }
        requests.parked = false;
        !requests.stop
    }

    /// Handles a SIGTSTP. The hook gives the terminal back, stops the process and takes the
    /// terminal back once it runs again, and then the SIGCONT that ended the stop is taken as
    /// that, with [`Event::Resumed`] queued.
    fn stopped(&mut self, events: &mut Vec<Event>) {
        let control = self.ended.control.clone();
        lock(&control.state).stopping = true;
        control.changed.notify_all();
        (self.hooks.stop)();
        // (SIGCONT's handler may run on another thread a moment after the process runs again.)
        if let Ok([true]) = wait([self.signals.resume.as_fd()], Some(RESUME_WAIT)) {
            drain(&self.signals.resume);
        }
        self.ended.shared.request_redraw();
        events.push(Event::Resumed);
        {
            let mut requests = lock(&control.state);
            requests.stopping = false;
            requests.stops += 1;
        }
        control.changed.notify_all();
    }

    fn resized(&mut self, events: &mut Vec<Event>) {
        if let Ok(size) = window_size(self.input.as_fd()) {
            self.ended.shared.set_size(size.cells);
            self.ended.shared.request_redraw();
            events.push(Event::Resize(size.cells));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::event::{KeyCode, KeyEvent, KeyModifiers};
    use crate::sys::Next;
    use rustix::termios::{LocalModes, Winsize};
    use std::process::{Child, Command, Stdio};

    pub(crate) const LONG: Duration = Duration::from_secs(5);

    /// A pseudo-terminal, whose master side the test reads and writes as the terminal would.
    pub(crate) struct Pty {
        pub(crate) master: File,
        pub(crate) slave: OwnedFd,
        /// What the master read so far.
        pub(crate) shown: Vec<u8>,
    }

    impl Pty {
        pub(crate) fn open(size: Size) -> Self {
            use rustix::fs::{Mode, OFlags};
            use rustix::io::{FdFlags, fcntl_setfd};
            use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
            let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
            fcntl_setfd(&master, FdFlags::CLOEXEC).unwrap();
            grantpt(&master).unwrap();
            unlockpt(&master).unwrap();
            let name = ptsname(&master, Vec::new()).unwrap();
            let slave = rustix::fs::open(
                name.as_c_str(),
                OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap();
            let pty = Self {
                master: File::from(master),
                slave,
                shown: Vec::new(),
            };
            pty.resize(size);
            pty
        }

        pub(crate) fn resize(&self, size: Size) {
            let winsize = Winsize {
                ws_row: size.height,
                ws_col: size.width,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            termios::tcsetwinsize(&self.master, winsize).unwrap();
        }

        /// The terminal, as the library finds it.
        pub(crate) fn tty(&self) -> Tty {
            Tty {
                input: self.slave.try_clone().unwrap(),
                output: self.slave.try_clone().unwrap(),
            }
        }

        /// The slave's modes, all of them, as text.
        pub(crate) fn modes(&self) -> String {
            format!("{:?}", termios::tcgetattr(&self.slave).unwrap())
        }

        /// Whether the slave is in raw mode.
        pub(crate) fn is_raw(&self) -> bool {
            let modes = termios::tcgetattr(&self.slave).unwrap();
            !modes
                .local_modes
                .intersects(LocalModes::ICANON | LocalModes::ECHO | LocalModes::ISIG)
        }

        /// Types `bytes` into the terminal.
        pub(crate) fn type_in(&mut self, bytes: &[u8]) {
            self.master.write_all(bytes).unwrap();
        }

        /// Reads what the program writes until what's shown after `from` holds `needle`, for
        /// `timeout` at most. Returns where `needle` starts.
        pub(crate) fn read_until(
            &mut self,
            from: usize,
            needle: &[u8],
            timeout: Duration,
        ) -> usize {
            let until = Instant::now() + timeout;
            loop {
                if let Some(at) = find(&self.shown[from.min(self.shown.len())..], needle) {
                    return from + at;
                }
                let left = until.saturating_duration_since(Instant::now());
                assert!(
                    !left.is_zero(),
                    "{:?} never came, after {:?}",
                    String::from_utf8_lossy(needle),
                    String::from_utf8_lossy(&self.shown)
                );
                if let Ok([true]) = wait([self.master.as_fd()], Some(left)) {
                    let mut buffer = [0; 4096];
                    match (&self.master).read(&mut buffer) {
                        Ok(read) => self.shown.extend_from_slice(&buffer[..read]),
                        Err(error) => panic!("the terminal failed: {error}"),
                    }
                }
            }
        }
    }

    impl Pty {
        /// Reads what the program wrote that is still to be read, without waiting.
        pub(crate) fn read_ready(&mut self) {
            while let Ok([true]) = wait([self.master.as_fd()], Some(Duration::ZERO)) {
                let mut buffer = [0; 4096];
                match (&self.master).read(&mut buffer) {
                    Ok(read) if read > 0 => self.shown.extend_from_slice(&buffer[..read]),
                    _ => return,
                }
            }
        }
    }

    /// Where `needle` first starts in `haystack`.
    pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// Decodes each byte as the key that types it, for tests, with ESC kept until the next byte
    /// (which comes with Alt) or until the escape timeout makes it Esc.
    #[derive(Debug, Default)]
    pub(crate) struct Bytes {
        escape: bool,
    }

    impl Decode for Bytes {
        fn feed(&mut self, bytes: &[u8], out: &mut Vec<Decoded>) {
            for &byte in bytes {
                if byte == 0x1b && !self.escape {
                    self.escape = true;
                    continue;
                }
                let modifiers = if std::mem::take(&mut self.escape) {
                    KeyModifiers::ALT
                } else {
                    KeyModifiers::NONE
                };
                out.push(Decoded::Event(key(char::from(byte), modifiers)));
            }
        }

        fn has_pending(&self) -> bool {
            self.escape
        }

        fn flush_pending(&mut self, out: &mut Vec<Decoded>) {
            if std::mem::take(&mut self.escape) {
                out.push(Decoded::Event(Event::Key(KeyEvent::new(
                    KeyCode::Esc,
                    KeyModifiers::NONE,
                ))));
            }
        }

        fn expect(&mut self, _: Query) {}

        fn cancel_expected(&mut self) {}

        fn has_held(&self) -> bool {
            self.escape
        }

        fn flush_held(&mut self, out: &mut Vec<Decoded>) {
            self.flush_pending(out);
        }
    }

    pub(crate) fn key(c: char, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), modifiers))
    }

    pub(crate) fn next(shared: &Shared) -> Event {
        match shared.events.next_timeout(LONG) {
            Next::Event(event) => event,
            other => panic!("no event came, but {other:?}"),
        }
    }

    /// A reader on a pseudo-terminal, with pipes that stand in for SIGWINCH and SIGCONT.
    struct Reading {
        pty: Pty,
        shared: Arc<Shared>,
        reader: Reader,
        resize: PipeWriter,
        resume: PipeWriter,
        resumed: Arc<Mutex<usize>>,
    }

    fn reading(escape_timeout: Duration) -> Reading {
        reading_with(Bytes::default(), escape_timeout)
    }

    fn reading_with(decoder: impl Decode, escape_timeout: Duration) -> Reading {
        let pty = Pty::open(Size::new(80, 24));
        let modes = Modes::enter(pty.slave.as_fd()).unwrap();
        let shared = Arc::new(Shared::new(Size::new(80, 24)));
        let (signals, resize, resume) = Signals::unregistered().unwrap();
        let resumed = Arc::new(Mutex::new(0));
        let counted = resumed.clone();
        let reader = Reader::spawn(
            pty.slave.try_clone().unwrap(),
            decoder,
            shared.clone(),
            escape_timeout,
            signals,
            Hooks {
                resume: Box::new(move || {
                    let _ = modes.apply_raw();
                    *lock(&counted) += 1;
                }),
                ..Hooks::none()
            },
        )
        .unwrap();
        Reading {
            pty,
            shared,
            reader,
            resize,
            resume,
            resumed,
        }
    }

    /// With the library's decoder, replies go to the reply slot while it's open and nowhere
    /// otherwise, a cursor report is one only while it's expected, and a sequence a terminal left
    /// unfinished is given up after a second of silence.
    #[test]
    fn the_decoder_routes_replies_and_gives_up_on_what_it_holds() {
        use crate::event::decode::{DecoderOptions, Reply};
        let mut reading = reading_with(Decoder::new(DecoderOptions::default()), Duration::ZERO);
        let ctrl_f3 = Event::Key(KeyEvent::new(KeyCode::F(3), KeyModifiers::CONTROL));
        reading.shared.replies.open();
        reading.reader.expect(Query::CursorPosition);
        reading.pty.type_in(b"\x1b[3;5R\x1b[?62;c");
        assert!(
            reading
                .shared
                .replies
                .wait(LONG, |replies| replies.len() == 2)
        );
        assert_eq!(
            reading.shared.replies.close(),
            [
                Reply::CursorPosition { row: 2, column: 4 },
                Reply::Da1 {
                    attributes: vec![62]
                }
            ]
        );
        // A late reply is dropped, and a cursor report nobody expects is the key it reads as.
        reading.pty.type_in(b"\x1b[?62;c\x1b[1;5R");
        assert_eq!(next(&reading.shared), ctrl_f3);
        reading.reader.expect(Query::CursorPosition);
        reading.reader.cancel_expected();
        reading.pty.type_in(b"\x1b[1;5R");
        assert_eq!(next(&reading.shared), ctrl_f3);
        // A sequence cut short holds nothing back once the input was silent a while.
        reading.pty.type_in(b"\x1b[1;");
        std::thread::sleep(HELD_WAIT + Duration::from_millis(200));
        reading.pty.type_in(b"a");
        assert_eq!(next(&reading.shared), key('a', KeyModifiers::NONE));
    }

    /// Raw mode delivers each key as it's typed, with no echo, and the modes the program found
    /// come back exactly. Backspace is the terminal's erase character.
    #[test]
    fn raw_mode_comes_and_goes_exactly() {
        let pty = Pty::open(Size::new(80, 24));
        let mut modes = termios::tcgetattr(&pty.slave).unwrap();
        modes.special_codes[SpecialCodeIndex::VERASE] = 0x08;
        termios::tcsetattr(&pty.slave, OptionalActions::Now, &modes).unwrap();
        let before = pty.modes();
        assert!(!pty.is_raw());
        let raw = Modes::enter(pty.slave.as_fd()).unwrap();
        assert!(pty.is_raw());
        let applied = termios::tcgetattr(&pty.slave).unwrap();
        assert_eq!(applied.special_codes[SpecialCodeIndex::VMIN], 1);
        assert_eq!(applied.special_codes[SpecialCodeIndex::VTIME], 0);
        assert_eq!(raw.erase(), 0x08);
        raw.restore().unwrap();
        assert_eq!(pty.modes(), before);
        raw.apply_raw().unwrap();
        assert!(pty.is_raw());
        raw.restore().unwrap();
        assert_eq!(pty.modes(), before);

        modes.special_codes[SpecialCodeIndex::VERASE] = 0x7f;
        termios::tcsetattr(&pty.slave, OptionalActions::Now, &modes).unwrap();
        let raw = Modes::enter(pty.slave.as_fd()).unwrap();
        assert_eq!(raw.erase(), 0x7f);
        raw.restore().unwrap();
    }

    #[test]
    fn the_size_comes_from_the_terminal() {
        let pty = Pty::open(Size::new(132, 43));
        let size = window_size(pty.slave.as_fd()).unwrap();
        assert_eq!(size.cells, Size::new(132, 43));
        assert!(Modes::enter(io::pipe().unwrap().0.as_fd()).is_err());
    }

    /// Keys typed come out as events, in order, and a sequence the decoder keeps decodes at once
    /// when no more input is ready and the escape timeout is zero.
    #[test]
    fn keys_come_out_and_a_lone_escape_decodes_at_once() {
        let mut reading = reading(Duration::ZERO);
        reading.pty.type_in(b"ab");
        assert_eq!(next(&reading.shared), key('a', KeyModifiers::NONE));
        assert_eq!(next(&reading.shared), key('b', KeyModifiers::NONE));
        reading.pty.type_in(b"\x1b");
        assert_eq!(
            next(&reading.shared),
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        );
        reading.pty.type_in(b"\x1bc");
        assert_eq!(next(&reading.shared), key('c', KeyModifiers::ALT));
        reading.reader.stop();
        assert_eq!(reading.shared.events.next_timeout(LONG), Next::Closed);
    }

    /// With an escape timeout, a kept sequence waits for the rest, and decodes alone once the
    /// timeout passes with nothing more.
    #[test]
    fn a_kept_sequence_waits_for_the_escape_timeout() {
        let timeout = Duration::from_millis(200);
        let mut reading = reading(timeout);
        reading.pty.type_in(b"\x1b");
        std::thread::sleep(Duration::from_millis(50));
        reading.pty.type_in(b"d");
        assert_eq!(next(&reading.shared), key('d', KeyModifiers::ALT));
        let typed = Instant::now();
        reading.pty.type_in(b"\x1b");
        assert_eq!(
            next(&reading.shared),
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        );
        assert!(typed.elapsed() >= timeout);
    }

    /// A resize reads the new size into the atomic, queues it and asks for a full redraw. A
    /// resume puts raw mode back and queues `Resumed`.
    #[test]
    fn signals_give_resize_and_resumed() {
        let mut reading = reading(Duration::ZERO);
        assert!(reading.shared.take_redraw());
        reading.pty.resize(Size::new(100, 30));
        reading.resize.write_all(b"X").unwrap();
        assert_eq!(next(&reading.shared), Event::Resize(Size::new(100, 30)));
        assert_eq!(reading.shared.size(), Size::new(100, 30));
        assert!(reading.shared.take_redraw());

        // (A shell that had the terminal meanwhile left it in its own modes.)
        let cooked = termios::tcgetattr(&reading.pty.slave).unwrap();
        let mut shell = cooked.clone();
        shell.local_modes |= LocalModes::ICANON | LocalModes::ECHO;
        termios::tcsetattr(&reading.pty.slave, OptionalActions::Now, &shell).unwrap();
        reading.resume.write_all(b"X").unwrap();
        assert_eq!(next(&reading.shared), Event::Resumed);
        assert!(reading.pty.is_raw());
        assert_eq!(*lock(&reading.resumed), 1);
        assert!(reading.shared.take_redraw());
    }

    /// A parked reader reads nothing and handles no signal until it goes on.
    #[test]
    fn a_parked_reader_holds_still() {
        let mut reading = reading(Duration::ZERO);
        assert!(reading.reader.park());
        reading.pty.type_in(b"e");
        reading.resume.write_all(b"X").unwrap();
        assert_eq!(
            reading
                .shared
                .events
                .next_timeout(Duration::from_millis(100)),
            Next::Empty
        );
        assert_eq!(*lock(&reading.resumed), 0);
        reading.reader.unpark();
        assert_eq!(next(&reading.shared), Event::Resumed);
        assert_eq!(next(&reading.shared), key('e', KeyModifiers::NONE));
        assert_eq!(*lock(&reading.resumed), 1);
        assert_eq!(
            reading
                .shared
                .events
                .next_timeout(Duration::from_millis(100)),
            Next::Empty
        );
    }

    /// The queue closes when the terminal hangs up.
    #[test]
    fn a_terminal_that_closes_ends_the_events() {
        let reading = reading(Duration::ZERO);
        let Reading {
            pty,
            shared,
            reader,
            ..
        } = reading;
        let Pty { master, slave, .. } = pty;
        drop((master, slave));
        assert_eq!(shared.events.next_timeout(LONG), Next::Closed);
        reader.stop();
    }

    /// `wait` says which descriptors are ready, and gives up at its timeout.
    #[test]
    fn waiting_says_which_are_ready() {
        let (first, mut first_end) = io::pipe().unwrap();
        let (second, second_end) = io::pipe().unwrap();
        let started = Instant::now();
        let ready = wait(
            [first.as_fd(), second.as_fd()],
            Some(Duration::from_millis(50)),
        );
        assert_eq!(ready.unwrap(), [false, false]);
        assert!(started.elapsed() >= Duration::from_millis(50));
        first_end.write_all(b"x").unwrap();
        assert_eq!(
            wait([first.as_fd(), second.as_fd()], None).unwrap(),
            [true, false]
        );
        // A pipe whose writer closed counts as ready, as a terminal that hung up does. (A child
        // another test forks holds a copy of every descriptor until it execs, so the hang-up may
        // come a moment later.)
        drop(second_end);
        assert_eq!(wait([second.as_fd()], Some(LONG)).unwrap(), [true]);
    }

    /// The variable that makes a test's process the child that runs its case.
    const CHILD: &str = "TUIFRAME_TEST_CHILD";

    /// Whether this process is the child that runs the case `case`.
    pub(crate) fn is_child(case: &str) -> bool {
        std::env::var(CHILD).is_ok_and(|running| running == case)
    }

    /// Runs this test binary again as a child that runs only the test `test` of `module` (its
    /// `module_path!()`), as the case `case`, with its standard input, output and error on the
    /// terminal `pty`. It's in a process group of its own, so a stop stops it alone.
    pub(crate) fn spawn_child(module: &str, test: &str, case: &str, pty: &Pty) -> Child {
        use std::os::unix::process::CommandExt;
        let module = module.split_once("::").map_or("", |(_, rest)| rest);
        let slave = || Stdio::from(pty.slave.try_clone().unwrap());
        Command::new(std::env::current_exe().unwrap())
            .args([&format!("{module}::{test}"), "--exact", "--nocapture"])
            .args(["--test-threads", "1"])
            .env(CHILD, case)
            .stdin(slave())
            .stdout(slave())
            .stderr(slave())
            .process_group(0)
            .spawn()
            .unwrap()
    }

    /// SIGWINCH reaches the reader through its pipe, which gives the new size.
    #[test]
    fn sigwinch_gives_the_new_size() {
        if is_child("resize") {
            let tty = Tty::open().unwrap();
            let modes = Modes::enter(tty.input.as_fd()).unwrap();
            let shared = Arc::new(Shared::new(Size::new(80, 24)));
            let reader = Reader::spawn(
                tty.input,
                Bytes::default(),
                shared.clone(),
                Duration::ZERO,
                Signals::register(false).unwrap(),
                Hooks::none(),
            )
            .unwrap();
            (&File::from(tty.output)).write_all(b"<ready>").unwrap();
            assert_eq!(next(&shared), Event::Resize(Size::new(100, 30)));
            assert_eq!(shared.size(), Size::new(100, 30));
            reader.stop();
            modes.restore().unwrap();
            return;
        }
        let mut pty = Pty::open(Size::new(80, 24));
        let mut child = spawn_child(
            module_path!(),
            "sigwinch_gives_the_new_size",
            "resize",
            &pty,
        );
        pty.read_until(0, b"<ready>", LONG);
        pty.resize(Size::new(100, 30));
        let pid = rustix::process::Pid::from_child(&child);
        rustix::process::kill_process(pid, rustix::process::Signal::WINCH).unwrap();
        let status = child.wait().unwrap();
        pty.read_ready();
        assert!(
            status.success(),
            "{status}: {}",
            String::from_utf8_lossy(&pty.shown)
        );
    }
}
