//! Tests of the Windows layer against a console played in memory, which run on every system, and
//! tests that need a real console, which run on Windows with `--ignored`.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::modes::bits::{
    ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_MOUSE_INPUT,
    ENABLE_PROCESSED_INPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT,
};
use super::modes::{InputSettings, Modes, OUTPUT_MODE, Saved};
use super::reader::{Reader, Settings, vt_bytes};
use super::screen::ScreenOn;
use super::{
    Console, Input, InputMode, READ_RECORDS, Ready, Utf16Writer, Wake, Window, WriteUnits,
    on_console_event,
};
use crate::event::decode::{Decoder, DecoderOptions};
use crate::event::records::{FocusRecord, InputRecord, KeyRecord, MouseRecord, WindowSizeRecord};
use crate::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crate::layout::Size;
use crate::output::writer::lock;
use crate::restore::Enabled;
use crate::sys::{Next, Shared};
use crate::terminal::MouseMode;

const LONG: Duration = Duration::from_secs(5);

/// A console in memory: its modes and window, the records waiting in its input, and
/// what was written to it.
#[derive(Debug, Default)]
pub(crate) struct Fake {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct State {
    input_mode: u32,
    output_mode: u32,
    window: Window,
    /// Whether it takes virtual terminal processing.
    vt: bool,
    /// Setting the input mode fails.
    input_fails: bool,
    records: VecDeque<InputRecord>,
    woken: bool,
    /// Reading fails, as when the console goes away.
    broken: bool,
    written: Vec<u8>,
}

impl Fake {
    /// A console as a program might find it, with Windows' default modes and an 80 by 24 window
    /// low in a taller buffer.
    pub(crate) fn new() -> Arc<Self> {
        let fake = Self::default();
        {
            let mut state = lock(&fake.state);
            state.input_mode = ENABLE_PROCESSED_INPUT
                | ENABLE_LINE_INPUT
                | ENABLE_ECHO_INPUT
                | ENABLE_MOUSE_INPUT
                | ENABLE_QUICK_EDIT_MODE
                | ENABLE_EXTENDED_FLAGS
                | 0x20;
            state.output_mode = 0x3;
            state.window = Window {
                left: 0,
                top: 100,
                right: 79,
                bottom: 123,
            };
            state.vt = true;
        }
        Arc::new(fake)
    }

    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let value = f(&mut lock(&self.state));
        self.changed.notify_all();
        value
    }

    /// Puts records in the input, as if typed.
    pub(crate) fn type_in(&self, records: &[InputRecord]) {
        self.with(|state| state.records.extend(records.iter().copied()));
    }

    fn saved(&self) -> Saved {
        self.with(|state| Saved {
            input: state.input_mode,
            output: state.output_mode,
        })
    }

