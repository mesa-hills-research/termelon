//! The harness's calls into Windows, the only unsafe code in the harness. Each call, and each read
//! of a union field, is in a block of its own, whose comment names its entry in
//! `docs/why-this-is-safe.md`. The module copies what the calls give into plain values and decides
//! nothing, so what the drivers do with them is safe code.
#![expect(
    unsafe_code,
    reason = "the console and input APIs are reachable only through foreign calls, and their records are unions"
)]

use std::fs::File;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Console::{
    AttachConsole, CHAR_INFO, CONSOLE_CURSOR_INFO, CONSOLE_SCREEN_BUFFER_INFO, COORD, FreeConsole,
    GetConsoleCursorInfo, GetConsoleScreenBufferInfo, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT,
    KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, ReadConsoleOutputW, SMALL_RECT,
    SetConsoleScreenBufferSize, SetConsoleWindowInfo, WriteConsoleInputW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, SetForegroundWindow,
};
use windows_sys::core::BOOL;

fn raw(file: &File) -> HANDLE {
    file.as_handle().as_raw_handle()
}

/// The error of a call that returned `FALSE`.
fn check(ok: BOOL) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Leaves the console the process has, if it has one, and attaches it to the console process
/// `pid` has. `CONIN$` and `CONOUT$` then open that console's input and active screen buffer.
///
/// # Errors
///
/// When the process has no console, or is gone.
pub fn attach_console(pid: u32) -> io::Result<()> {
    // SAFETY: [harness-free-console] it takes nothing, and fails harmlessly for a process with no
    // console.
    let _ = unsafe { FreeConsole() };
    // SAFETY: [harness-attach-console] it takes a process id, which the call checks.
    check(unsafe { AttachConsole(pid) })
}

/// The screen buffer `output` is open on, as `GetConsoleScreenBufferInfo` gives it.
fn info(output: &File) -> io::Result<CONSOLE_SCREEN_BUFFER_INFO> {
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: [harness-buffer-info] the handle is borrowed for the call, and `info` is a structure
    // of the type the call writes.
    check(unsafe { GetConsoleScreenBufferInfo(raw(output), &raw mut info) })?;
    Ok(info)
}

fn coordinate(value: u16) -> i16 {
    i16::try_from(value).unwrap_or(i16::MAX)
}

/// Makes the screen buffer `output` is open on `cols` by `rows`, with a window as large, so it has
/// no scrollback.
///
/// # Errors
///
/// When the console refuses the size, such as one larger than the screen.
pub fn set_console_size(output: &File, cols: u16, rows: u16) -> io::Result<()> {
    let (cols, rows) = (coordinate(cols.max(1)), coordinate(rows.max(1)));
    // (The window can't be larger than the buffer, so it shrinks to a cell first.)
    let cell = SMALL_RECT {
        Left: 0,
        Top: 0,
        Right: 0,
        Bottom: 0,
    };
    // SAFETY: [harness-console-size] the handle is borrowed for the call, and the rectangle is a
    // local that lives through it.
    check(unsafe { SetConsoleWindowInfo(raw(output), 1, &raw const cell) })?;
    let size = COORD { X: cols, Y: rows };
    // SAFETY: [harness-console-size] the handle is borrowed for the call, and the size is a value.
    check(unsafe { SetConsoleScreenBufferSize(raw(output), size) })?;
    let window = SMALL_RECT {
        Left: 0,
        Top: 0,
        Right: cols - 1,
        Bottom: rows - 1,
    };
    // SAFETY: [harness-console-size] the handle is borrowed for the call, and the rectangle is a
    // local that lives through it.
    check(unsafe { SetConsoleWindowInfo(raw(output), 1, &raw const window) })
}

/// A console window's cells, row by row, and its cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cells {
    /// The window's width.
    pub cols: u16,
    /// The window's height.
    pub rows: u16,
    /// Each cell's UTF-16 unit and attributes, row by row.
    pub cells: Vec<(u16, u16)>,
    /// The cursor, relative to the window.
    pub cursor: (u16, u16),
    /// Whether the cursor shows.
    pub visible: bool,
}

