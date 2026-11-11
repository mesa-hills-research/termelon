//! The console API, the only unsafe code on Windows. Each call, and each read of a union field, is
//! in a block of its own, whose comment names its entry in `docs/why-this-is-safe.md`. This module
//! copies what the calls give into plain structs and decides nothing, so the logic stays in safe
//! code that's tested on every system.
#![expect(
    unsafe_code,
    reason = "the console API is reachable only through foreign calls, and its records are unions"
)]

use std::fs::File;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, FOCUS_EVENT, GetConsoleMode, GetConsoleScreenBufferInfo,
    GetNumberOfConsoleInputEvents, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT, ReadConsoleInputW,
    SetConsoleCtrlHandler, SetConsoleMode, WINDOW_BUFFER_SIZE_EVENT, WriteConsoleW,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
};
use windows_sys::core::BOOL;

use super::{Input, READ_RECORDS, Ready, Wake, Window, WriteUnits};
use crate::event::records::{FocusRecord, InputRecord, KeyRecord, MouseRecord, WindowSizeRecord};
use crate::layout::Size;

fn raw(handle: BorrowedHandle<'_>) -> HANDLE {
    handle.as_raw_handle()
}

/// The error of a call that returned `FALSE`.
fn check(ok: BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The mode of the console input or output `console` is open on.
pub(super) fn mode(console: &impl AsHandle) -> io::Result<u32> {
    let mut mode = 0;
    // SAFETY: [win-console-mode] the handle is borrowed for the call, so it stays open, and `mode`
    // is a u32 the call writes.
    let ok = unsafe { GetConsoleMode(raw(console.as_handle()), &raw mut mode) };
    check(ok)?;
    Ok(mode)
}

/// Sets the mode of the console input or output `console` is open on.
pub(super) fn set_mode(console: &impl AsHandle, mode: u32) -> io::Result<()> {
    // SAFETY: [win-console-mode] the handle is borrowed for the call, so it stays open. A mode the
    // console doesn't take fails the call, which is reported.
    check(unsafe { SetConsoleMode(raw(console.as_handle()), mode) })
}

/// The window's rectangle in the screen buffer `output` is open on.
pub(super) fn window(output: &impl AsHandle) -> io::Result<Window> {
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: [win-screen-buffer-info] the handle is borrowed for the call, and `info` is a
    // structure of the size the call writes.
    let ok = unsafe { GetConsoleScreenBufferInfo(raw(output.as_handle()), &raw mut info) };
    check(ok)?;
    let rect = info.srWindow;
    Ok(Window {
        left: rect.Left,
        top: rect.Top,
        right: rect.Right,
        bottom: rect.Bottom,
    })
}

/// Writes UTF-16 to the screen buffer a file is open on, with `WriteConsoleW`, which takes the
/// console's characters whatever its code page.
impl WriteUnits for File {
    fn write_units(&mut self, units: &[u16]) -> io::Result<usize> {
        let count = u32::try_from(units.len()).unwrap_or(u32::MAX);
        let mut written = 0;
        // SAFETY: [win-write-console] the handle is borrowed for the call, the slice holds at
        // least `count` units, which bounds what the call reads, `written` is a u32 it writes, and
        // the reserved pointer is null, as the call requires.
        let ok = unsafe {
            WriteConsoleW(
                raw(self.as_handle()),
                units.as_ptr(),
                count,
                &raw mut written,
                std::ptr::null(),
            )
        };
        check(ok)?;
        Ok(usize::try_from(written).unwrap_or(0))
    }
}

/// Makes `handler` called, on a thread the system starts, for each console control event
/// (Ctrl+Break, the console closing, logoff and shutdown).
pub(super) fn set_ctrl_handler(handler: extern "system" fn(u32) -> BOOL) -> io::Result<()> {
    let handler = handler as unsafe extern "system" fn(u32) -> BOOL;
    // SAFETY: [win-ctrl-handler] the handler is a function of the program's, which lives as long
    // as the process, so the system may call it any time.
    check(unsafe { SetConsoleCtrlHandler(Some(handler), 1) })
}

/// How many records are in the input buffer `input` is open on.
fn pending(input: &impl AsHandle) -> io::Result<u32> {
    let mut count = 0;
    // SAFETY: [win-pending-input] the handle is borrowed for the call, and `count` is a u32 the
    // call writes.
    let ok = unsafe { GetNumberOfConsoleInputEvents(raw(input.as_handle()), &raw mut count) };
    check(ok)?;
    Ok(count)
}

/// Reads what records are ready from `input`, at most [`READ_RECORDS`], and appends them as
/// plain structs. Returns at once when none is ready, since `ReadConsoleInputW` would wait.
fn read_records(input: &impl AsHandle, records: &mut Vec<InputRecord>) -> io::Result<()> {
    if pending(input)? == 0 {
        return Ok(());
    }
    let mut buffer = [INPUT_RECORD::default(); READ_RECORDS];
    let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    let mut read = 0;
    // SAFETY: [win-read-input] the handle is borrowed for the call, the buffer holds `capacity`
    // records, which bounds what the call writes, and `read` is a u32 it writes.
    let ok = unsafe {
        ReadConsoleInputW(
            raw(input.as_handle()),
            buffer.as_mut_ptr(),
            capacity,
            &raw mut read,
        )
    };
    check(ok)?;
    let read = usize::try_from(read).unwrap_or(0).min(buffer.len());
    records.extend(buffer.iter().take(read).filter_map(copy_record));
    Ok(())
}

/// A record's fields, as the union field its event type names holds them. A window size record's
/// size is the buffer's, so it's left for the reader, which reads the window's. Menu records are
/// dropped.
fn copy_record(record: &INPUT_RECORD) -> Option<InputRecord> {
    let coordinate = |at: i16| u16::try_from(at).unwrap_or(0);
    match u32::from(record.EventType) {
        KEY_EVENT => {
            // SAFETY: [win-record-union] the event type says the union holds a key record.
            let key = unsafe { record.Event.KeyEvent };
            // SAFETY: [win-record-union] both fields of this union are integers of the same
            // place, and `ReadConsoleInputW` fills the UTF-16 one.
            let unicode = unsafe { key.uChar.UnicodeChar };
            Some(InputRecord::Key(KeyRecord {
                key_down: key.bKeyDown != 0,
                repeat: key.wRepeatCount,
                virtual_key: key.wVirtualKeyCode,
                scan_code: key.wVirtualScanCode,
                unicode,
                control_state: key.dwControlKeyState,
            }))
        }
        MOUSE_EVENT => {
            // SAFETY: [win-record-union] the event type says the union holds a mouse record.
            let mouse = unsafe { record.Event.MouseEvent };
            Some(InputRecord::Mouse(MouseRecord {
                column: coordinate(mouse.dwMousePosition.X),
                row: coordinate(mouse.dwMousePosition.Y),
                button_state: mouse.dwButtonState,
                control_state: mouse.dwControlKeyState,
                event_flags: mouse.dwEventFlags,
            }))
        }
        WINDOW_BUFFER_SIZE_EVENT => Some(InputRecord::WindowSize(WindowSizeRecord {
            size: Size::ZERO,
        })),
        FOCUS_EVENT => {
            // SAFETY: [win-record-union] the event type says the union holds a focus record.
            let focus = unsafe { record.Event.FocusEvent };
            Some(InputRecord::Focus(FocusRecord {
                focused: focus.bSetFocus != 0,
            }))
        }
        _ => None,
    }
}

/// An event object that wakes the reader, reset by the wait it ends.
#[derive(Debug)]
pub(crate) struct WakeEvent(OwnedHandle);

impl WakeEvent {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: [win-create-event] null security attributes and a null name are allowed, and
        // it makes an auto-reset event that starts unsignaled.
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: [win-own-event] the handle is new and nothing else holds it, so this is its one
        // owner, which closes it once.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(event) }))
    }
}

