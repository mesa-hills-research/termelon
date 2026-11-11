//! Taking the terminal and giving it back, once, in every way a program can end.
//!
//! The packet that takes the terminal and the reset that gives it back are built from one
//! [`Enabled`], so the reset undoes exactly what was done. A [`RestoreHandle`] gives the terminal
//! back from any thread, and [`install_panic_hook`] does it before a panic's message prints. On
//! Unix, [`Screen`](screen::Screen) holds the terminal while a program has it, and suspends it.

use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::ThreadId;
use std::time::Duration;

use crate::output::writer::{
    self, Direct, Finish, LOCK_WAIT, Takeover, Writer, lock, try_lock_for,
};
use crate::terminal::MouseMode;

/// How long giving the terminal back waits for the terminal to take the reset, before what comes
/// next (a panic's message, the end of the process).
pub(crate) const CLOSE_WAIT: Duration = Duration::from_secs(1);

/// What a program turns on while it has the terminal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Enabled {
    /// The alternate screen, mode 1049.
    pub(crate) alternate_screen: bool,
    /// The screen is cleared when the program takes it and when it gives it back, where there's no
    /// alternate screen to leave it untouched.
    pub(crate) clear: bool,
    pub(crate) mouse: MouseMode,
    /// Focus reports, mode 1004.
    pub(crate) focus: bool,
    /// Bracketed paste, mode 2004.
    pub(crate) bracketed_paste: bool,
    /// Grapheme clusters as single characters, mode 2027.
    pub(crate) grapheme_clusters: bool,
    /// Reports of the terminal switching between dark and light, mode 2031.
    pub(crate) theme_reports: bool,
    /// The kitty keyboard flags pushed, if any were.
    pub(crate) keyboard: Option<u8>,
    /// xterm's modifyOtherKeys at level 1.
    pub(crate) modify_other_keys: bool,
    /// The window's title saved on xterm's title stack, to be put back from it.
    pub(crate) save_title: bool,
    /// Frames may hold hyperlinks, so the reset ends one left open.
    pub(crate) hyperlinks: bool,
    /// Frames may change the cursor's shape, so the reset puts the default back.
    pub(crate) cursor_shape: bool,
}

/// The private modes `mouse` turns on, in order.
fn mouse_modes(mouse: MouseMode) -> &'static [u16] {
    match mouse {
        MouseMode::Off => &[],
        MouseMode::ButtonsAndDrags => &[1000, 1002, 1006],
        MouseMode::AnyMotion => &[1000, 1002, 1003, 1006],
    }
}

/// What turns the mouse reports from `from` to `to`.
pub(crate) fn mouse_switch(from: MouseMode, to: MouseMode) -> Vec<u8> {
    let mut out = Vec::new();
    if from != to {
        for &mode in mouse_modes(from).iter().rev() {
            private_mode(&mut out, mode, false);
        }
        for &mode in mouse_modes(to) {
            private_mode(&mut out, mode, true);
        }
    }
    out
}

impl Enabled {
    /// The private modes it turns on after the alternate screen, in order.
    fn modes(&self) -> Vec<u16> {
        let mut modes = mouse_modes(self.mouse).to_vec();
        let switches = [
            (self.focus, 1004),
            (self.bracketed_paste, 2004),
            (self.grapheme_clusters, 2027),
            (self.theme_reports, 2031),
        ];
        modes.extend(switches.iter().filter(|(on, _)| *on).map(|(_, mode)| mode));
        modes
    }

    /// What takes the terminal. It saves the title, enters the alternate screen, hides the cursor
    /// until a frame shows it, turns autowrap off so the last column never scrolls, then the
    /// reports and the keyboard flags.
    pub(crate) fn setup(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.save_title {
            out.extend_from_slice(b"\x1b[22;0t");
        }
        if self.alternate_screen {
            out.extend_from_slice(b"\x1b[?1049h");
        }
        out.extend_from_slice(b"\x1b[?25l\x1b[?7l");
        for mode in self.modes() {
            private_mode(&mut out, mode, true);
        }
        if let Some(flags) = self.keyboard {
            out.extend_from_slice(format!("\x1b[>{flags}u").as_bytes());
        }
        if self.modify_other_keys {
            out.extend_from_slice(b"\x1b[>4;1m");
        }
        if self.clear {
            out.extend_from_slice(b"\x1b[H\x1b[2J");
        }
        out
    }

    /// What takes the terminal after `done` took it, which is [`setup`](Self::setup) without
    /// entering the alternate screen again, since entering it twice loses the main screen's
    /// cursor.
    pub(crate) fn setup_after(&self, done: &Enabled) -> Vec<u8> {
        let mut rest = self.clone();
        if done.alternate_screen {
            rest.alternate_screen = false;
        }
        rest.setup()
    }

    /// What gives the terminal back. It ends a synchronized update a frame left open, pops the
    /// keyboard flags, turns the reports off, resets the pen and any open hyperlink, turns
    /// autowrap back on, leaves the alternate screen, shows the cursor with its default shape and
    /// puts the title back.
    pub(crate) fn reset(&self) -> Vec<u8> {
        let mut out = b"\x1b[?2026l".to_vec();
        if self.keyboard.is_some() {
            out.extend_from_slice(b"\x1b[<1u");
        }
        if self.modify_other_keys {
            out.extend_from_slice(b"\x1b[>4;0m");
        }
        for mode in self.modes().into_iter().rev() {
            private_mode(&mut out, mode, false);
        }
        out.extend_from_slice(b"\x1b[0m");
        if self.hyperlinks {
            out.extend_from_slice(b"\x1b]8;;\x1b\\");
        }
        // (Autowrap back on, then the whole screen as the scroll region, before leaving.)
        out.extend_from_slice(b"\x1b[?7h\x1b[r");
        if self.alternate_screen {
            out.extend_from_slice(b"\x1b[?1049l");
        }
        if self.cursor_shape {
            out.extend_from_slice(b"\x1b[0 q");
        }
        if self.clear {
            out.extend_from_slice(b"\x1b[H\x1b[2J");
        }
        out.extend_from_slice(b"\x1b[?25h");
        if self.save_title {
            out.extend_from_slice(b"\x1b[23;0t");
        }
        out
    }
}