    /// Waits until what was written holds `needle`.
    fn wait_written(&self, needle: &[u8]) {
        let until = Instant::now() + LONG;
        let mut state = lock(&self.state);
        while !state
            .written
            .windows(needle.len().max(1))
            .any(|window| window == needle)
        {
            let left = until.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "{:?} was never written, only {:?}",
                String::from_utf8_lossy(needle),
                String::from_utf8_lossy(&state.written)
            );
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// The fake console as the Windows layer takes it.
#[derive(Debug, Clone)]
pub(crate) struct FakeConsole(pub(crate) Arc<Fake>);

#[derive(Debug)]
pub(crate) struct FakeOutput(Arc<Fake>);

impl Write for FakeOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.with(|state| state.written.extend_from_slice(bytes));
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct FakeInput(Arc<Fake>);

impl Wake for Fake {
    fn wake(&self) {
        self.with(|state| state.woken = true);
    }
}

impl Console for FakeConsole {
    type Input = FakeInput;
    type Output = FakeOutput;

    fn input_mode(&self) -> io::Result<u32> {
        Ok(self.0.with(|state| state.input_mode))
    }

    fn set_input_mode(&self, mode: u32) -> io::Result<()> {
        self.0.with(|state| {
            if state.input_fails {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            state.input_mode = mode;
            Ok(())
        })
    }

    fn output_mode(&self) -> io::Result<u32> {
        Ok(self.0.with(|state| state.output_mode))
    }

    fn set_output_mode(&self, mode: u32) -> io::Result<()> {
        self.0.with(|state| {
            if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 && !state.vt {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            state.output_mode = mode;
            Ok(())
        })
    }

    fn window(&self) -> io::Result<Window> {
        Ok(self.0.with(|state| state.window))
    }

    fn output(&self) -> io::Result<FakeOutput> {
        Ok(FakeOutput(self.0.clone()))
    }

    fn input(&self) -> io::Result<(FakeInput, Arc<dyn Wake>)> {
        Ok((FakeInput(self.0.clone()), self.0.clone()))
    }
}

impl Input for FakeInput {
    fn wait(&self, timeout: Option<Duration>) -> io::Result<Ready> {
        let until = timeout.map(|timeout| Instant::now() + timeout);
        let mut state = lock(&self.0.state);
        loop {
            if state.broken {
                return Err(io::Error::from(io::ErrorKind::BrokenPipe));
            }
            if !state.records.is_empty() {
                return Ok(Ready::Input);
            }
            if std::mem::take(&mut state.woken) {
                return Ok(Ready::Woken);
            }
            let left = until.map(|until| until.saturating_duration_since(Instant::now()));
            if left.is_some_and(|left| left.is_zero()) {
                return Ok(Ready::TimedOut);
            }
            state = self
                .0
                .changed
                .wait_timeout(state, left.unwrap_or(LONG))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn read(&self, records: &mut Vec<InputRecord>) -> io::Result<()> {
        self.0.with(|state| {
            if state.broken {
                return Err(io::Error::from(io::ErrorKind::BrokenPipe));
            }
            let count = state.records.len().min(READ_RECORDS);
            records.extend(state.records.drain(..count));
            Ok(())
        })
    }

    fn more(&self) -> io::Result<bool> {
        Ok(self.0.with(|state| !state.records.is_empty()))
    }

    fn window(&self) -> io::Result<Window> {
        Ok(self.0.with(|state| state.window))
    }
}

fn key_record(key_down: bool, virtual_key: u16, unicode: u16, control_state: u32) -> InputRecord {
    InputRecord::Key(KeyRecord {
        key_down,
        repeat: 1,
        virtual_key,
        scan_code: 0,
        unicode,
        control_state,
    })
}

/// The records a console in virtual terminal input mode gives for `text`, one key-down record a
/// UTF-16 unit.
fn vt_records(text: &str) -> Vec<InputRecord> {
    text.encode_utf16()
        .map(|unit| key_record(true, 0, unit, 0))
        .collect()
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn next(shared: &Shared) -> Event {
    match shared.events.next_timeout(LONG) {
        Next::Event(event) => event,
        other => panic!("no event came, but {other:?}"),
    }
}

fn nothing_more(shared: &Shared) {
    assert_eq!(
        shared.events.next_timeout(Duration::from_millis(100)),
        Next::Empty
    );
}

#[test]
fn the_window_gives_the_size_and_the_cells_the_program_draws() {
    let window = Window {
        left: 2,
        top: 100,
        right: 81,
        bottom: 123,
    };
    assert_eq!(window.size(), Size::new(80, 24));
    assert_eq!(window.relative(2, 100), (0, 0));
    assert_eq!(window.relative(10, 110), (8, 10));
    // Above or left of the window counts as its edge.
    assert_eq!(window.relative(0, 50), (0, 0));
    assert_eq!(Window::default().size(), Size::new(1, 1));
    let inverted = Window {
        left: 5,
        top: 5,
        right: 1,
        bottom: 1,
    };
    assert_eq!(inverted.size(), Size::ZERO);
}

/// Entering sets the modes a program owns the console with, and restoring puts back exactly what
/// was found.
#[test]
fn modes_are_set_and_given_back_exactly() {
    let fake = Fake::new();
    let before = fake.saved();
    let modes = Modes::enter(
        Arc::new(FakeConsole(fake.clone())),
        InputSettings::default(),
    )
    .unwrap();
    assert_eq!(modes.saved(), before);
    let set = fake.saved();
    assert_eq!(set.output, OUTPUT_MODE);
    // No line input, echo or processed input, so keys and Ctrl+C arrive as they're typed.
    assert_eq!(
        set.input,
        ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS | ENABLE_QUICK_EDIT_MODE
    );
    for off in [
        ENABLE_LINE_INPUT,
        ENABLE_ECHO_INPUT,
        ENABLE_PROCESSED_INPUT,
        ENABLE_MOUSE_INPUT,
    ] {
        assert_eq!(set.input & off, 0);
    }
    // With the mouse, quick edit goes, since it would take the clicks.
    modes
        .set_input(InputSettings {
            mode: InputMode::Records,
            mouse: true,
        })
        .unwrap();
    assert_eq!(
        fake.saved().input,
        ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS | ENABLE_MOUSE_INPUT
    );
    modes
        .set_input(InputSettings {
            mode: InputMode::Vt,
            mouse: true,
        })
        .unwrap();
    assert_ne!(fake.saved().input & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
    // Something else changed the modes, and applying puts the program's back.
    fake.with(|state| state.input_mode = before.input);
    modes.apply().unwrap();
    assert_ne!(fake.saved().input & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
    modes.restore().unwrap();
    assert_eq!(fake.saved(), before);
    // A console found without quick edit stays without it.
    fake.with(|state| state.input_mode = ENABLE_EXTENDED_FLAGS);
    let modes = Modes::enter(
        Arc::new(FakeConsole(fake.clone())),
        InputSettings::default(),
    )
    .unwrap();
    assert_eq!(
        fake.saved().input,
        ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS
    );
    modes.restore().unwrap();
}

#[test]
fn a_console_without_virtual_terminal_processing_is_left_as_it_was() {
    let fake = Fake::new();
    fake.with(|state| state.vt = false);
    let before = fake.saved();
    let error = Modes::enter(
        Arc::new(FakeConsole(fake.clone())),
        InputSettings::default(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(fake.saved(), before);
    // A failure later on puts back what changed too.
    let fake = Fake::new();
    fake.with(|state| state.input_fails = true);
    let before = fake.saved();
    assert!(
        Modes::enter(
            Arc::new(FakeConsole(fake.clone())),
            InputSettings::default()
        )
        .is_err()
    );
    assert_eq!(fake.saved(), before);
}

/// A reader on the fake console, decoding with the library's decoder.
fn reading(
    fake: &Arc<Fake>,
    settings: &Arc<Settings>,
    escape_timeout: Duration,
) -> (Reader, Arc<Shared>) {
    let shared = Arc::new(Shared::new(Size::new(80, 24)));
    let (input, wake) = FakeConsole(fake.clone()).input().unwrap();
    let reader = Reader::spawn(
        input,
        wake,
        Decoder::new(DecoderOptions::default()),
        shared.clone(),
        settings.clone(),
        escape_timeout,
    )
    .unwrap();
    (reader, shared)
}

/// Key records come out as keys, a window size record as the window's size (not the buffer's),
/// and mouse records at the window's cells.
#[test]
fn records_come_out_as_events() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    let (reader, shared) = reading(&fake, &settings, Duration::ZERO);
    assert!(shared.take_redraw());
    fake.type_in(&[
        key_record(true, 0x41, u16::from(b'a'), 0),
        key_record(false, 0x41, u16::from(b'a'), 0),
        key_record(true, 0x0d, 0x0d, 0x10),
    ]);
    assert_eq!(next(&shared), key(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(
        next(&shared),
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            crate::event::KeyEventKind::Release
        ))
    );
    assert_eq!(next(&shared), key(KeyCode::Enter, KeyModifiers::SHIFT));
    fake.with(|state| {
        state.window = Window {
            left: 0,
            top: 90,
            right: 99,
            bottom: 119,
        };
    });
    fake.type_in(&[
        InputRecord::WindowSize(WindowSizeRecord {
            size: Size::new(100, 9000),
        }),
        InputRecord::Mouse(MouseRecord {
            column: 5,
            row: 95,
            button_state: 1,
            control_state: 0,
            event_flags: 0,
        }),
    ]);
    assert_eq!(next(&shared), Event::Resize(Size::new(100, 30)));
    assert_eq!(shared.size(), Size::new(100, 30));
    assert!(shared.take_redraw());
    assert_eq!(
        next(&shared),
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        })
    );
    reader.stop();
    assert_eq!(shared.events.next_timeout(LONG), Next::Closed);
}

/// The console sends focus records whether or not the program asked, so they become events only
/// once it did.
#[test]
fn focus_records_count_only_once_asked_for() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    let (_reader, shared) = reading(&fake, &settings, Duration::ZERO);
    fake.type_in(&[InputRecord::Focus(FocusRecord { focused: false })]);
    nothing_more(&shared);
    settings.set_focus(true);
    fake.type_in(&[
        InputRecord::Focus(FocusRecord { focused: false }),
        InputRecord::Focus(FocusRecord { focused: true }),
    ]);
    assert_eq!(next(&shared), Event::FocusLost);
    assert_eq!(next(&shared), Event::FocusGained);
}

/// In virtual terminal input mode, the records' characters are bytes, so sequences decode as on
/// Unix, across records and reads, replies never become keys, and a lone ESC is Esc once no more
/// input is ready.
#[test]
fn vt_input_decodes_the_bytes_the_records_carry() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    settings.set_mode(InputMode::Vt);
    let (reader, shared) = reading(&fake, &settings, Duration::ZERO);
    fake.type_in(&vt_records("\x1b[1;5A"));
    assert_eq!(next(&shared), key(KeyCode::Up, KeyModifiers::CONTROL));
    // A paste, with a character outside the BMP as a surrogate pair, and key-up records between.
    let mut paste = vt_records("\x1b[200~a😀\rb\x1b[201~");
    paste.insert(3, key_record(false, 0, u16::from(b'['), 0));
    fake.type_in(&paste);
    assert_eq!(next(&shared), Event::Paste("a😀\nb".into()));
    // A reply to the probe decodes as one, which goes nowhere without a reply slot.
    fake.type_in(&vt_records("\x1b[?62;22c\x1b[?1u"));
    nothing_more(&shared);
    fake.type_in(&vt_records("\x1b"));
    assert_eq!(next(&shared), key(KeyCode::Esc, KeyModifiers::NONE));
    // Back in records mode, the records are keys again.
    settings.set_mode(InputMode::Records);
    fake.type_in(&[key_record(true, 0x0d, 0x0d, 0x10)]);
    assert_eq!(next(&shared), key(KeyCode::Enter, KeyModifiers::SHIFT));
    reader.stop();
}

/// With an escape timeout, a lone ESC waits for what follows it.
#[test]
fn vt_input_waits_the_escape_timeout() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    settings.set_mode(InputMode::Vt);
    let timeout = Duration::from_millis(200);
    let (_reader, shared) = reading(&fake, &settings, timeout);
    fake.type_in(&vt_records("\x1b"));
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&vt_records("x"));
    assert_eq!(next(&shared), key(KeyCode::Char('x'), KeyModifiers::ALT));
    let typed = Instant::now();
    fake.type_in(&vt_records("\x1b"));
    assert_eq!(next(&shared), key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(typed.elapsed() >= timeout);
}

/// A query the reader is told to expect makes its answer a reply, not a key.
#[test]
fn an_expected_cursor_report_is_no_key() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    settings.set_mode(InputMode::Vt);
    let (reader, shared) = reading(&fake, &settings, Duration::ZERO);
    fake.type_in(&vt_records("\x1b[1;5R"));
    assert_eq!(next(&shared), key(KeyCode::F(3), KeyModifiers::CONTROL));
    reader.expect(crate::event::decode::Query::CursorPosition);
    // (The reader takes the query at its next turn, which the wake starts.)
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&vt_records("\x1b[1;5R"));
    nothing_more(&shared);
    // A probe that stopped waiting cancels it, and the next report is the key again.
    reader.expect(crate::event::decode::Query::CursorPosition);
    reader.cancel_expected();
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&vt_records("\x1b[1;5R"));
    assert_eq!(next(&shared), key(KeyCode::F(3), KeyModifiers::CONTROL));
}

/// Replies go to the reply slot while the probe waits, and nowhere otherwise.
#[test]
fn replies_go_to_the_reply_slot_while_it_is_open() {
    use crate::event::decode::Reply;
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    settings.set_mode(InputMode::Vt);
    let (_reader, shared) = reading(&fake, &settings, Duration::ZERO);
    shared.replies.open();
    fake.type_in(&vt_records("\x1b[?1u\x1b[?62;22c"));
    assert!(shared.replies.wait(LONG, |replies| replies.len() == 2));
    assert_eq!(
        shared.replies.close(),
        [
            Reply::KittyKeyboard(1),
            Reply::Da1 {
                attributes: vec![62, 22]
            }
        ]
    );
    fake.type_in(&vt_records("\x1b[?62;22c"));
    nothing_more(&shared);
    assert!(shared.replies.close().is_empty());
}

/// A console whose input fails ends the events with the failure.
#[test]
fn a_console_that_fails_ends_the_events() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    let (reader, shared) = reading(&fake, &settings, Duration::ZERO);
    fake.with(|state| state.broken = true);
    assert_eq!(
        shared.events.next_timeout(LONG),
        Next::Failed(io::ErrorKind::BrokenPipe)
    );
    reader.stop();
}

#[test]
fn vt_bytes_are_the_characters_of_key_downs() {
    let mut surrogate = None;
    let mut bytes = Vec::new();
    let repeated = InputRecord::Key(KeyRecord {
        key_down: true,
        repeat: 3,
        virtual_key: 0,
        scan_code: 0,
        unicode: u16::from(b'z'),
        control_state: 0,
    });
    for record in vt_records("é😀")
        .into_iter()
        .chain([key_record(false, 0x41, u16::from(b'a'), 0), repeated])
        .chain([key_record(true, 0x10, 0, 0x10)])
    {
        let InputRecord::Key(key) = record else {
            unreachable!()
        };
        vt_bytes(key, &mut surrogate, &mut bytes);
    }
    assert_eq!(bytes, "é😀zzz".as_bytes());
    // A low surrogate with no high one before it is dropped.
    bytes.clear();
    vt_bytes(
        KeyRecord {
            key_down: true,
            repeat: 1,
            virtual_key: 0,
            scan_code: 0,
            unicode: 0xde00,
            control_state: 0,
        },
        &mut surrogate,
        &mut bytes,
    );
    assert!(bytes.is_empty());
}

fn enabled() -> Enabled {
    Enabled {
        alternate_screen: true,
        mouse: MouseMode::ButtonsAndDrags,
        focus: true,
        bracketed_paste: true,
        ..Enabled::default()
    }
}

/// Opening takes the console and sends nothing. Enabling sends the setup and takes the mouse.
/// Keys come out as events, and closing writes the reset and the tail and gives back the modes
/// exactly.
#[test]
fn a_screen_takes_the_console_and_gives_it_back_exactly() {
    let fake = Fake::new();
    let before = fake.saved();
    let screen = ScreenOn::open(
        FakeConsole(fake.clone()),
        |erase| {
            Decoder::new(DecoderOptions {
                erase,
                ..DecoderOptions::default()
            })
        },
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(screen.shared().size(), Size::new(80, 24));
    assert_eq!(screen.window_size(), Some(Size::new(80, 24)));
    assert_eq!(fake.saved().output, OUTPUT_MODE);
    assert!(fake.with(|state| state.written.is_empty()));
    screen.enable(&enabled()).unwrap();
    fake.wait_written(&enabled().setup());
    screen.draw_here();
    screen.writer().send(b"[frame]".to_vec()).unwrap();
    fake.wait_written(b"[frame]");
    assert_ne!(fake.saved().input & ENABLE_MOUSE_INPUT, 0);
    assert_eq!(fake.saved().input & ENABLE_QUICK_EDIT_MODE, 0);
    fake.type_in(&[
        key_record(true, 0x51, u16::from(b'q'), 0),
        InputRecord::Focus(FocusRecord { focused: false }),
    ]);
    assert_eq!(
        next(screen.shared()),
        key(KeyCode::Char('q'), KeyModifiers::NONE)
    );
    assert_eq!(next(screen.shared()), Event::FocusLost);
    screen.set_input(InputMode::Vt).unwrap();
    assert_ne!(fake.saved().input & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
    fake.type_in(&vt_records("\x1b[13;2u"));
    assert_eq!(
        next(screen.shared()),
        key(KeyCode::Enter, KeyModifiers::SHIFT)
    );
    // A lone ESC waits the escape timeout the program sets.
    screen.set_escape_timeout(Duration::from_millis(300));
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&vt_records("\x1b"));
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&vt_records("x"));
    assert_eq!(
        next(screen.shared()),
        key(KeyCode::Char('x'), KeyModifiers::ALT)
    );
    screen.expect(crate::event::decode::Query::CursorPosition);
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&vt_records("\x1b[1;5R"));
    nothing_more(screen.shared());
    screen.expect(crate::event::decode::Query::CursorPosition);
    screen.cancel_expected();
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&vt_records("\x1b[1;5R"));
    assert_eq!(
        next(screen.shared()),
        key(KeyCode::F(3), KeyModifiers::CONTROL)
    );
    screen.set_input(InputMode::Records).unwrap();
    assert_eq!(fake.saved().input & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
    assert_eq!(
        screen.suspend().unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    let shared = screen.shared().clone();
    let handle = screen.restore_handle();
    assert!(screen.close(b"[tail]", LONG));
    let mut reset = enabled().reset();
    reset.extend_from_slice(b"[tail]");
    fake.wait_written(&reset);
    assert_eq!(fake.saved(), before);
    assert!(handle.is_given_back());
    assert_eq!(shared.events.next_timeout(LONG), Next::Closed);
}

#[test]
fn a_screen_that_cant_open_leaves_the_console_as_it_was() {
    let fake = Fake::new();
    fake.with(|state| state.vt = false);
    let before = fake.saved();
    let opened = ScreenOn::open(
        FakeConsole(fake.clone()),
        |erase| {
            Decoder::new(DecoderOptions {
                erase,
                ..DecoderOptions::default()
            })
        },
        Duration::ZERO,
    );
    assert_eq!(opened.unwrap_err().kind(), io::ErrorKind::Unsupported);
    assert_eq!(fake.saved(), before);
}

/// UTF-16 units written somewhere, a few at a time.
#[derive(Debug, Default)]
struct Units {
    written: Vec<u16>,
    /// The most units a write takes, or none for zero.
    at_most: usize,
}

impl WriteUnits for Units {
    fn write_units(&mut self, units: &[u16]) -> io::Result<usize> {
        let taken = units.len().min(self.at_most);
        self.written.extend_from_slice(&units[..taken]);
        Ok(taken)
    }
}

/// The writer's UTF-8 reaches the console as UTF-16, whole when a write ends inside a character,
/// with what isn't UTF-8 as U+FFFD, however few units the console takes at a time.
#[test]
fn utf8_reaches_the_console_as_utf16() {
    let text = "é日😀 \x1b[1;31mred\x1b[0m";
    for at_most in [1, 3, 1024] {
        for piece in [1, 2, 5, text.len()] {
            let mut writer = Utf16Writer::new(Units {
                at_most,
                ..Units::default()
            });
            for bytes in text.as_bytes().chunks(piece) {
                assert_eq!(writer.write(bytes).unwrap(), bytes.len());
            }
            writer.flush().unwrap();
            assert_eq!(
                String::from_utf16(&writer.out.written).unwrap(),
                text,
                "{at_most} units a write, {piece} bytes at a time"
            );
        }
    }
    let mut writer = Utf16Writer::new(Units {
        at_most: 64,
        ..Units::default()
    });
    writer.write_all(b"a\xffb\xe2\x28").unwrap();
    assert_eq!(
        String::from_utf16(&writer.out.written).unwrap(),
        "a\u{fffd}b\u{fffd}("
    );
    let mut stuck = Utf16Writer::new(Units::default());
    assert_eq!(
        stuck.write(b"x").unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
}

/// The records a console gives for a reply it put in the input buffer, a key-down and a key-up
/// record for each character, the ESC with `VK_ESCAPE`, the rest with `virtual_key`.
fn reply_records(reply: &str, virtual_key: u16) -> Vec<InputRecord> {
    reply
        .encode_utf16()
        .flat_map(|unit| {
            let vk = if unit == 0x1b { 0x1b } else { virtual_key };
            [
                key_record(true, vk, unit, 0),
                key_record(false, vk, unit, 0),
            ]
        })
        .collect()
}

/// In records mode, a reply that comes after the probe stopped waiting decodes as a reply and is
/// dropped, never as keys, whether its ESC comes with `VK_ESCAPE` or with no virtual key, and
/// across reads. A real Esc press, alone in its batch, is Esc.
#[test]
fn late_replies_in_records_mode_never_become_keys() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    let (_reader, shared) = reading(&fake, &settings, Duration::ZERO);
    for virtual_key in [0, 0x41] {
        fake.type_in(&reply_records("\x1b[?62;22c", virtual_key));
        nothing_more(&shared);
    }
    // With no virtual key, the ESC too.
    fake.type_in(&vt_records("\x1b]11;rgb:0000/0000/0000\x1b\\"));
    nothing_more(&shared);
    // Split across two reads.
    fake.type_in(&reply_records("\x1b[?6", 0x41));
    std::thread::sleep(Duration::from_millis(50));
    fake.type_in(&reply_records("2;1c", 0x41));
    nothing_more(&shared);
    // A real Esc, its key-up in a later batch.
    fake.type_in(&[key_record(true, 0x1b, 0x1b, 0)]);
    assert_eq!(next(&shared), key(KeyCode::Esc, KeyModifiers::NONE));
    fake.type_in(&[key_record(false, 0x1b, 0x1b, 0)]);
    assert!(matches!(next(&shared), Event::Key(up) if up.code == KeyCode::Esc));
    fake.type_in(&[key_record(true, 0x41, u16::from(b'a'), 0)]);
    assert_eq!(next(&shared), key(KeyCode::Char('a'), KeyModifiers::NONE));
}

/// Keys typed while the probe ran in virtual terminal input mode stay as character records with
/// no virtual key, and decode as the same keys once the reader is back in records mode.
#[test]
fn keys_typed_during_the_probe_are_the_same_keys_afterwards() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    let (_reader, shared) = reading(&fake, &settings, Duration::ZERO);
    fake.type_in(&vt_records("x\x1b[A\x1b[13;2u\x03é"));
    assert_eq!(next(&shared), key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(next(&shared), key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(next(&shared), key(KeyCode::Enter, KeyModifiers::SHIFT));
    assert_eq!(
        next(&shared),
        key(KeyCode::Char('c'), KeyModifiers::CONTROL)
    );
    assert_eq!(next(&shared), key(KeyCode::Char('é'), KeyModifiers::NONE));
    // And keys typed after are records again.
    fake.type_in(&[key_record(true, 0x0d, 0x0d, 0x10)]);
    assert_eq!(next(&shared), key(KeyCode::Enter, KeyModifiers::SHIFT));
}

/// A sequence cut short, then silence, is given up on, so what's typed next is itself.
#[test]
fn a_reply_cut_short_is_given_up_on_after_silence() {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    let (_reader, shared) = reading(&fake, &settings, Duration::ZERO);
    fake.type_in(&reply_records("\x1b]11;rgb:00", 0));
    std::thread::sleep(Duration::from_millis(1300));
    fake.type_in(&[key_record(true, 0x41, u16::from(b'a'), 0)]);
    assert_eq!(next(&shared), key(KeyCode::Char('a'), KeyModifiers::NONE));
}

/// The kitty keyboard flags go to the console only while keys come as bytes, since under a
/// pseudoconsole the push reaches the terminal, which would send keys a client reading records
/// can't take.
#[test]
fn kitty_flags_are_pushed_only_while_keys_come_as_bytes() {
    let fake = Fake::new();
    let screen = ScreenOn::open(
        FakeConsole(fake.clone()),
        |erase| {
            Decoder::new(DecoderOptions {
                erase,
                ..DecoderOptions::default()
            })
        },
        Duration::ZERO,
    )
    .unwrap();
    let kitty = Enabled {
        keyboard: Some(1),
        ..enabled()
    };
    screen.enable(&kitty).unwrap();
    let records = Enabled {
        keyboard: None,
        ..kitty.clone()
    };
    fake.wait_written(&records.setup());
    screen.set_input(InputMode::Vt).unwrap();
    fake.wait_written(b"\x1b[>1u");
    screen.set_input(InputMode::Vt).unwrap();
    screen.set_input(InputMode::Records).unwrap();
    fake.wait_written(b"\x1b[<1u");
    assert!(screen.close(b"", LONG));
    fake.wait_written(&records.reset());
    let written = fake.with(|state| state.written.clone());
    let count = |needle: &[u8]| {
        written
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    };
    assert_eq!(count(b"\x1b[>1u"), 1);
    assert_eq!(count(b"\x1b[<1u"), 1);
}

/// A console control event gives the console back once the program asked for that, and leaves
/// the rest to the system.
#[test]
fn a_console_control_event_gives_the_console_back() {
    let fake = Fake::new();
    let before = fake.saved();
    let screen = ScreenOn::open(
        FakeConsole(fake.clone()),
        |erase| {
            Decoder::new(DecoderOptions {
                erase,
                ..DecoderOptions::default()
            })
        },
        Duration::ZERO,
    )
    .unwrap();
    screen.enable(&enabled()).unwrap();
    // (Not asked for, it does nothing.)
    assert_eq!(on_console_event(2), 0);
    assert!(!screen.restore_handle().is_given_back());
    screen.restore_on_console_events().unwrap();
    assert_eq!(on_console_event(2), 0);
    assert!(screen.restore_handle().is_given_back());
    assert_eq!(fake.saved(), before);
    fake.wait_written(&enabled().reset());
    assert!(screen.close(b"", LONG));
}

/// The tests that need a real console, for the spikes that settle how Windows consoles behave.
/// They run on a Windows runner with `cargo test -- --ignored`, under conhost and under Windows
/// Terminal, and with the system's pseudoconsole and the one Windows Terminal ships (spike S8).
#[cfg(windows)]
mod real {
    use super::*;
    use crate::sys::windows::WinConsole;
    use crate::sys::windows::console::testing::{read_text, write_input};
    use crate::sys::windows::screen::Screen;
    use std::fs::OpenOptions;

    fn conin() -> std::fs::File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONIN$")
            .unwrap()
    }

    fn conout() -> std::fs::File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONOUT$")
            .unwrap()
    }

    /// The modes and code pages as the console has them now.
    fn found(console: &WinConsole) -> (Saved, u32, u32) {
        let saved = Saved {
            input: console.input_mode().unwrap(),
            output: console.output_mode().unwrap(),
        };
        let (input, output) = super::super::console::testing::code_pages().unwrap();
        (saved, input, output)
    }

    fn decoder(erase: u8) -> Decoder {
        Decoder::new(DecoderOptions {
            erase,
            ..DecoderOptions::default()
        })
    }

    /// One program on a real console. Shift+Enter, a click and a focus change put into the input
    /// buffer come out as events, and closing gives the console back as it was found, its code
    /// pages untouched (spike S3, in process).
    #[test]
    #[ignore = "needs a real console"]
    fn a_real_console_is_taken_and_given_back() {
        let probe = WinConsole::open().unwrap();
        let before = found(&probe);
        let screen = Screen::open(WinConsole::open().unwrap(), decoder, Duration::ZERO).unwrap();
        screen.enable(&enabled()).unwrap();
        let shift = 0x10;
        let top = u16::try_from(probe.window().unwrap().top).unwrap_or(0);
        write_input(
            &conin(),
            &[
                key_record(true, 0x0d, 0x0d, shift),
                key_record(false, 0x0d, 0x0d, shift),
                InputRecord::Mouse(MouseRecord {
                    column: 3,
                    row: top,
                    button_state: 1,
                    control_state: 0,
                    event_flags: 0,
                }),
                InputRecord::Focus(FocusRecord { focused: true }),
            ],
        )
        .unwrap();
        assert_eq!(
            next(screen.shared()),
            key(KeyCode::Enter, KeyModifiers::SHIFT)
        );
        assert!(
            matches!(next(screen.shared()), Event::Key(release) if release.code == KeyCode::Enter)
        );
        assert_eq!(
            next(screen.shared()),
            Event::Mouse(MouseEvent::new(
                MouseEventKind::Down(MouseButton::Left),
                3,
                0,
                KeyModifiers::NONE
            ))
        );
        assert_eq!(next(screen.shared()), Event::FocusGained);
        assert!(screen.close(b"", LONG));
        assert_eq!(found(&probe), before);
    }

    /// `Terminal` on a real console. It probes in virtual terminal input mode, reads keys as
    /// records afterwards, draws, and gives the console back as it was found.
    #[test]
    #[ignore = "needs a real console"]
    fn a_terminal_opens_draws_and_closes_on_a_real_console() {
        use crate::caps::KeyboardSupport;
        use crate::terminal::{Options, Terminal};
        let probe = WinConsole::open().unwrap();
        let before = found(&probe);
        let mut terminal = Terminal::open(Options::default()).unwrap();
        assert_eq!(
            terminal.capabilities().keyboard,
            KeyboardSupport::ConsoleRecords
        );
        assert_eq!(terminal.size(), probe.window().unwrap().size());
        let drawn = terminal
            .draw(|frame| {
                frame
                    .buffer_mut()
                    .set_string(0, 0, "tuiframe", crate::Style::new());
            })
            .unwrap();
        assert!(matches!(drawn, crate::Drawn::Written(_)));
        terminal.restore_on_console_events().unwrap();
        write_input(&conin(), &[key_record(true, 0x0d, 0x0d, 0x10)]).unwrap();
        let mut events = terminal.events();
        assert_eq!(
            events.next_timeout(LONG).unwrap(),
            Some(key(KeyCode::Enter, KeyModifiers::SHIFT))
        );
        terminal.close().unwrap();
        assert_eq!(found(&probe), before);
    }

    /// UTF-8 the writer gets in pieces that end inside characters reaches the screen whole, as
    /// UTF-16 through `WriteConsoleW`, whatever the code page.
    #[test]
    #[ignore = "needs a real console"]
    fn split_utf8_reaches_the_screen_whole() {
        let console = WinConsole::open().unwrap();
        let modes = Modes::enter(
            Arc::new(WinConsole::open().unwrap()),
            InputSettings::default(),
        )
        .unwrap();
        let window = console.window().unwrap();
        let row = i32::from(window.bottom) - i32::from(window.top) + 1;
        let mut out = console.output().unwrap();
        out.write_all(format!("\r\n\x1b[{row};1H\x1b[2K").as_bytes())
            .unwrap();
        for piece in "é日😀x".as_bytes().chunks(1) {
            out.write_all(piece).unwrap();
        }
        let shown = read_text(&conout(), 0, window.bottom, 8).unwrap();
        out.write_all(b"\r\x1b[2K").unwrap();
        modes.restore().unwrap();
        assert!(shown.starts_with("é日"), "{shown:?}");
        assert!(shown.contains('😀') && shown.contains('x'), "{shown:?}");
    }

    /// Spike S2: what the console delivers for the probe's queries in each input mode. In virtual
    /// terminal input mode the replies decode as replies, and in records mode they come as
    /// records the reader takes for bytes, so none becomes a key. It prints how each reply's ESC
    /// came (its virtual key) for the spike's record.
    #[test]
    #[ignore = "needs a real console"]
    fn the_probe_s_replies_never_become_keys() {
        use crate::event::decode::{Decoded, Reply};
        for mode in [InputMode::Vt, InputMode::Records] {
            let console = WinConsole::open().unwrap();
            let modes = Modes::enter(
                Arc::new(WinConsole::open().unwrap()),
                InputSettings { mode, mouse: false },
            )
            .unwrap();
            let (input, _wake) = console.input().unwrap();
            conout()
                .write_all(b"\x1b[?2026$p\x1b[>q\x1b[6n\x1b]11;?\x1b\\\x1b[?u\x1b[c")
                .unwrap();
            let mut decoder = decoder(0x7f);
            let (mut records, mut bytes, mut decoded, mut surrogate) =
                (Vec::new(), Vec::new(), Vec::new(), None);
            let mut escapes = Vec::new();
            let until = Instant::now() + LONG;
            decoder.expect(crate::event::decode::Query::CursorPosition);
            while !decoded
                .iter()
                .any(|d| matches!(d, Decoded::Reply(Reply::Da1 { .. })))
                && Instant::now() < until
            {
                if input.wait(Some(Duration::from_millis(100))).unwrap() != Ready::Input {
                    continue;
                }
                records.clear();
                input.read(&mut records).unwrap();
                for record in &records {
                    if let InputRecord::Key(key) = *record {
                        if key.unicode == 0x1b && key.key_down {
                            escapes.push(key.virtual_key);
                        }
                        bytes.clear();
                        vt_bytes(key, &mut surrogate, &mut bytes);
                        decoder.feed(&bytes, &mut decoded);
                    }
                }
            }
            modes.restore().unwrap();
            println!("{mode:?}: replies {decoded:?}, their ESCs' virtual keys {escapes:x?}");
            assert!(
                decoded.iter().all(|d| matches!(d, Decoded::Reply(_))),
                "{mode:?}: {decoded:?}"
            );
        }
    }
}
