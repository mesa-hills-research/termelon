//! The console while a program has it, with its writer, its reader and its modes, as
//! `restore::screen::Screen` is the terminal on Unix.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::modes::{InputSettings, Modes};
use super::reader::{Reader, Settings};
use super::{Console, Decode, InputMode, Window, forget_console_events, restore_on_console_events};
use crate::event::decode::Query;
use crate::layout::Size;
use crate::output::writer::{Writer, lock};
use crate::restore::{CLOSE_WAIT, Enabled, RestoreHandle, register, unregister};
use crate::sys::Shared;
use crate::terminal::MouseMode;

/// The console a program has.
#[cfg(windows)]
pub(crate) type Screen = ScreenOn<super::WinConsole>;

/// The console while a program has it, on any [`Console`].
#[derive(Debug)]
pub(crate) struct ScreenOn<C> {
    writer: Writer,
    shared: Arc<Shared>,
    reader: Option<Reader>,
    restore: RestoreHandle,
    modes: Arc<Modes<C>>,
    settings: Arc<Settings>,
    /// What the program turned on, of which the kitty keyboard flags apply only to bytes.
    enabled: Mutex<Enabled>,
    /// The restore handle a console control event uses, once the program asks for that.
    console_events: Arc<RestoreHandle>,
    closed: bool,
}

/// What of `enabled` the console takes in input `mode`. The kitty keyboard flags go only where
/// keys come as bytes. Under a pseudoconsole the push reaches the terminal, which would then send
/// keys a client reading records can't take.
fn effective(enabled: &Enabled, mode: InputMode) -> Enabled {
    let mut effective = enabled.clone();
    if mode == InputMode::Records {
        effective.keyboard = None;
    }
    effective
}

/// The console's output for a reset written without the writer, a handle kept from open, so
/// nothing needs opening then.
struct Direct<W>(Arc<Mutex<W>>);

impl<W: Write> Write for Direct<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        lock(&self.0).write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        lock(&self.0).flush()
    }
}