/// The cells of the window of the screen buffer `output` is open on, and its cursor.
///
/// # Errors
///
/// When the console doesn't say.
pub fn read_window(output: &File) -> io::Result<Cells> {
    let info = info(output)?;
    let window = info.srWindow;
    let span = |from: i16, to: i16| u16::try_from(i32::from(to) - i32::from(from) + 1).unwrap_or(0);
    let (cols, rows) = (
        span(window.Left, window.Right),
        span(window.Top, window.Bottom),
    );
    let mut buffer = vec![CHAR_INFO::default(); usize::from(cols) * usize::from(rows)];
    let mut region = window;
    let size = COORD {
        X: coordinate(cols),
        Y: coordinate(rows),
    };
    // SAFETY: [harness-read-output] the buffer holds `cols` by `rows` cells, the size the call is
    // given, and the region is the window, as large, so the call writes inside the buffer. The
    // handle is borrowed for the call.
    let ok = unsafe {
        ReadConsoleOutputW(
            raw(output),
            buffer.as_mut_ptr(),
            size,
            COORD { X: 0, Y: 0 },
            &raw mut region,
        )
    };
    check(ok)?;
    let cells = buffer
        .iter()
        .map(|cell| {
            // SAFETY: [harness-char-union] both fields of this union are integers of the same
            // place, and `ReadConsoleOutputW` fills the UTF-16 one.
            let unit = unsafe { cell.Char.UnicodeChar };
            (unit, cell.Attributes)
        })
        .collect();
    let mut cursor = CONSOLE_CURSOR_INFO::default();
    // SAFETY: [harness-cursor-info] the handle is borrowed for the call, and `cursor` is a
    // structure of the type the call writes.
    check(unsafe { GetConsoleCursorInfo(raw(output), &raw mut cursor) })?;
    let relative =
        |at: i16, origin: i16| u16::try_from(i32::from(at) - i32::from(origin)).unwrap_or(0);
    Ok(Cells {
        cols,
        rows,
        cells,
        cursor: (
            relative(info.dwCursorPosition.X, window.Left),
            relative(info.dwCursorPosition.Y, window.Top),
        ),
        visible: cursor.bVisible != 0,
    })
}

/// A key record to put into a console's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRecord {
    /// A press, or a release.
    pub down: bool,
    /// The virtual-key code, or 0 for a character with no key.
    pub virtual_key: u16,
    /// The scan code.
    pub scan_code: u16,
    /// The UTF-16 unit it types.
    pub unicode: u16,
    /// The control key state, as `wincon.h` has it.
    pub control_state: u32,
}

/// Puts key records into the input buffer `input` is open on, as if typed.
///
/// # Errors
///
/// When the console refuses them.
pub fn write_input(input: &File, keys: &[KeyRecord]) -> io::Result<()> {
    let records: Vec<INPUT_RECORD> = keys
        .iter()
        .map(|key| INPUT_RECORD {
            EventType: u16::try_from(KEY_EVENT).unwrap_or(1),
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: BOOL::from(key.down),
                    wRepeatCount: 1,
                    wVirtualKeyCode: key.virtual_key,
                    wVirtualScanCode: key.scan_code,
                    uChar: KEY_EVENT_RECORD_0 {
                        UnicodeChar: key.unicode,
                    },
                    dwControlKeyState: key.control_state,
                },
            },
        })
        .collect();
    let count = u32::try_from(records.len()).unwrap_or(u32::MAX);
    let mut written = 0;
    // SAFETY: [harness-write-input] the handle is borrowed for the call, the vector holds `count`
    // records, which bounds what the call reads, and `written` is a u32 it writes.
    let ok = unsafe { WriteConsoleInputW(raw(input), records.as_ptr(), count, &raw mut written) };
    check(ok)
}

/// A key's press or release, as `SendInput` makes it.
fn key_input(virtual_key: u16, down: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: if down { 0 } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Makes `inputs` with `SendInput`, as the keyboard would.
fn send(inputs: &[INPUT]) -> io::Result<()> {
    let count = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
    let size = i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX);
    // SAFETY: [harness-send-input] the slice holds `count` inputs, which bounds what the call
    // reads, each a keyboard input whose union holds `ki`, and `size` is the size of one.
    let sent = unsafe { SendInput(count, inputs.as_ptr(), size) };
    if sent == count {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Brings the first window of class `class` to the front, and presses the keys of `chord`
/// together there, in order, then releases them in reverse. Whether the window was found and in
/// front. Windows lets a process bring a window to the front when it made the last input, so it
/// presses and releases Alt first.
///
/// # Errors
///
/// When the keys couldn't be sent.
pub fn press_in_window(class: &str, chord: &[u16]) -> io::Result<bool> {
    /// The Alt key's virtual-key code, `VK_MENU`.
    const ALT: u16 = 0x12;
    let class: Vec<u16> = class.encode_utf16().chain([0]).collect();
    // SAFETY: [harness-find-window] the class name is a NUL-terminated UTF-16 string that lives
    // through the call, and the window name is null, which matches any.
    let window = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
    if window.is_null() {
        return Ok(false);
    }
    // SAFETY: [harness-foreground] it takes nothing.
    if unsafe { GetForegroundWindow() } != window {
        send(&[key_input(ALT, true), key_input(ALT, false)])?;
        // SAFETY: [harness-foreground] the window is one `FindWindowW` just gave, and one that
        // closed since fails the call, which changes nothing.
        let _ = unsafe { SetForegroundWindow(window) };
    }
    // SAFETY: [harness-foreground] it takes nothing.
    if unsafe { GetForegroundWindow() } != window {
        return Ok(false);
    }
    let mut inputs: Vec<INPUT> = chord.iter().map(|&key| key_input(key, true)).collect();
    inputs.extend(chord.iter().rev().map(|&key| key_input(key, false)));
    send(&inputs)?;
    Ok(true)
}
