//! The console on Windows. Its modes and code pages, saved and restored, the reader thread, which
//! waits on the console's input and a wake event and reads input records, and the screen that puts
//! them together with the writer.
//!
//! The calls into the console API are in `console`, the only unsafe code on Windows. Everything
//! around them works through the [`Console`] and [`Input`] traits, so it builds and is tested on
//! every system with a console played in memory.

#[cfg(windows)]
mod console;
pub(crate) mod modes;
pub(crate) mod reader;
pub(crate) mod screen;
#[cfg(test)]
pub(crate) mod tests;

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::event::decode::{Decoded, Decoder, Query};
use crate::event::records::InputRecord;
use crate::layout::Size;
use crate::output::writer::{LOCK_WAIT, lock, try_lock_for};
use crate::restore::{CLOSE_WAIT, RestoreHandle};

/// The console's window, as a rectangle of the screen buffer, inclusive at both ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) left: i16,
    pub(crate) top: i16,
    pub(crate) right: i16,
    pub(crate) bottom: i16,
}

impl Window {
    /// The window's size in cells.
    pub(crate) fn size(self) -> Size {
        let span =
            |from: i16, to: i16| u16::try_from(i32::from(to) - i32::from(from) + 1).unwrap_or(0);
        Size::new(span(self.left, self.right), span(self.top, self.bottom))
    }

    /// A cell of the screen buffer as a cell of the window, which is where the program draws.
    pub(crate) fn relative(self, column: u16, row: u16) -> (u16, u16) {
        let from = |at: u16, origin: i16| at.saturating_sub(u16::try_from(origin).unwrap_or(0));
        (from(column, self.left), from(row, self.top))
    }
}

/// The calls the library makes on a console, so the logic around them runs anywhere.
pub(crate) trait Console: Send + Sync + 'static {
    /// What the reader reads with.
    type Input: Input;
    /// What the writer writes to.
    type Output: Write + Send + 'static;

    fn input_mode(&self) -> io::Result<u32>;
    fn set_input_mode(&self, mode: u32) -> io::Result<()>;
    fn output_mode(&self) -> io::Result<u32>;
    fn set_output_mode(&self, mode: u32) -> io::Result<()>;
    /// The window's rectangle.
    fn window(&self) -> io::Result<Window>;
    /// A handle on the console's output of its own.
    fn output(&self) -> io::Result<Self::Output>;
    /// What the reader reads with, and what wakes it.
    fn input(&self) -> io::Result<(Self::Input, Arc<dyn Wake>)>;
}

/// The reader's end of the console's input.
pub(crate) trait Input: Send + 'static {
    /// Waits until records are ready or the reader is woken, at most `timeout`, or with no limit
    /// for `None`.
    fn wait(&self, timeout: Option<Duration>) -> io::Result<Ready>;
    /// Appends the records that are ready, a batch of at most [`READ_RECORDS`], or none without
    /// waiting when none is.
    fn read(&self, records: &mut Vec<InputRecord>) -> io::Result<()>;
    /// Whether more records are ready.
    fn more(&self) -> io::Result<bool>;
    /// The window's rectangle, which mouse records and size records need.
    fn window(&self) -> io::Result<Window>;
}

/// Wakes the reader from its wait.
pub(crate) trait Wake: Send + Sync {
    fn wake(&self);
}

/// What [`Input::wait`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ready {
    Input,
    Woken,
    TimedOut,
}

/// How many records the reader reads at once.
pub(crate) const READ_RECORDS: usize = 64;

/// How the console delivers keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum InputMode {
    /// Key records, which keep what a byte encoding loses (Shift+Enter, releases, Alt Gr).
    #[default]
    Records,
    /// Bytes as a terminal sends them (`ENABLE_VIRTUAL_TERMINAL_INPUT`), each in the character of a
    /// key record, so replies to queries, the kitty keyboard protocol and bracketed paste decode
    /// as on Unix.
    Vt,
}

/// The decoding the reader does, of records and of the bytes records carry.
pub(crate) trait Decode: Send + 'static {
    fn feed(&mut self, bytes: &[u8], out: &mut Vec<Decoded>);
    fn has_pending(&self) -> bool;
    fn flush_pending(&mut self, out: &mut Vec<Decoded>);
    fn has_held(&self) -> bool;
    fn flush_held(&mut self, out: &mut Vec<Decoded>);
    fn expect(&mut self, query: Query);
    fn cancel_expected(&mut self);
    fn decode_records(&mut self, records: &[InputRecord], out: &mut Vec<Decoded>);
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

    fn has_held(&self) -> bool {
        Decoder::has_held(self)
    }

    fn flush_held(&mut self, out: &mut Vec<Decoded>) {
        Decoder::flush_held(self, out);
    }

    fn expect(&mut self, query: Query) {
        Decoder::expect(self, query);
    }

    fn cancel_expected(&mut self) {
        Decoder::cancel_expected(self);
    }

    fn decode_records(&mut self, records: &[InputRecord], out: &mut Vec<Decoded>) {
        Decoder::decode_records(self, records, out);
    }
}