impl<C: Console> ScreenOn<C> {
    /// Takes the console. It saves its modes and code pages and sets them for the program (input
    /// as key records, output reading escape sequences, both code pages UTF-8), starts the writer
    /// and the reader, whose decoder `decoder` makes from the byte Backspace sends. A panic gives
    /// the console back from now on, where `install_panic_hook` was called. It sends nothing
    /// until [`enable`](Self::enable). When it fails, the console is as it was. A console without
    /// virtual terminal processing fails with [`io::ErrorKind::Unsupported`].
    pub(crate) fn open<D: Decode>(
        console: C,
        decoder: impl FnOnce(u8) -> D,
        escape_timeout: Duration,
    ) -> io::Result<Self> {
        let console = Arc::new(console);
        let modes = Arc::new(Modes::enter(console.clone(), InputSettings::default())?);
        let size = console.window().map_or(Size::ZERO, Window::size);
        let started = console.output().and_then(|output| {
            let direct = Arc::new(Mutex::new(console.output()?));
            let writer = Writer::spawn(output, Enabled::default().reset())?;
            Ok((writer, direct))
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
            Arc::new(move || Box::new(Direct(direct.clone()))),
            Box::new({
                let modes = modes.clone();
                move || {
                    let _ = modes.restore();
                }
            }),
        );
        register(&restore);
        let shared = Arc::new(Shared::new(size));
        writer.on_failure({
            let shared = shared.clone();
            move || shared.events.close()
        });
        let settings = Arc::new(Settings::default());
        let reader = console.input().and_then(|(input, wake)| {
            Reader::spawn(
                input,
                wake,
                decoder(0x7f),
                shared.clone(),
                settings.clone(),
                escape_timeout,
            )
        });
        match reader {
            Ok(reader) => Ok(Self {
                writer,
                shared,
                reader: Some(reader),
                console_events: Arc::new(restore.clone()),
                restore,
                modes,
                settings,
                enabled: Mutex::new(Enabled::default()),
                closed: false,
            }),
            Err(error) => {
                restore.give_back(&writer, &[], CLOSE_WAIT);
                unregister(&restore);
                Err(error)
            }
        }
    }

    pub(crate) fn writer(&self) -> &Writer {
        &self.writer
    }

    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub(crate) fn restore_handle(&self) -> RestoreHandle {
        self.restore.clone()
    }

    /// Makes this thread the one that draws, whose panic gives the console back.
    pub(crate) fn draw_here(&self) {
        self.restore.draw_here();
    }

    /// Turns on what `enabled` says, which the reset then undoes. The kitty keyboard flags are
    /// pushed only while keys come as bytes ([`InputMode::Vt`]). Fails once the writer failed.
    pub(crate) fn enable(&self, enabled: &Enabled) -> io::Result<()> {
        self.set_enabled(enabled);
        let effective = effective(enabled, self.modes.input().mode);
        self.writer.send(effective.setup())?;
        Ok(())
    }

    /// Makes `enabled` what the reset undoes, takes mouse records (with quick edit off) when the
    /// mouse is on, and passes focus records on when focus reports are. The program sends what
    /// turns the change on. A console that refuses the input mode keeps the one it had, which
    /// only a console gone wrong does.
    pub(crate) fn set_enabled(&self, enabled: &Enabled) {
        *lock(&self.enabled) = enabled.clone();
        let mut input = self.modes.input();
        self.writer
            .set_reset(effective(enabled, input.mode).reset());
        self.settings.set_focus(enabled.focus);
        input.mouse = enabled.mouse != MouseMode::Off;
        let _ = self.modes.set_input(input);
    }

    /// Switches how the console delivers keys. The probe runs in [`InputMode::Vt`], where the
    /// console's replies decode as replies rather than as keys typed. Fails when the console's
    /// input mode couldn't change.
    pub(crate) fn set_input(&self, mode: InputMode) -> io::Result<()> {
        let mut input = self.modes.input();
        let enabled = lock(&self.enabled).clone();
        if input.mode != mode
            && let Some(flags) = enabled.keyboard
        {
            self.writer.set_reset(effective(&enabled, mode).reset());
            let change = match mode {
                InputMode::Vt => format!("\x1b[>{flags}u"),
                InputMode::Records => "\x1b[<1u".to_owned(),
            };
            let _ = self.writer.send(change.into_bytes());
        }
        input.mode = mode;
        // (Records read meanwhile decode in the other mode. The reader goes first into bytes, and
        // last out of them, so a key record read as bytes loses no more than its modifiers.)
        if mode == InputMode::Vt {
            self.settings.set_mode(mode);
            self.modes.set_input(input)
        } else {
            let changed = self.modes.set_input(input);
            self.settings.set_mode(mode);
            changed
        }
    }

    /// Tells the decoder of a query whose answer is outstanding, before the query is written.
    pub(crate) fn expect(&self, query: Query) {
        if let Some(reader) = &self.reader {
            reader.expect(query);
        }
    }

    /// Tells the decoder to forget the queries whose answers are outstanding.
    pub(crate) fn cancel_expected(&self) {
        if let Some(reader) = &self.reader {
            reader.cancel_expected();
        }
    }

    /// Makes a console control event (Ctrl+Break, the console window closing, logoff or shutdown)
    /// give the console back before the system ends the process, as a program's own signal
    /// handling does on Unix. Fails when the handler couldn't be installed.
    pub(crate) fn restore_on_console_events(&self) -> io::Result<()> {
        restore_on_console_events(&self.console_events)
    }

    /// Windows has no job control, so a program can't be suspended there.
    #[expect(
        clippy::unused_self,
        reason = "the same call as on Unix, where suspending needs the screen"
    )]
    pub(crate) fn suspend(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "windows has no job control to suspend with",
        ))
    }
}

impl<C> ScreenOn<C> {
    /// Gives the console back, with `tail` after the reset, waiting at most `wait` for it. Returns
    /// whether the reset reached the console. Its modes and code pages are then as the program
    /// found them.
    pub(crate) fn close(mut self, tail: &[u8], wait: Duration) -> bool {
        self.finish(tail, wait)
    }

    fn finish(&mut self, tail: &[u8], wait: Duration) -> bool {
        self.closed = true;
        if let Some(reader) = self.reader.take() {
            reader.stop();
        }
        let written = self.restore.give_back(&self.writer, tail, wait);
        unregister(&self.restore);
        forget_console_events(&self.console_events);
        written
    }
}

impl<C> Drop for ScreenOn<C> {
    fn drop(&mut self) {
        if !self.closed {
            self.finish(&[], CLOSE_WAIT);
        }
    }
}