impl Wake for WakeEvent {
    fn wake(&self) {
        // SAFETY: [win-set-event] the handle is this event's own, open while `self` is.
        let _ = unsafe { SetEvent(raw(self.0.as_handle())) };
    }
}

/// Which of `handles` was signaled first, waiting at most `timeout`, or with no limit for `None`.
fn wait<const N: usize>(
    handles: [BorrowedHandle<'_>; N],
    timeout: Option<Duration>,
) -> io::Result<Option<usize>> {
    let raws = handles.map(raw);
    let count = u32::try_from(N).unwrap_or(u32::MAX);
    let millis = timeout.map_or(INFINITE, |timeout| {
        u32::try_from(timeout.as_millis()).map_or(INFINITE - 1, |ms| ms.min(INFINITE - 1))
    });
    // SAFETY: [win-wait] the array holds `count` handles, each borrowed for the call, and fewer
    // than the 64 the call takes.
    let waited = unsafe { WaitForMultipleObjects(count, raws.as_ptr(), 0, millis) };
    match waited {
        WAIT_FAILED => Err(io::Error::last_os_error()),
        WAIT_TIMEOUT => Ok(None),
        signaled => match usize::try_from(signaled.wrapping_sub(WAIT_OBJECT_0)) {
            Ok(index) if index < N => Ok(Some(index)),
            _ => Err(io::Error::other("the console's wait was abandoned")),
        },
    }
}

/// The reader's end of the console's input, `CONIN$` with the wake event, and `CONOUT$` for the
/// window's rectangle.
#[derive(Debug)]
pub(crate) struct ConsoleInput {
    input: File,
    output: File,
    wake: Arc<WakeEvent>,
}

impl ConsoleInput {
    pub(super) fn new(input: File, output: File, wake: Arc<WakeEvent>) -> Self {
        Self {
            input,
            output,
            wake,
        }
    }
}

impl Input for ConsoleInput {
    fn wait(&self, timeout: Option<Duration>) -> io::Result<Ready> {
        let handles = [self.input.as_handle(), self.wake.0.as_handle()];
        Ok(match wait(handles, timeout)? {
            Some(0) => Ready::Input,
            Some(_) => Ready::Woken,
            None => Ready::TimedOut,
        })
    }

    fn read(&self, records: &mut Vec<InputRecord>) -> io::Result<()> {
        read_records(&self.input, records)
    }

    fn more(&self) -> io::Result<bool> {
        Ok(pending(&self.input)? > 0)
    }

    fn window(&self) -> io::Result<Window> {
        window(&self.output)
    }
}

/// Calls that only the tests that need a real console make, to put records into the input buffer
/// and read back what the screen shows.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use windows_sys::Win32::System::Console::{
        COORD, FOCUS_EVENT_RECORD, GetConsoleCP, GetConsoleOutputCP, INPUT_RECORD_0,
        KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, MOUSE_EVENT_RECORD, ReadConsoleOutputCharacterW,
        WriteConsoleInputW,
    };

    /// The console's input and output code pages, which the library leaves as it finds them.
    pub(crate) fn code_pages() -> io::Result<(u32, u32)> {
        // SAFETY: [win-code-page] it takes nothing and reads the console of the process.
        let input = unsafe { GetConsoleCP() };
        // SAFETY: [win-code-page] it takes nothing and reads the console of the process.
        let output = unsafe { GetConsoleOutputCP() };
        if input == 0 || output == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((input, output))
    }

    /// Puts `records` into the input buffer `input` is open on, as if they were typed.
    pub(crate) fn write_input(input: &impl AsHandle, records: &[InputRecord]) -> io::Result<()> {
        let raws: Vec<INPUT_RECORD> = records.iter().filter_map(to_raw).collect();
        let count = u32::try_from(raws.len()).unwrap_or(u32::MAX);
        let mut written = 0;
        // SAFETY: [win-write-input] the handle is borrowed for the call, the vector holds `count`
        // records, which bounds what the call reads, and `written` is a u32 it writes.
        let ok = unsafe {
            WriteConsoleInputW(
                raw(input.as_handle()),
                raws.as_ptr(),
                count,
                &raw mut written,
            )
        };
        check(ok)
    }

    fn to_raw(record: &InputRecord) -> Option<INPUT_RECORD> {
        let at = |v: u16| i16::try_from(v).unwrap_or(i16::MAX);
        let (kind, event) = match *record {
            InputRecord::Key(key) => (
                KEY_EVENT,
                INPUT_RECORD_0 {
                    KeyEvent: KEY_EVENT_RECORD {
                        bKeyDown: BOOL::from(key.key_down),
                        wRepeatCount: key.repeat,
                        wVirtualKeyCode: key.virtual_key,
                        wVirtualScanCode: key.scan_code,
                        uChar: KEY_EVENT_RECORD_0 {
                            UnicodeChar: key.unicode,
                        },
                        dwControlKeyState: key.control_state,
                    },
                },
            ),
            InputRecord::Mouse(mouse) => (
                MOUSE_EVENT,
                INPUT_RECORD_0 {
                    MouseEvent: MOUSE_EVENT_RECORD {
                        dwMousePosition: COORD {
                            X: at(mouse.column),
                            Y: at(mouse.row),
                        },
                        dwButtonState: mouse.button_state,
                        dwControlKeyState: mouse.control_state,
                        dwEventFlags: mouse.event_flags,
                    },
                },
            ),
            InputRecord::Focus(focus) => (
                FOCUS_EVENT,
                INPUT_RECORD_0 {
                    FocusEvent: FOCUS_EVENT_RECORD {
                        bSetFocus: BOOL::from(focus.focused),
                    },
                },
            ),
            InputRecord::WindowSize(_) => return None,
        };
        Some(INPUT_RECORD {
            EventType: u16::try_from(kind).unwrap_or(0),
            Event: event,
        })
    }

    /// The characters of `count` cells of the screen buffer `output` is open on, from column
    /// `column` of row `row`.
    pub(crate) fn read_text(
        output: &impl AsHandle,
        column: i16,
        row: i16,
        count: usize,
    ) -> io::Result<String> {
        let mut units = vec![0u16; count];
        let length = u32::try_from(units.len()).unwrap_or(u32::MAX);
        let mut read = 0;
        // SAFETY: [win-read-output] the handle is borrowed for the call, the buffer holds
        // `length` units, which bounds what the call writes, and `read` is a u32 it writes.
        let ok = unsafe {
            ReadConsoleOutputCharacterW(
                raw(output.as_handle()),
                units.as_mut_ptr(),
                length,
                COORD { X: column, Y: row },
                &raw mut read,
            )
        };
        check(ok)?;
        units.truncate(usize::try_from(read).unwrap_or(0));
        Ok(String::from_utf16_lossy(&units))
    }
}