/// What writes UTF-16 to the console.
pub(crate) trait WriteUnits {
    /// Writes some of `units`, and says how many.
    fn write_units(&mut self, units: &[u16]) -> io::Result<usize>;
}

/// Writes the library's UTF-8 to the console as UTF-16, so the console's code page never
/// matters and never changes. A write that ends inside a character keeps its start for the next.
#[derive(Debug)]
pub(crate) struct Utf16Writer<U> {
    out: U,
    /// The start of a character the last write ended inside.
    carry: Vec<u8>,
    units: Vec<u16>,
}

impl<U> Utf16Writer<U> {
    pub(crate) fn new(out: U) -> Self {
        Self {
            out,
            carry: Vec::new(),
            units: Vec::new(),
        }
    }
}

impl<U: WriteUnits> Write for Utf16Writer<U> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut text = std::mem::take(&mut self.carry);
        text.extend_from_slice(bytes);
        self.units.clear();
        let mut rest = text.as_slice();
        while !rest.is_empty() {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    self.units.extend(valid.encode_utf16());
                    rest = &[];
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    self.units
                        .extend(String::from_utf8_lossy(valid).encode_utf16());
                    match error.error_len() {
                        // (A character the next write finishes.)
                        None => {
                            self.carry = after.to_vec();
                            rest = &[];
                        }
                        Some(bad) => {
                            self.units.push(0xfffd);
                            rest = after.get(bad..).unwrap_or(&[]);
                        }
                    }
                }
            }
        }
        let mut done = 0;
        while let Some(left) = self.units.get(done..).filter(|left| !left.is_empty()) {
            match self.out.write_units(left)? {
                0 => return Err(io::ErrorKind::WriteZero.into()),
                written => done += written,
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The screen whose console a console control event gives back, when it's asked to.
static CONSOLE_EVENTS: Mutex<Option<Arc<RestoreHandle>>> = Mutex::new(None);

/// Makes a console control event (Ctrl+Break, the console closing, logoff or shutdown) give back
/// the console `handle` has, from now on.
pub(crate) fn restore_on_console_events(handle: &Arc<RestoreHandle>) -> io::Result<()> {
    *lock(&CONSOLE_EVENTS) = Some(handle.clone());
    install_console_handler()
}

/// Forgets `handle`, if console control events give back its console.
pub(crate) fn forget_console_events(handle: &Arc<RestoreHandle>) {
    let mut current = lock(&CONSOLE_EVENTS);
    if current
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, handle))
    {
        *current = None;
    }
}

#[cfg(windows)]
fn install_console_handler() -> io::Result<()> {
    static INSTALLED: std::sync::OnceLock<io::Result<()>> = std::sync::OnceLock::new();
    match INSTALLED.get_or_init(|| console::set_ctrl_handler(on_console_event)) {
        Ok(()) => Ok(()),
        Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
    }
}

#[cfg(not(windows))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same as on Windows, where installing the handler can fail"
)]
fn install_console_handler() -> io::Result<()> {
    Ok(())
}

/// What the system calls on a console control event, on a thread of its own. It gives the
/// terminal back, waiting a second at most, and returns false, so the system goes on to end the
/// process as it does without a handler.
pub(crate) extern "system" fn on_console_event(_event: u32) -> i32 {
    if let Some(handle) =
        try_lock_for(&CONSOLE_EVENTS, LOCK_WAIT).and_then(|current| current.clone())
    {
        handle.restore_now(CLOSE_WAIT);
    }
    0
}

#[cfg(windows)]
pub(crate) use self::console::{ConsoleInput, WakeEvent};

/// The console the process is attached to, `CONIN$` and `CONOUT$`, opened whatever standard
/// input and output are redirected to.
#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct WinConsole {
    input: std::fs::File,
    output: std::fs::File,
}

#[cfg(windows)]
impl WinConsole {
    /// Opens the console's input and output. Fails when the process has no console.
    pub(crate) fn open() -> io::Result<Self> {
        let open = |name| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(name)
        };
        Ok(Self {
            input: open("CONIN$")?,
            output: open("CONOUT$")?,
        })
    }
}

#[cfg(windows)]
impl Console for WinConsole {
    type Input = ConsoleInput;
    type Output = Utf16Writer<std::fs::File>;

    fn input_mode(&self) -> io::Result<u32> {
        console::mode(&self.input)
    }

    fn set_input_mode(&self, mode: u32) -> io::Result<()> {
        console::set_mode(&self.input, mode)
    }

    fn output_mode(&self) -> io::Result<u32> {
        console::mode(&self.output)
    }

    fn set_output_mode(&self, mode: u32) -> io::Result<()> {
        console::set_mode(&self.output, mode)
    }

    fn window(&self) -> io::Result<Window> {
        console::window(&self.output)
    }

    fn output(&self) -> io::Result<Utf16Writer<std::fs::File>> {
        Ok(Utf16Writer::new(self.output.try_clone()?))
    }

    fn input(&self) -> io::Result<(ConsoleInput, Arc<dyn Wake>)> {
        let wake = Arc::new(WakeEvent::new()?);
        let input = ConsoleInput::new(
            self.input.try_clone()?,
            self.output.try_clone()?,
            wake.clone(),
        );
        Ok((input, wake))
    }
}