/// Appends `CSI ? mode h`, or `l` when `on` is false.
fn private_mode(out: &mut Vec<u8>, mode: u16, on: bool) {
    let end = if on { 'h' } else { 'l' };
    out.extend_from_slice(format!("\x1b[?{mode}{end}").as_bytes());
}

/// Opens what a reset written without the writer goes to.
pub(crate) type OpenOutput = Arc<dyn Fn() -> Box<dyn Write + Send> + Send + Sync>;

/// Puts the terminal's modes back as the program found them.
pub(crate) type RestoreModes = Box<dyn Fn() + Send + Sync>;

/// Gives the terminal back from any thread, for panic hooks and threads that handle signals.
///
/// Clones share one record, so the reset is written once however many of them give the terminal
/// back, and from however many threads. It takes no lock that the writer holds while it writes, so
/// it works while the terminal has stopped reading. Inside a signal handler only writing to a pipe
/// is safe, so call it from a thread the handler wakes.
#[derive(Clone)]
pub struct RestoreHandle {
    inner: Arc<Inner>,
}

struct Inner {
    takeover: Takeover,
    out: OpenOutput,
    modes: RestoreModes,
    /// The modes went back as the program found them, so they're no longer the library's to
    /// change.
    given_back: AtomicBool,
    /// The thread that draws, whose panic gives the terminal back.
    drawing: Mutex<Option<ThreadId>>,
}

impl fmt::Debug for RestoreHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RestoreHandle")
            .field("given_back", &self.is_given_back())
            .finish_non_exhaustive()
    }
}

impl RestoreHandle {
    /// A handle on the terminal that `writer` writes to. A reset written without the writer goes
    /// to what `out` opens, and `modes` puts the terminal's modes back. The thread that calls this
    /// draws until [`draw_here`](Self::draw_here) says otherwise.
    pub(crate) fn new(writer: &Writer, out: OpenOutput, modes: RestoreModes) -> Self {
        Self {
            inner: Arc::new(Inner {
                takeover: writer.takeover(),
                out,
                modes,
                given_back: AtomicBool::new(false),
                drawing: Mutex::new(Some(std::thread::current().id())),
            }),
        }
    }

    /// Gives the terminal back now, without waiting for the writer, for a program about to end.
    ///
    /// It writes CAN and ST, which end an escape sequence or a string that a write left half done,
    /// then the reset, from a thread of its own, waits at most `timeout` for them, and puts the terminal's
    /// modes back as the program found them. Returns whether the reset reached the terminal, from
    /// this call or an earlier one. Once the terminal was given back, it writes nothing more.
    pub fn restore_now(&self, timeout: Duration) -> bool {
        let written = self.reset_directly(timeout);
        self.restore_modes();
        written
    }

    /// Gives the terminal back through `writer`, as closing does, with `tail` after the reset.
    /// Waits at most `wait` for the reset, writes it without the writer when the writer failed,
    /// then puts the modes back. Returns whether the reset reached the terminal.
    pub(crate) fn give_back(&self, writer: &Writer, tail: &[u8], wait: Duration) -> bool {
        let (ticket, _) = writer.close(tail);
        let written = match writer.wait(ticket, wait) {
            Finish::Written => true,
            Finish::Failed => self.reset_directly(wait),
            Finish::TimedOut => false,
        };
        self.restore_modes();
        written
    }

    /// How long the terminal has kept the write going on waiting, as
    /// [`Terminal::writer_waiting`](crate::Terminal::writer_waiting) gives it, for a watchdog on a
    /// thread of its own.
    pub fn writer_waiting(&self) -> Option<Duration> {
        self.inner
            .takeover
            .writer()?
            .busy_since()
            .map(|since| since.elapsed())
    }

    /// Whether the terminal was given back, so its modes are the program's own again.
    pub(crate) fn is_given_back(&self) -> bool {
        self.inner.given_back.load(Ordering::SeqCst)
    }

    /// Makes this thread the one that draws, whose panic gives the terminal back.
    pub(crate) fn draw_here(&self) {
        *lock(&self.inner.drawing) = Some(std::thread::current().id());
    }

    fn reset_directly(&self, wait: Duration) -> bool {
        let out = self.inner.out.clone();
        let direct =
            self.inner
                .takeover
                .reset_directly(wait, Box::new(move || out()), &mut |run| {
                    writer::spawn_named("termelon-reset", run)
                });
        matches!(direct, Direct::Unneeded | Direct::Written)
    }

    fn restore_modes(&self) {
        if !self.inner.given_back.swap(true, Ordering::SeqCst) {
            (self.inner.modes)();
        }
    }
}

/// The terminal a program has, for the panic hook. Locked only to take or put a handle, never
/// together with the writer's lock.
static CURRENT: Mutex<Option<RestoreHandle>> = Mutex::new(None);

/// Makes `handle` the one a panic gives the terminal back with.
pub(crate) fn register(handle: &RestoreHandle) {
    *lock(&CURRENT) = Some(handle.clone());
}

/// Forgets `handle`, if it's the one a panic gives the terminal back with.
pub(crate) fn unregister(handle: &RestoreHandle) {
    let mut current = lock(&CURRENT);
    if current
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(&current.inner, &handle.inner))
    {
        *current = None;
    }
}

/// Whether the current thread is one of the library's writer threads, which write to the terminal.
///
/// A program with a panic hook of its own uses it to tell a panic on the writer's thread, which
/// can't give the terminal back through the writer, from one elsewhere, and calls
/// [`RestoreHandle::restore_now`] there, which writes the reset without the writer.
pub fn is_writer_thread() -> bool {
    writer::on_writer_thread()
}

/// Makes a panic give the terminal back before its message prints, so the message shows on the
/// main screen, in the modes the program found.
///
/// It chains the hook already set. A panic on the main thread or on the thread that draws gives the
/// terminal back through the writer, waiting at most a second for it, and one on the writer's own
/// thread writes the reset without it. A panic on any other thread leaves the terminal to the
/// program. Calling it again does nothing.
///
/// A program with rules of its own for which panics end it builds its hook on [`RestoreHandle`]
/// instead.
pub fn install_panic_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            give_back_on_panic();
            previous(info);
        }));
    });
}

/// What a panic ends, and so how it gives the terminal back.
#[derive(Debug, PartialEq, Eq)]
enum Ends {
    /// The program, on the thread that draws or the main thread, whose panic ends the process.
    Program,
    /// The writer, which can't give the terminal back itself.
    Writer,
    /// Nothing the terminal needs.
    Nothing,
}

/// What a panic on `thread`, named `name`, ends, where `writer` writes and `drawing` draws.
fn panic_ends(
    thread: ThreadId,
    name: Option<&str>,
    writer: Option<ThreadId>,
    drawing: Option<ThreadId>,
) -> Ends {
    if writer == Some(thread) {
        Ends::Writer
    } else if drawing == Some(thread) || name == Some("main") {
        Ends::Program
    } else {
        Ends::Nothing
    }
}

fn give_back_on_panic() {
    let Some(current) = try_lock_for(&CURRENT, LOCK_WAIT).and_then(|current| current.clone())
    else {
        return;
    };
    if current.is_given_back() {
        return;
    }
    let thread = std::thread::current();
    let writer = current.inner.takeover.writer();
    let drawing = try_lock_for(&current.inner.drawing, LOCK_WAIT).and_then(|drawing| *drawing);
    match panic_ends(
        thread.id(),
        thread.name(),
        writer.as_ref().and_then(Writer::thread),
        drawing,
    ) {
        Ends::Program => match writer {
            Some(writer) => {
                current.give_back(&writer, &[], CLOSE_WAIT);
            }
            None => {
                current.restore_now(CLOSE_WAIT);
            }
        },
        Ends::Writer => {
            current.restore_now(CLOSE_WAIT);
        }
        Ends::Nothing => {}
    }
}

/// The terminal while a program has it, on Unix.
#[cfg(unix)]
pub(crate) mod screen {
    use std::ffi::c_int;
    use std::fs::File;
    use std::io::{self, Write};
    use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{CLOSE_WAIT, Enabled, RestoreHandle, register, unregister};
    use crate::event::decode::Query;
    use crate::layout::Size;
    use crate::output::writer::{Writer, lock};
    use crate::sys::Shared;
    use crate::sys::unix::{
        Decode, Hooks, Modes, Reader, Signals, Tty, stop_group, stop_now, window_size,
    };

    /// How long a suspend waits for the reader to take the SIGTSTP it sent. A process whose
    /// group is orphaned never gets it, and the suspend then returns.
    const STOP_START: Duration = Duration::from_millis(250);

    /// The terminal while a program has it, with its writer, its reader and its modes.
    #[derive(Debug)]
    pub(crate) struct Screen {
        session: Arc<Session>,
        reader: Option<Reader>,
        /// The terminal, for its size.
        size_fd: OwnedFd,
        closed: bool,
    }

    /// What giving the terminal back and taking it back need, shared with the reader's hooks.
    #[derive(Debug)]
    struct Session {
        writer: Writer,
        shared: Arc<Shared>,
        restore: RestoreHandle,
        modes: Arc<Modes>,
        /// What takes the terminal, sent again when the program runs again after a stop.
        setup: Mutex<Vec<u8>>,
    }

    impl Session {
        /// Gives the terminal back until [`take_back`](Self::take_back), with the program's output
        /// still open.
        fn give_back_for_now(&self) -> io::Result<()> {
            self.writer.send(self.writer.reset().to_vec())?;
            let _ = self.writer.wait(self.writer.ticket(), CLOSE_WAIT);
            self.modes.restore()
        }

        /// Takes the terminal back, after something else had it, and makes the next frame a full
        /// redraw. With `reset`, the reset goes first, for a terminal something else stopped,
        /// which still has what the program turned on, so pushing it again doesn't grow the title
        /// and keyboard stacks.
        fn take_back(&self, reset: bool) -> io::Result<()> {
            self.modes.apply_raw()?;
            let mut packet = if reset {
                self.writer.reset().to_vec()
            } else {
                Vec::new()
            };
            packet.extend_from_slice(&lock(&self.setup));
            self.writer.send(packet)?;
            self.shared.request_redraw();
            Ok(())
        }

        /// What the reader does on the signals.
        fn hooks(session: &Arc<Self>) -> Hooks {
            let (resumed, stopped, ended) = (session.clone(), session.clone(), session.clone());
            Hooks {
                resume: Box::new(move || {
                    if !resumed.restore.is_given_back() {
                        let _ = resumed.take_back(true);
                    }
                }),
                stop: Box::new(move || {
                    if stopped.restore.is_given_back() {
                        let _ = stop_now();
                        return;
                    }
                    let given_back = stopped.give_back_for_now();
                    let _ = stop_now();
                    if given_back.is_ok() {
                        let _ = stopped.take_back(false);
                    }
                }),
                end: Box::new(move |signal: c_int| {
                    ended.restore.restore_now(CLOSE_WAIT);
                    let _ = signal_hook::low_level::emulate_default_handler(signal);
                }),
            }
        }
    }

    /// The size of the terminal `fd` is on, unless it says 0x0.
    fn cells(fd: BorrowedFd<'_>) -> Option<Size> {
        window_size(fd)
            .ok()
            .map(|size| size.cells)
            .filter(|size| size.width > 0 && size.height > 0)
    }

    /// The terminal's output for a reset written without the writer, a descriptor kept from open,
    /// so nothing needs opening then.
    struct Output(Arc<File>);

    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            (&*self.0).write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&*self.0).flush()
        }
    }

    impl Screen {
        /// Takes the terminal `tty`. It puts it in raw mode, starts the writer, then starts the
        /// reader, whose decoder `decoder` makes from the byte Backspace sends, and which learns of
        /// signals through `signals`. A panic gives the terminal back from now on, where
        /// [`install_panic_hook`](super::install_panic_hook) was called, and a writer that fails
        /// ends the events. [`enable`](Self::enable) then turns on what the program uses. When it
        /// fails, the terminal is as it was.
        pub(crate) fn open_with<D: Decode>(
            tty: Tty,
            decoder: impl FnOnce(u8) -> D,
            escape_timeout: Duration,
            signals: Signals,
        ) -> io::Result<Self> {
            let Tty { input, output } = tty;
            let modes = Arc::new(Modes::enter(input.as_fd())?);
            // (The output is the screen, unless it isn't a terminal.)
            let size_fd = if rustix::termios::isatty(&output) {
                output.try_clone()
            } else {
                input.try_clone()
            };
            let size_fd = match size_fd {
                Ok(fd) => fd,
                Err(error) => {
                    let _ = modes.restore();
                    return Err(error);
                }
            };
            let size = cells(size_fd.as_fd()).unwrap_or(Size::ZERO);
            let shared = Arc::new(Shared::new(size));
            let started = output.try_clone().and_then(|direct| {
                let writer = Writer::spawn(File::from(output), Enabled::default().reset())?;
                Ok((writer, Arc::new(File::from(direct))))
            });
            let (writer, direct) = match started {
                Ok(started) => started,
                Err(error) => {
                    let _ = modes.restore();
                    return Err(error);
                }
            };
            let restore = RestoreHandle::new(
                &writer,
                Arc::new(move || Box::new(Output(direct.clone()))),
                Box::new({
                    let modes = modes.clone();
                    move || {
                        let _ = modes.restore();
                    }
                }),
            );
            register(&restore);
            writer.on_failure({
                let shared = shared.clone();
                move || shared.events.close()
            });
            let session = Arc::new(Session {
                writer,
                shared,
                restore,
                modes,
                setup: Mutex::new(Vec::new()),
            });
            let decoder = decoder(session.modes.erase());
            let reader = Reader::spawn(
                input,
                decoder,
                session.shared.clone(),
                escape_timeout,
                signals,
                Session::hooks(&session),
            );
            match reader {
                Ok(reader) => Ok(Self {
                    session,
                    reader: Some(reader),
                    size_fd,
                    closed: false,
                }),
                Err(error) => {
                    session.restore.give_back(&session.writer, &[], CLOSE_WAIT);
                    unregister(&session.restore);
                    Err(error)
                }
            }
        }

        /// The terminal's size as it says now, or `None` when it doesn't say or says 0x0.
        pub(crate) fn window_size(&self) -> Option<Size> {
            cells(self.size_fd.as_fd())
        }

        pub(crate) fn writer(&self) -> &Writer {
            &self.session.writer
        }

        pub(crate) fn shared(&self) -> &Arc<Shared> {
            &self.session.shared
        }

        pub(crate) fn restore_handle(&self) -> RestoreHandle {
            self.session.restore.clone()
        }

        /// Makes this thread the one that draws, whose panic gives the terminal back.
        pub(crate) fn draw_here(&self) {
            self.session.restore.draw_here();
        }

        /// Tells the decoder of a query whose answer is outstanding, before the query is written.
        pub(crate) fn expect(&self, query: Query) {
            if let Some(reader) = &self.reader {
                reader.expect(query);
            }
        }

        /// Makes a lone ESC wait `timeout` for a following byte from now on.
        pub(crate) fn set_escape_timeout(&self, timeout: Duration) {
            if let Some(reader) = &self.reader {
                reader.set_escape_timeout(timeout);
            }
        }

        /// Tells the decoder to forget the queries whose answers are outstanding.
        pub(crate) fn cancel_expected(&self) {
            if let Some(reader) = &self.reader {
                reader.cancel_expected();
            }
        }

        /// Turns on what `enabled` says, which the reset then undoes and a resume turns on again.
        /// Fails once the writer failed.
        pub(crate) fn enable(&self, enabled: &Enabled) -> io::Result<()> {
            self.set_enabled(enabled);
            self.session.writer.send(enabled.setup())?;
            Ok(())
        }

        /// Makes `enabled` what the reset undoes and what a resume turns on again. The program
        /// sends what turns the change on.
        pub(crate) fn set_enabled(&self, enabled: &Enabled) {
            self.session.writer.set_reset(enabled.reset());
            *lock(&self.session.setup) = enabled.setup();
        }

        /// Suspends the program as Ctrl+Z does in a shell. It sends SIGTSTP to the process's
        /// group, and the reader takes it as it takes one from outside: it gives the terminal
        /// back, stops the process, and when the process runs again takes the terminal back and
        /// queues [`Event::Resumed`], and the next frame redraws everything. This returns once
        /// that's done, or at once when the stop never came, as in a group the shell no longer
        /// leads. Fails when the signal couldn't be sent.
        ///
        /// [`Event::Resumed`]: crate::event::Event::Resumed
        pub(crate) fn suspend(&self) -> io::Result<()> {
            let Some(reader) = &self.reader else {
                return Ok(());
            };
            let since = reader.stops();
            stop_group()?;
            reader.wait_for_stop(since, STOP_START);
            Ok(())
        }

        /// Gives the terminal back as closing does, without ending the program's output, and holds
        /// the reader still, for running another program on the terminal, such as an editor, until
        /// [`reacquire`](Self::reacquire).
        pub(crate) fn release(&self) -> io::Result<()> {
            if let Some(reader) = &self.reader {
                reader.park();
            }
            self.session.give_back_for_now()
        }

        /// Takes the terminal back after [`release`](Self::release), and makes the next frame a
        /// full redraw.
        pub(crate) fn reacquire(&self) -> io::Result<()> {
            let taken_back = self.session.take_back(false);
            if let Some(reader) = &self.reader {
                reader.unpark();
            }
            taken_back
        }

        /// Gives the terminal back, with `tail` after the reset, waiting at most `wait` for it.
        /// Returns whether the reset reached the terminal.
        pub(crate) fn close(mut self, tail: &[u8], wait: Duration) -> bool {
            self.finish(tail, wait)
        }

        fn finish(&mut self, tail: &[u8], wait: Duration) -> bool {
            self.closed = true;
            if let Some(reader) = self.reader.take() {
                reader.stop();
            }
            let session = &self.session;
            let written = session.restore.give_back(&session.writer, tail, wait);
            unregister(&session.restore);
            written
        }
    }

    impl Drop for Screen {
        fn drop(&mut self) {
            if !self.closed {
                self.finish(&[], CLOSE_WAIT);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::writer::tests::{Breaks, Gate, until_writing};
    use std::io;

    const LONG: Duration = Duration::from_secs(5);

    /// Everything on.
    fn everything() -> Enabled {
        Enabled {
            alternate_screen: true,
            clear: false,
            mouse: MouseMode::ButtonsAndDrags,
            focus: true,
            bracketed_paste: true,
            grapheme_clusters: true,
            theme_reports: true,
            keyboard: Some(1),
            modify_other_keys: true,
            save_title: true,
            hyperlinks: true,
            cursor_shape: true,
        }
    }

    /// The reset undoes, in the reverse order, what the setup did, and only that.
    #[test]
    fn the_reset_undoes_what_the_setup_did() {
        let enabled = everything();
        assert_eq!(
            String::from_utf8(enabled.setup()).unwrap(),
            "\x1b[22;0t\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1004h\
             \x1b[?2004h\x1b[?2027h\x1b[?2031h\x1b[>1u\x1b[>4;1m"
        );
        assert_eq!(
            String::from_utf8(enabled.reset()).unwrap(),
            "\x1b[?2026l\x1b[<1u\x1b[>4;0m\x1b[?2031l\x1b[?2027l\x1b[?2004l\x1b[?1004l\x1b[?1006l\
             \x1b[?1002l\x1b[?1000l\x1b[0m\x1b]8;;\x1b\\\x1b[?7h\x1b[r\x1b[?1049l\x1b[0 q\x1b[?25h\
             \x1b[23;0t"
        );
        let nothing = Enabled::default();
        assert_eq!(nothing.setup(), b"\x1b[?25l\x1b[?7l");
        assert_eq!(
            nothing.reset(),
            b"\x1b[?2026l\x1b[0m\x1b[?7h\x1b[r\x1b[?25h"
        );
        let motion = Enabled {
            mouse: MouseMode::AnyMotion,
            ..Enabled::default()
        };
        assert!(
            String::from_utf8(motion.setup())
                .unwrap()
                .ends_with("\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h")
        );
        // Without an alternate screen, the screen is cleared on the way in and out.
        let console = Enabled {
            clear: true,
            ..Enabled::default()
        };
        assert_eq!(console.setup(), b"\x1b[?25l\x1b[?7l\x1b[H\x1b[2J");
        assert_eq!(
            console.reset(),
            b"\x1b[?2026l\x1b[0m\x1b[?7h\x1b[r\x1b[H\x1b[2J\x1b[?25h"
        );
        assert_eq!(
            mouse_switch(MouseMode::ButtonsAndDrags, MouseMode::AnyMotion),
            b"\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h"
        );
        assert_eq!(mouse_switch(MouseMode::Off, MouseMode::Off), b"");
    }

    /// A handle on a writer to `terminal`, whose resets written without the writer go to
    /// `direct`, and how many times it put the modes back.
    fn handle(writer: &Writer, direct: &Gate) -> (RestoreHandle, Arc<Mutex<usize>>) {
        let restored = Arc::new(Mutex::new(0));
        let direct = direct.clone();
        let counted = restored.clone();
        let handle = RestoreHandle::new(
            writer,
            Arc::new(move || Box::new(direct.clone())),
            Box::new(move || *lock(&counted) += 1),
        );
        (handle, restored)
    }

    /// A terminal that stopped reading gets CAN and the reset from a thread of its own, once,
    /// however many times and from however many threads it's asked.
    #[test]
    fn restore_now_writes_the_reset_once() {
        let stuck = Gate::default();
        let writer = Writer::spawn(stuck.clone(), b"[reset]".to_vec()).unwrap();
        stuck.hold(true);
        writer.send(b"[frame]".to_vec()).unwrap();
        until_writing(&writer);
        let direct = Gate::default();
        let (handle, restored) = handle(&writer, &direct);
        // (A watchdog sees how long the stuck write has waited.)
        std::thread::sleep(Duration::from_millis(20));
        assert!(handle.writer_waiting() >= Some(Duration::from_millis(20)));
        let threads: Vec<_> = (0..3)
            .map(|_| {
                let handle = handle.clone();
                std::thread::spawn(move || handle.restore_now(LONG))
            })
            .collect();
        for thread in threads {
            assert!(thread.join().unwrap());
        }
        assert!(handle.restore_now(LONG));
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");
        assert_eq!(*lock(&restored), 1);
        assert!(handle.is_given_back());
        // The writer writes nothing more.
        writer.send(b"[another]".to_vec()).unwrap();
        stuck.hold(false);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(stuck.taken(), b"[frame]");
    }

    /// Once the writer gave the terminal back, restoring it again writes nothing, and puts the
    /// modes back no second time.
    #[test]
    fn a_terminal_given_back_is_given_back_once() {
        let terminal = Gate::default();
        let writer = Writer::spawn(terminal.clone(), b"[reset]".to_vec()).unwrap();
        let direct = Gate::default();
        let (handle, restored) = handle(&writer, &direct);
        assert!(handle.give_back(&writer, b"[tail]", LONG));
        assert!(handle.restore_now(LONG));
        assert!(handle.give_back(&writer, b"", LONG));
        assert_eq!(terminal.taken(), b"[reset][tail]");
        assert_eq!(direct.taken(), b"");
        assert_eq!(*lock(&restored), 1);
    }

    /// When the writer died, giving the terminal back writes the reset without it.
    #[test]
    fn giving_back_through_a_dead_writer_writes_the_reset_directly() {
        let writer = Writer::spawn(Breaks, b"[reset]".to_vec()).unwrap();
        let direct = Gate::default();
        let (handle, restored) = handle(&writer, &direct);
        writer.send(b"[frame]".to_vec()).unwrap();
        assert!(writer.wait_idle(LONG));
        assert!(handle.give_back(&writer, b"", LONG));
        assert_eq!(direct.taken(), b"\x18\x1b\\[reset]");
        assert_eq!(*lock(&restored), 1);
    }

    /// A panic gives the terminal back on the thread that draws and on the main thread, writes the
    /// reset without the writer on the writer's own thread, and leaves it on any other.
    #[test]
    fn which_panics_give_the_terminal_back() {
        let here = std::thread::current().id();
        let other = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        assert_eq!(
            panic_ends(here, None, Some(other), Some(here)),
            Ends::Program
        );
        assert_eq!(
            panic_ends(here, Some("main"), Some(other), Some(other)),
            Ends::Program
        );
        assert_eq!(panic_ends(here, None, Some(here), Some(here)), Ends::Writer);
        assert_eq!(
            panic_ends(here, Some("worker"), Some(other), Some(other)),
            Ends::Nothing
        );
        assert_eq!(panic_ends(here, None, None, None), Ends::Nothing);
    }

    /// The thread whose panic gives the terminal back is the one that opened it until another
    /// draws.
    #[test]
    fn the_thread_that_draws_is_the_last_to_say_so() {
        let writer = Writer::spawn(io::sink(), Vec::new()).unwrap();
        let (handle, _) = handle(&writer, &Gate::default());
        let here = std::thread::current().id();
        assert_eq!(*lock(&handle.inner.drawing), Some(here));
        let drawing = {
            let handle = handle.clone();
            std::thread::spawn(move || {
                handle.draw_here();
                std::thread::current().id()
            })
            .join()
            .unwrap()
        };
        assert_eq!(*lock(&handle.inner.drawing), Some(drawing));
    }

    /// The handle a panic uses is the terminal's while it's open, and none after.
    #[test]
    fn a_closed_terminal_is_forgotten() {
        let writer = Writer::spawn(io::sink(), Vec::new()).unwrap();
        let (first, _) = handle(&writer, &Gate::default());
        let (later, _) = handle(&writer, &Gate::default());
        register(&first);
        unregister(&later);
        // (Other tests may register their own meanwhile.)
        let current = lock(&CURRENT).clone();
        if let Some(current) = current {
            assert!(!Arc::ptr_eq(&current.inner, &later.inner));
        }
        unregister(&first);
        let current = lock(&CURRENT).clone();
        assert!(current.is_none_or(|current| !Arc::ptr_eq(&current.inner, &first.inner)));
    }

    #[cfg(unix)]
    mod unix {
        use super::super::screen::Screen;
        use super::super::{Enabled, install_panic_hook};
        use crate::event::{Event, KeyModifiers};
        use crate::layout::Size;
        use crate::sys::Next;
        use crate::sys::unix::Signals;
        use crate::sys::unix::tests::{Bytes, LONG, Pty, find, is_child, key, next, spawn_child};
        use crate::terminal::MouseMode;
        use std::io::{self, Write};
        use std::time::Duration;

        fn enabled() -> Enabled {
            Enabled {
                alternate_screen: true,
                mouse: MouseMode::ButtonsAndDrags,
                focus: true,
                bracketed_paste: true,
                save_title: true,
                ..Enabled::default()
            }
        }

        /// A screen on `pty`, with pipes that stand in for SIGWINCH and SIGCONT.
        fn screen(pty: &Pty) -> (Screen, io::PipeWriter, io::PipeWriter) {
            let (signals, resize, resume) = Signals::unregistered().unwrap();
            let screen =
                Screen::open_with(pty.tty(), |_| Bytes::default(), Duration::ZERO, signals)
                    .unwrap();
            screen.enable(&enabled()).unwrap();
            (screen, resize, resume)
        }

        /// Taking the terminal puts it in raw mode and sends the setup, keys come out as events,
        /// and closing writes the reset, then the tail, and puts the modes back exactly.
        #[test]
        fn a_screen_takes_the_terminal_and_gives_it_back_exactly() {
            let mut pty = Pty::open(Size::new(80, 24));
            let before = pty.modes();
            let (screen, _resize, _resume) = screen(&pty);
            assert!(pty.is_raw());
            assert_eq!(screen.shared().size(), Size::new(80, 24));
            pty.read_until(0, &enabled().setup(), LONG);
            pty.type_in(b"q");
            assert_eq!(next(screen.shared()), key('q', KeyModifiers::NONE));
            let shared = screen.shared().clone();
            let handle = screen.restore_handle();
            assert!(screen.close(b"[tail]", LONG));
            let reset = pty.read_until(0, &enabled().reset(), LONG);
            pty.read_until(reset, b"[tail]", LONG);
            assert_eq!(pty.modes(), before);
            assert!(handle.is_given_back());
            assert_eq!(shared.events.next_timeout(LONG), Next::Closed);
            // A restore handle kept after close writes nothing more.
            assert!(handle.restore_now(LONG));
            pty.read_ready();
            assert_eq!(find(&pty.shown[reset..], b"\x18"), None);
            assert_eq!(pty.modes(), before);
        }

        /// After a stop that something else ended, SIGCONT puts raw mode back, sends the reset and
        /// then the setup, so the stacks the setup pushes don't grow, queues `Resumed`, and the
        /// next frame redraws everything.
        #[test]
        fn sigcont_takes_the_terminal_back() {
            let mut pty = Pty::open(Size::new(80, 24));
            let before = pty.modes();
            let (screen, _resize, mut resume) = screen(&pty);
            let setup = enabled().setup();
            let first = pty.read_until(0, &setup, LONG);
            assert!(screen.shared().take_redraw());
            // (A shell had the terminal meanwhile, in its own modes.)
            let raw = pty.modes();
            {
                let mut shell = rustix::termios::tcgetattr(&pty.slave).unwrap();
                shell.local_modes |=
                    rustix::termios::LocalModes::ICANON | rustix::termios::LocalModes::ECHO;
                rustix::termios::tcsetattr(
                    &pty.slave,
                    rustix::termios::OptionalActions::Now,
                    &shell,
                )
                .unwrap();
            }
            // (The program turned the mouse off since it opened.)
            let changed = Enabled {
                mouse: MouseMode::Off,
                ..enabled()
            };
            screen.set_enabled(&changed);
            resume.write_all(b"X").unwrap();
            assert_eq!(next(screen.shared()), Event::Resumed);
            assert_eq!(pty.modes(), raw);
            let mut again = changed.reset();
            again.extend_from_slice(&changed.setup());
            pty.read_until(first + setup.len(), &again, LONG);
            assert!(screen.shared().take_redraw());
            assert!(screen.close(b"", LONG));
            pty.read_until(first + setup.len(), &changed.reset(), LONG);
            assert_eq!(pty.modes(), before);
        }

        /// The panics of [`a_panic_gives_the_terminal_back_before_its_message`], each with what it
        /// says and whether it gives the terminal back.
        const PANICS: [(&str, &str, bool); 3] = [
            ("panic-opening", "the thread that opened breaks", true),
            ("panic-drawing", "the thread that draws breaks", true),
            ("panic-elsewhere", "a worker breaks", false),
        ];

        /// A panic on the thread that opened the terminal, or on the one that draws, gives the
        /// terminal back before its message prints, and one on another thread leaves it to the
        /// program.
        #[test]
        fn a_panic_gives_the_terminal_back_before_its_message() {
            const TEST: &str = "a_panic_gives_the_terminal_back_before_its_message";
            for (case, message, _) in PANICS {
                if is_child(case) {
                    install_panic_hook();
                    let screen = Screen::open_with(
                        crate::sys::unix::Tty::open().unwrap(),
                        |_| Bytes::default(),
                        Duration::ZERO,
                        Signals::register(false).unwrap(),
                    )
                    .unwrap();
                    screen.enable(&enabled()).unwrap();
                    screen.writer().send(b"<ready>".to_vec()).unwrap();
                    assert!(screen.writer().wait_idle(LONG));
                    assert!(case != "panic-opening", "{message}");
                    let drawing = case == "panic-drawing";
                    std::thread::scope(|scope| {
                        let thread = scope.spawn(|| {
                            if drawing {
                                screen.draw_here();
                            }
                            panic!("{message}");
                        });
                        assert!(thread.join().is_err());
                    });
                    assert_eq!(screen.restore_handle().is_given_back(), drawing);
                    screen.writer().send(b"<after>".to_vec()).unwrap();
                    assert!(screen.writer().wait_idle(LONG));
                    assert!(screen.close(b"", LONG));
                    return;
                }
            }
            let reset = enabled().reset();
            for (case, message, gives_back) in PANICS {
                let mut pty = Pty::open(Size::new(80, 24));
                let before = pty.modes();
                let mut child = spawn_child(module_path!(), TEST, case, &pty);
                let ready = pty.read_until(0, b"<ready>", LONG);
                let message = pty.read_until(ready, message.as_bytes(), LONG);
                let status = child.wait().unwrap();
                pty.read_ready();
                let shown = String::from_utf8_lossy(&pty.shown).into_owned();
                let given_back = pty.read_until(ready, &reset, LONG);
                assert_eq!(given_back < message, gives_back, "{case}: {shown:?}");
                // (Only the opening thread's panic fails the child's test.)
                assert_eq!(
                    status.success(),
                    case != "panic-opening",
                    "{case}: {shown:?}"
                );
                // Once the terminal is given back, nothing the program writes reaches it.
                let after = find(&pty.shown, b"<after>");
                assert_eq!(after.is_some(), !gives_back, "{case}: {shown:?}");
                assert_eq!(pty.modes(), before, "{case}");
            }
        }

        /// Suspending, or a SIGTSTP from outside, gives the terminal back while the process is
        /// stopped, and once it runs again takes it back, queues one `Resumed` and asks for a full
        /// redraw.
        #[test]
        fn a_stop_gives_the_terminal_back_until_the_process_runs_again() {
            use rustix::process::{Pid, Signal, WaitOptions, kill_process, waitpid};
            const TEST: &str = "a_stop_gives_the_terminal_back_until_the_process_runs_again";
            for case in ["suspend", "outside"] {
                if is_child(case) {
                    let screen = Screen::open_with(
                        crate::sys::unix::Tty::open().unwrap(),
                        |_| Bytes::default(),
                        Duration::ZERO,
                        Signals::register(false).unwrap(),
                    )
                    .unwrap();
                    screen.enable(&enabled()).unwrap();
                    assert!(screen.shared().take_redraw());
                    screen.writer().send(b"<ready>".to_vec()).unwrap();
                    if case == "suspend" {
                        screen.suspend().unwrap();
                    }
                    assert_eq!(next(screen.shared()), Event::Resumed);
                    assert!(screen.shared().take_redraw());
                    // (The SIGCONT that ended the stop gives no second `Resumed`.)
                    assert_eq!(
                        screen
                            .shared()
                            .events
                            .next_timeout(Duration::from_millis(200)),
                        Next::Empty
                    );
                    let modes = rustix::termios::tcgetattr(rustix::stdio::stdin()).unwrap();
                    assert!(
                        !modes
                            .local_modes
                            .contains(rustix::termios::LocalModes::ICANON)
                    );
                    screen.writer().send(b"<resumed>".to_vec()).unwrap();
                    assert!(screen.writer().wait_idle(LONG));
                    assert!(screen.close(b"", LONG));
                    return;
                }
            }
            let (setup, reset) = (enabled().setup(), enabled().reset());
            for case in ["suspend", "outside"] {
                let mut pty = Pty::open(Size::new(80, 24));
                let before = pty.modes();
                let mut child = spawn_child(module_path!(), TEST, case, &pty);
                let pid = Pid::from_child(&child);
                let ready = pty.read_until(0, b"<ready>", LONG);
                if case == "outside" {
                    kill_process(pid, Signal::TSTP).unwrap();
                }
                let given_back = pty.read_until(ready, &reset, LONG);
                let started = std::time::Instant::now();
                loop {
                    let status = waitpid(Some(pid), WaitOptions::UNTRACED | WaitOptions::NOHANG);
                    match status.unwrap() {
                        Some((_, status)) if status.stopped() => break,
                        Some((_, status)) => panic!("{case}: the child ended, {status:?}"),
                        None => {
                            assert!(started.elapsed() < LONG, "{case}: the child never stopped");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                }
                assert_eq!(pty.modes(), before, "{case}");
                kill_process(pid, Signal::CONT).unwrap();
                let taken_back = pty.read_until(given_back, &setup, LONG);
                let resumed = pty.read_until(taken_back, b"<resumed>", LONG);
                let status = child.wait().unwrap();
                pty.read_ready();
                let shown = String::from_utf8_lossy(&pty.shown).into_owned();
                assert!(status.success(), "{case}: {status}: {shown:?}");
                pty.read_until(resumed, &reset, LONG);
                assert_eq!(pty.modes(), before, "{case}");
            }
        }

        /// With the ending signals asked for, SIGTERM gives the terminal back and then ends the
        /// program as it would have. Once the terminal closed, SIGTSTP and SIGTERM act as they did
        /// before it opened.
        #[test]
        fn signals_act_as_before_once_the_terminal_closes() {
            use rustix::process::{Pid, Signal, WaitOptions, kill_process, waitpid};
            use std::os::unix::process::ExitStatusExt;
            const TEST: &str = "signals_act_as_before_once_the_terminal_closes";
            for case in ["term-open", "after-close"] {
                if is_child(case) {
                    let screen = Screen::open_with(
                        crate::sys::unix::Tty::open().unwrap(),
                        |_| Bytes::default(),
                        Duration::ZERO,
                        Signals::register(true).unwrap(),
                    )
                    .unwrap();
                    screen.enable(&enabled()).unwrap();
                    if case == "after-close" {
                        assert!(screen.close(b"<closed>", LONG));
                    } else {
                        screen.writer().send(b"<ready>".to_vec()).unwrap();
                    }
                    std::thread::sleep(Duration::from_secs(10));
                    return;
                }
            }
            let reset = enabled().reset();
            let mut pty = Pty::open(Size::new(80, 24));
            let before = pty.modes();
            let child = spawn_child(module_path!(), TEST, "term-open", &pty);
            let ready = pty.read_until(0, b"<ready>", LONG);
            kill_process(Pid::from_child(&child), Signal::TERM).unwrap();
            let mut child = child;
            let status = child.wait().unwrap();
            assert_eq!(status.signal(), Some(Signal::TERM.as_raw()), "{status}");
            pty.read_until(ready, &reset, LONG);
            assert_eq!(pty.modes(), before);

            let mut pty = Pty::open(Size::new(80, 24));
            let mut child = spawn_child(module_path!(), TEST, "after-close", &pty);
            let pid = Pid::from_child(&child);
            pty.read_until(0, b"<closed>", LONG);
            kill_process(pid, Signal::TSTP).unwrap();
            let started = std::time::Instant::now();
            loop {
                match waitpid(Some(pid), WaitOptions::UNTRACED | WaitOptions::NOHANG).unwrap() {
                    Some((_, status)) if status.stopped() => break,
                    Some((_, status)) => panic!("the child ended, {status:?}"),
                    None => {
                        assert!(started.elapsed() < LONG, "SIGTSTP didn't stop it");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
            kill_process(pid, Signal::CONT).unwrap();
            kill_process(pid, Signal::TERM).unwrap();
            let status = child.wait().unwrap();
            assert_eq!(status.signal(), Some(Signal::TERM.as_raw()), "{status}");
        }
    }
}
