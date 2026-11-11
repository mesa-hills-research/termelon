//! The terminal end to end, on a pty whose other side the emulator plays as each terminal: the
//! probe's results, what open turns on, keys in, a resize, and close.

use std::fs::File;
use std::io::{PipeWriter, Read, Write};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tuiframe_vt::{Emulator, Key, Mods, Profile};

use super::{Drawn, Error, MouseMode, Options, Terminal};
use crate::caps::env::Vars;
use crate::caps::{Capabilities, ColorDepth, GlyphSupport, KeyboardSupport, Multiplexer, Support};
use crate::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crate::layout::Size;
use crate::output::writer::lock;
use crate::sys::unix::tests::{LONG, Pty, find};
use crate::sys::unix::{Signals, Tty, wait};

/// A terminal the emulator plays on a pty's master side. It answers what the program asks, and
/// keeps what the program draws.
struct Played {
    pty: Pty,
    emulator: Arc<Mutex<Emulator>>,
    /// The sequences the emulator didn't know, which no test expects.
    unknown: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    resize: PipeWriter,
    _resume: PipeWriter,
}

impl Played {
    fn new(profile: Profile) -> (Self, Signals) {
        let pty = Pty::open(Size::new(80, 24));
        let emulator = Arc::new(Mutex::new(Emulator::with_profile(profile, 80, 24)));
        let unknown = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (mut master, emulator, unknown, stop) = (
                pty.master.try_clone().unwrap(),
                emulator.clone(),
                unknown.clone(),
                stop.clone(),
            );
            std::thread::spawn(move || {
                let mut buffer = [0; 4096];
                while !stop.load(Ordering::SeqCst) {
                    if !matches!(
                        wait([master.as_fd()], Some(Duration::from_millis(5))),
                        Ok([true])
                    ) {
                        continue;
                    }
                    let Ok(read) = master.read(&mut buffer) else {
                        return;
                    };
                    let answer = {
                        let mut emulator = lock(&emulator);
                        if let Err(error) = emulator.feed(&buffer[..read]) {
                            lock(&unknown).push(format!("{:?}", error.sequences()));
                        }
                        emulator.take_output()
                    };
                    if master.write_all(&answer).is_err() {
                        return;
                    }
                }
            })
        };
        let (signals, resize, resume) = Signals::unregistered().unwrap();
        let played = Self {
            pty,
            emulator,
            unknown,
            stop,
            thread: Some(thread),
            resize,
            _resume: resume,
        };
        (played, signals)
    }

    fn tty(&self) -> Tty {
        self.pty.tty()
    }

    /// Types `bytes` as the terminal sends them.
    fn send(&mut self, bytes: &[u8]) {
        self.pty.master.write_all(bytes).unwrap();
    }

    fn key(&mut self, key: Key, mods: Mods) {
        let bytes = lock(&self.emulator).encode_key(key, mods);
        self.send(&bytes);
    }

    /// Waits until the emulator shows what `done` looks for.
    fn until(&self, what: &str, done: impl Fn(&Emulator) -> bool) {
        let started = Instant::now();
        while !done(&lock(&self.emulator)) {
            assert!(started.elapsed() < LONG, "never {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Closes the terminal's side, as a terminal window that closes does.
    fn hang_up(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let master = std::mem::replace(&mut self.pty.master, File::open("/dev/null").unwrap());
        drop(master);
    }

    fn resize(&mut self, size: Size) {
        lock(&self.emulator).resize(size.width, size.height);
        self.pty.resize(size);
        self.resize.write_all(b"X").unwrap();
    }
}

impl Drop for Played {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Opens a terminal on `played` with `options`, the variables `vars` and `TUIFRAME_CAPS` unset.
fn open(
    played: &Played,
    signals: Signals,
    vars: &'static [(&'static str, &'static str)],
    options: &Options,
) -> Terminal {
    Terminal::open_on(played.tty(), options.clone(), &Vars(vars), signals).unwrap()
}

fn rung(emulator: &Emulator) -> bool {
    emulator
        .events()
        .iter()
        .any(|event| matches!(event, tuiframe_vt::Event::Bell))
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn next(terminal: &Terminal) -> Event {
    terminal
        .events()
        .next_timeout(LONG)
        .unwrap()
        .expect("an event")
}

/// The probe, open, keys, a resize and close in one terminal, with what `check` expects of the
/// capabilities.
fn end_to_end(
    profile: Profile,
    vars: &'static [(&'static str, &'static str)],
    check: impl Fn(&Capabilities, &Emulator),
) {
    let name = profile.name.clone();
    let (mut played, signals) = Played::new(profile);
    let before = played.pty.modes();
    let mut terminal = open(&played, signals, vars, &Options::default());
    assert!(played.pty.is_raw(), "{name}");
    let caps = terminal.capabilities().clone();
    // (The bell goes out after the setup.)
    terminal.bell().unwrap();
    played.until("set up", rung);
    check(&caps, &lock(&played.emulator));
    assert_eq!(terminal.size(), Size::new(80, 24));
    // Nothing the probe asked shows on the screen.
    assert!(
        lock(&played.emulator).rows().iter().all(String::is_empty),
        "{name}: {:?}",
        lock(&played.emulator).rows()
    );

    played.key(Key::Char('a'), Mods::NONE);
    assert_eq!(next(&terminal), key(KeyCode::Char('a'), KeyModifiers::NONE));
    played.key(Key::Char('c'), Mods::CTRL);
    assert_eq!(
        next(&terminal),
        key(KeyCode::Char('c'), KeyModifiers::CONTROL)
    );
    played.resize(Size::new(100, 30));
    assert_eq!(next(&terminal), Event::Resize(Size::new(100, 30)));
    assert_eq!(terminal.size(), Size::new(100, 30));

    terminal.close().unwrap();
    played.until("given back", |emulator| emulator.mode(25));
    let emulator = lock(&played.emulator);
    assert!(!emulator.alternate_screen(), "{name}");
    assert_eq!(emulator.keyboard_flags(), 0, "{name}");
    assert_eq!(emulator.modify_other_keys(), 0, "{name}");
    for mode in [1000, 1002, 1003, 1004, 1006, 2004, 2027] {
        assert!(!emulator.mode(mode), "{name}: mode {mode} is still on");
    }
    assert!(emulator.title_stack().is_empty(), "{name}");
    drop(emulator);
    assert_eq!(played.pty.modes(), before, "{name}");
    assert!(
        lock(&played.unknown).is_empty(),
        "{name}: {:?}",
        lock(&played.unknown)
    );
}

#[test]
fn kitty_end_to_end() {
    end_to_end(
        Profile::kitty(),
        &[("TERM", "xterm-kitty")],
        |caps, emulator| {
            assert_eq!(caps.terminal.name.as_deref(), Some("kitty"));
            assert_eq!(caps.terminal.version.as_deref(), Some("0.32.2"));
            assert_eq!(caps.keyboard, KeyboardSupport::Kitty { flags: 0b1_1111 });
            assert_eq!(caps.synchronized_output, Support::Yes);
            assert_eq!(caps.colors, ColorDepth::TrueColor);
            assert_eq!(caps.background, Some(crate::caps::Rgb::new(0, 0, 0)));
            assert!(emulator.alternate_screen());
            assert_eq!(emulator.keyboard_flags(), 0b101);
            assert_eq!(emulator.modify_other_keys(), 0);
            // (Mode 1002 takes over from 1000, as in xterm, which tracks one mouse mode.)
            for mode in [1002, 1006, 1004, 2004] {
                assert!(emulator.mode(mode), "mode {mode}");
            }
            assert!(!emulator.mode(7), "autowrap is off");
            assert_eq!(emulator.title_stack().len(), 1);
        },
    );
}

#[test]
fn xterm_end_to_end() {
    end_to_end(
        Profile::xterm(),
        &[("TERM", "xterm-256color")],
        |caps, emulator| {
            assert_eq!(caps.terminal.name.as_deref(), Some("XTerm"));
            assert_eq!(caps.keyboard, KeyboardSupport::ModifyOtherKeys);
            assert_eq!(caps.synchronized_output, Support::No);
            assert_eq!(caps.hyperlinks, Support::No);
            assert!(emulator.alternate_screen());
            assert_eq!(emulator.keyboard_flags(), 0);
            assert_eq!(emulator.modify_other_keys(), 1);
            assert_eq!(emulator.title_stack().len(), 1);
        },
    );
}

#[test]
fn konsole_end_to_end() {
    end_to_end(
        Profile::konsole(),
        &[("TERM", "xterm-256color"), ("KONSOLE_VERSION", "230805")],
        |caps, emulator| {
            assert_eq!(caps.terminal.name.as_deref(), Some("Konsole"));
            assert_eq!(caps.synchronized_output, Support::Unknown);
            assert_eq!(caps.keyboard, KeyboardSupport::Legacy);
            assert!(emulator.alternate_screen());
            assert!(emulator.mode(2004));
        },
    );
}

#[test]
fn windows_terminal_end_to_end() {
    end_to_end(
        Profile::windows_terminal(),
        &[("WT_SESSION", "1")],
        |caps, emulator| {
            assert_eq!(caps.terminal.name.as_deref(), Some("Windows Terminal"));
            assert_eq!(caps.grapheme_clustering, Support::Yes);
            assert!(caps.width_profile.joins_emoji);
            assert!(matches!(caps.keyboard, KeyboardSupport::Kitty { .. }));
            assert!(emulator.mode(2027));
            assert_eq!(emulator.keyboard_flags(), 0b101);
        },
    );
}

#[test]
fn tmux_end_to_end() {
    end_to_end(
        Profile::tmux_3_4(),
        &[
            ("TERM", "tmux-256color"),
            ("TMUX", "/tmp/tmux-1/default,1,0"),
        ],
        |caps, emulator| {
            assert_eq!(
                caps.multiplexer,
                Some(Multiplexer::Tmux {
                    version: Some("3.4".to_owned())
                })
            );
            assert_eq!(caps.synchronized_output, Support::No);
            assert_eq!(caps.colors, ColorDepth::Ansi256);
            assert!(emulator.alternate_screen());
        },
    );
}

/// The Linux console gets DA1 alone, so no string prints on its screen, and with no alternate
/// screen the program's screen is cleared on the way in and out.
#[test]
fn linux_console_end_to_end() {
    end_to_end(
        Profile::linux_console(),
        &[("TERM", "linux")],
        |caps, emulator| {
            assert_eq!(caps.alternate_screen, Support::No);
            assert_eq!(caps.glyphs, GlyphSupport::Limited);
            assert_eq!(caps.colors, ColorDepth::Ansi16);
            assert!(!emulator.alternate_screen());
            assert!(!emulator.mode(1000));
        },
    );
}

/// A terminal that answers nothing makes the probe give up at its timeout, with nothing known,
/// and its replies when they come late never reach the program as keys.
#[test]
fn a_silent_terminal_times_the_probe_out() {
    let (mut played, signals) = Played::new(Profile::silent());
    let options = Options {
        probe_timeout: Some(Duration::from_millis(200)),
        ..Options::default()
    };
    let started = Instant::now();
    let terminal = open(&played, signals, &[], &options);
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(200), "{took:?}");
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert_eq!(
        terminal.capabilities(),
        &Capabilities {
            probe: crate::caps::Probe::TimedOut,
            ..Capabilities::default()
        }
    );
    played.send(
        b"\x1b[?62;c\x1b[?2026;2$y\x1b[?0u\x1bP>|kitty(0.32.2)\x1b\\\x1b]11;rgb:0000/0000/0000\x1b\\",
    );
    played.key(Key::Char('x'), Mods::NONE);
    assert_eq!(next(&terminal), key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(
        terminal
            .events()
            .next_timeout(Duration::from_millis(50))
            .unwrap(),
        None
    );
    terminal.close().unwrap();
}

/// The title, the bell, a copy and the mouse go out where the terminal takes them, in order.
#[test]
fn out_of_band_commands_reach_the_terminal() {
    let (played, signals) = Played::new(Profile::kitty());
    let mut terminal = open(&played, signals, &[], &Options::default());
    terminal.set_title("proj \u{7}· arko").unwrap();
    terminal.copy_to_clipboard("copied ✓").unwrap();
    terminal.set_mouse(MouseMode::AnyMotion).unwrap();
    terminal.bell().unwrap();
    played.until("rung", |emulator| {
        emulator
            .events()
            .iter()
            .any(|event| matches!(event, tuiframe_vt::Event::Bell))
    });
    {
        let emulator = lock(&played.emulator);
        assert_eq!(emulator.title(), "proj · arko");
        let copies = emulator.clipboard();
        assert_eq!(
            copies.last().map(|copy| copy.data.as_slice()),
            Some("copied ✓".as_bytes())
        );
        assert!(emulator.mode(1003));
    }
    terminal.set_mouse(MouseMode::Off).unwrap();
    played.until("mouse off", |emulator| !emulator.mode(1000));
    terminal.close().unwrap();
    played.until("given back", |emulator| emulator.mode(25));
    assert!(
        lock(&played.unknown).is_empty(),
        "{:?}",
        lock(&played.unknown)
    );
}

/// A terminal that doesn't take the title or a copy gets neither.
#[test]
fn strings_go_only_where_they_are_known() {
    let (played, signals) = Played::new(Profile::xterm());
    let options = Options {
        capabilities: Some(Capabilities::default()),
        ..Options::default()
    };
    let mut terminal = open(&played, signals, &[], &options);
    terminal.set_title("not sent").unwrap();
    terminal.copy_to_clipboard("not sent").unwrap();
    terminal.bell().unwrap();
    played.until("rung", |emulator| {
        emulator
            .events()
            .iter()
            .any(|event| matches!(event, tuiframe_vt::Event::Bell))
    });
    assert_eq!(lock(&played.emulator).title(), "");
    assert!(lock(&played.emulator).clipboard().is_empty());
    terminal.close().unwrap();
}

/// The width probes measure how the terminal draws, and clear what they drew.
#[test]
fn width_probes_measure_the_terminal() {
    let (played, signals) = Played::new(Profile::kitty());
    let options = Options {
        width_probes: true,
        ..Options::default()
    };
    let terminal = open(&played, signals, &[], &options);
    let profile = terminal.capabilities().width_profile;
    assert!(profile.joins_emoji && profile.vs16_widens && !profile.ambiguous_wide);
    assert!(lock(&played.emulator).rows().iter().all(String::is_empty));
    terminal.close().unwrap();
    assert!(
        lock(&played.unknown).is_empty(),
        "{:?}",
        lock(&played.unknown)
    );
}

/// When the terminal stops taking output, the events end and `failure` says why, while the
/// terminal's input is still open.
#[test]
fn a_writer_that_fails_ends_the_events() {
    let pty = Pty::open(Size::new(80, 24));
    let (read, write) = std::io::pipe().unwrap();
    drop(read);
    let tty = Tty {
        input: pty.slave.try_clone().unwrap(),
        output: write.into(),
    };
    let (signals, _resize, _resume) = Signals::unregistered().unwrap();
    let options = Options {
        capabilities: Some(Capabilities::default()),
        ..Options::default()
    };
    let mut terminal = Terminal::open_on(tty, options, &Vars(&[]), signals).unwrap();
    assert!(matches!(
        terminal.events().next_timeout(LONG),
        Err(Error::Closed)
    ));
    assert_eq!(terminal.failure(), Some(std::io::ErrorKind::BrokenPipe));
    assert!(matches!(terminal.bell(), Err(Error::Io(_))));
}

/// A terminal that closes ends the events.
#[test]
fn a_terminal_that_closes_ends_the_events() {
    let (mut played, signals) = Played::new(Profile::kitty());
    let terminal = open(&played, signals, &[], &Options::default());
    played.hang_up();
    assert!(matches!(
        terminal.events().next_timeout(LONG),
        Err(Error::Closed)
    ));
}

/// A frame draws its cells and puts the cursor where it asks, one that changes nothing writes
/// nothing, and a resize redraws everything at the new size.
#[test]
fn frames_reach_the_terminal() {
    let (mut played, signals) = Played::new(Profile::kitty());
    let mut terminal = open(&played, signals, &[], &Options::default());
    let drawn = terminal
        .draw(|frame| {
            frame
                .buffer_mut()
                .set_string(0, 0, "hello", crate::style::Style::new());
            frame.set_cursor_position((5, 0));
        })
        .unwrap();
    let Drawn::Written(completed) = drawn else {
        panic!("the first frame was held");
    };
    assert_eq!(completed.buffer().symbol((0, 0)), "h");
    assert!(completed.stats().bytes > 0);
    played.until("drawn", |emulator| {
        emulator.row_text(0).as_deref() == Some("hello") && emulator.cursor().visible
    });
    let cursor = lock(&played.emulator).cursor();
    assert_eq!((cursor.x, cursor.y), (5, 0));

    wait_until_written(&terminal);
    let same = terminal.draw(|frame| {
        frame
            .buffer_mut()
            .set_string(0, 0, "hello", crate::style::Style::new());
        frame.set_cursor_position((5, 0));
    });
    assert!(matches!(same, Ok(Drawn::Written(completed)) if completed.stats().bytes == 0));

    played.resize(Size::new(100, 30));
    assert_eq!(next(&terminal), Event::Resize(Size::new(100, 30)));
    wait_until_written(&terminal);
    let resized = terminal.draw(|frame| {
        assert_eq!(frame.area(), crate::layout::Rect::new(0, 0, 100, 30));
        frame
            .buffer_mut()
            .set_string(90, 29, "corner", crate::style::Style::new());
    });
    assert!(matches!(resized, Ok(Drawn::Written(_))));
    played.until("redrawn", |emulator| {
        emulator
            .row_text(29)
            .as_deref()
            .is_some_and(|row| row.ends_with("corner"))
            && emulator.row_text(0).as_deref() == Some("")
            && !emulator.cursor().visible
    });
    terminal.close().unwrap();
    assert!(
        lock(&played.unknown).is_empty(),
        "{:?}",
        lock(&played.unknown)
    );
}

fn wait_until_written(terminal: &Terminal) {
    assert!(crate::output::writer::tests::block_on(
        terminal.written(),
        LONG
    ));
}

/// While the terminal hasn't taken the last packet, `draw` holds the frame without drawing it,
/// `written` completes once it has, and the full redraw asked for meanwhile survives the held
/// frames.
#[test]
fn frames_wait_for_the_terminal() {
    use crate::output::writer::tests::{block_on, full_pipe};
    let pty = Pty::open(Size::new(80, 24));
    let (mut read, write, filled) = full_pipe();
    let tty = Tty {
        input: pty.slave.try_clone().unwrap(),
        output: write.into(),
    };
    let (signals, _resize, _resume) = Signals::unregistered().unwrap();
    let options = Options {
        capabilities: Some(Capabilities::default()),
        ..Options::default()
    };
    let mut terminal = Terminal::open_on(tty, options, &Vars(&[]), signals).unwrap();
    let mut drawn = false;
    assert!(matches!(terminal.draw(|_| drawn = true), Ok(Drawn::Held)));
    assert!(!drawn);
    assert!(!block_on(terminal.written(), Duration::from_millis(50)));

    // The terminal reads again.
    let shown = Arc::new(Mutex::new(Vec::new()));
    let reading = {
        let shown = shown.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = read.read(&mut buffer) {
                if count == 0 {
                    return;
                }
                lock(&shown).extend_from_slice(&buffer[..count]);
            }
        })
    };
    assert!(block_on(terminal.written(), LONG));
    let draw = |terminal: &mut Terminal, text: &'static str| {
        let drawn = terminal.draw(move |frame| {
            frame
                .buffer_mut()
                .set_string(0, 0, text, crate::style::Style::new());
        });
        assert!(matches!(drawn, Ok(Drawn::Written(_))));
        assert!(block_on(terminal.written(), LONG));
    };
    // What the terminal read since the last call, once it holds `needle`.
    let take_until = |needle: &[u8]| {
        let started = Instant::now();
        loop {
            {
                let mut shown = lock(&shown);
                if find(&shown, needle).is_some() {
                    return shown.split_off(0);
                }
            }
            assert!(
                started.elapsed() < LONG,
                "{:?} never came, after {:?}",
                String::from_utf8_lossy(needle),
                String::from_utf8_lossy(&lock(&shown)).replace('.', "")
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    draw(&mut terminal, "one");
    let first = take_until(b"one");
    let first = &first[filled.min(first.len())..];
    assert!(
        find(first, b"\x1b[2J").is_some(),
        "{:?}",
        String::from_utf8_lossy(first)
    );
    draw(&mut terminal, "one");
    std::thread::sleep(Duration::from_millis(50));
    assert!(lock(&shown).is_empty());
    terminal.clear();
    draw(&mut terminal, "one");
    let cleared = take_until(b"one");
    assert!(find(&cleared, b"\x1b[2J").is_some());
    terminal.close().unwrap();
    drop(pty);
    let _ = reading.join();
}

/// A partial frame writes inside its regions alone, keeps the cursor where the last frame put it,
/// and waits for a full frame after a clear.
#[test]
fn partial_frames_touch_only_their_regions() {
    use crate::layout::Rect;
    use crate::style::Style;
    let (played, signals) = Played::new(Profile::kitty());
    let mut terminal = open(&played, signals, &[], &Options::default());
    let full = terminal.draw(|frame| {
        for y in 0..24 {
            frame
                .buffer_mut()
                .set_string(0, y, "abcdefgh", Style::new());
        }
        frame.set_cursor_position((3, 10));
    });
    assert!(matches!(full, Ok(Drawn::Written(_))));
    played.until("drawn", |emulator| {
        emulator.row_text(23).as_deref() == Some("abcdefgh") && emulator.cursor().visible
    });
    wait_until_written(&terminal);
    lock(&played.emulator).take_events();

    let region = Rect::new(2, 1, 3, 2);
    let partial = terminal.draw_regions(&[region], |frame| {
        // (The writes outside the region are dropped.)
        for y in 0..24 {
            frame
                .buffer_mut()
                .set_string(0, y, "XXXXXXXX", Style::new());
        }
    });
    let Ok(Drawn::Written(completed)) = partial else {
        panic!("the partial frame wasn't written");
    };
    assert_eq!(
        completed.cursor(),
        Some(crate::output::Cursor::at((3, 10).into()))
    );
    played.until("patched", |emulator| {
        emulator.row_text(2).as_deref() == Some("abXXXfgh")
    });
    let emulator = lock(&played.emulator);
    assert_eq!(emulator.row_text(0).as_deref(), Some("abcdefgh"));
    assert_eq!(emulator.row_text(1).as_deref(), Some("abXXXfgh"));
    assert_eq!(emulator.row_text(3).as_deref(), Some("abcdefgh"));
    for event in emulator.events() {
        if let tuiframe_vt::Event::Drawn { at, .. } = event {
            assert!(
                region.contains((at.x, at.y).into()),
                "drew at {at:?}, outside {region:?}"
            );
        }
    }
    assert_eq!((emulator.cursor().x, emulator.cursor().y), (3, 10));
    drop(emulator);

    wait_until_written(&terminal);
    terminal.clear();
    assert!(matches!(
        terminal.draw_regions(&[region], |_| panic!("drawn")),
        Ok(Drawn::FullFrameDue)
    ));
    terminal.close().unwrap();
    assert!(
        lock(&played.unknown).is_empty(),
        "{:?}",
        lock(&played.unknown)
    );
}

/// The tail after close goes on the main screen, each line from the left.
#[test]
fn the_tail_follows_the_reset_on_the_main_screen() {
    let (played, signals) = Played::new(Profile::kitty());
    let terminal = open(&played, signals, &[], &Options::default());
    terminal.close_with_tail(b"one\ntwo\r\nthree").unwrap();
    played.until("written", |emulator| {
        emulator.row_text(2).as_deref() == Some("three")
    });
    let emulator = lock(&played.emulator);
    assert!(!emulator.alternate_screen());
    assert_eq!(emulator.row_text(0).as_deref(), Some("one"));
    assert_eq!(emulator.row_text(1).as_deref(), Some("two"));
}

/// The cursor's shape is the user's until a frame asks for another, so neither a redraw nor the
/// reset sends the default before then, and the reset puts the default back after.
#[test]
fn the_cursor_keeps_the_users_shape_until_a_frame_sets_one() {
    use crate::output::writer::tests::block_on;
    use crate::output::{Cursor, CursorShape};
    let pty = Pty::open(Size::new(80, 24));
    let (mut read, write) = std::io::pipe().unwrap();
    let write = std::os::fd::OwnedFd::from(write);
    let caps = Capabilities {
        cursor_shape: Support::Yes,
        ..Capabilities::default()
    };
    let options = Options {
        capabilities: Some(caps),
        ..Options::default()
    };
    let shown = Arc::new(Mutex::new(Vec::new()));
    let reading = {
        let shown = shown.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(count) = read.read(&mut buffer) {
                if count == 0 {
                    return;
                }
                lock(&shown).extend_from_slice(&buffer[..count]);
            }
        })
    };
    let run = |shape: Option<CursorShape>| {
        let tty = Tty {
            input: pty.slave.try_clone().unwrap(),
            output: write.try_clone().unwrap(),
        };
        let (signals, _resize, _resume) = Signals::unregistered().unwrap();
        let mut terminal = Terminal::open_on(tty, options.clone(), &Vars(&[]), signals).unwrap();
        for _ in 0..2 {
            assert!(block_on(terminal.written(), LONG));
            terminal.clear();
            let drawn = terminal.draw(|frame| {
                frame.set_cursor(Some(Cursor {
                    shape: shape.unwrap_or_default(),
                    ..Cursor::at((1, 1).into())
                }));
            });
            assert!(matches!(drawn, Ok(Drawn::Written(_))));
        }
        terminal.close().unwrap();
    };
    run(None);
    std::thread::sleep(Duration::from_millis(50));
    let unshaped = lock(&shown).split_off(0);
    assert!(find(&unshaped, b"\x1b[?25h").is_some());
    assert_eq!(
        find(&unshaped, b" q"),
        None,
        "{:?}",
        String::from_utf8_lossy(&unshaped)
    );
    run(Some(CursorShape::Bar));
    std::thread::sleep(Duration::from_millis(50));
    let shaped = lock(&shown).split_off(0);
    let bar = find(&shaped, b"\x1b[6 q").expect("the bar");
    assert!(find(&shaped[bar..], b"\x1b[0 q").is_some());
    drop(write);
    reading.join().unwrap();
}

/// The alternate screen comes before the probe, so what a terminal prints of a query it can't
/// parse never reaches the main screen.
#[test]
fn what_a_terminal_prints_of_the_probe_stays_off_the_main_screen() {
    let prints_strings = Profile {
        linux_console: true,
        ..Profile::kitty()
    };
    let (played, signals) = Played::new(prints_strings);
    let mut terminal = open(&played, signals, &[], &Options::default());
    played.until("printed", |emulator| {
        emulator.alternate_screen() && emulator.rows().iter().any(|row| !row.is_empty())
    });
    let drawn = terminal.draw(|_| {});
    assert!(matches!(drawn, Ok(Drawn::Written(_))));
    played.until("cleared", |emulator| {
        emulator.rows().iter().all(String::is_empty)
    });
    terminal.close().unwrap();
    played.until("given back", |emulator| emulator.mode(25));
    let emulator = lock(&played.emulator);
    assert!(!emulator.alternate_screen());
    assert!(
        emulator.rows().iter().all(String::is_empty),
        "{:?}",
        emulator.rows()
    );
}

/// A lone ESC waits 20 ms for more, or 100 ms where the kitty protocol is on, then it's Esc.
#[test]
fn a_lone_escape_waits_longer_under_the_kitty_protocol() {
    for (profile, wait) in [(Profile::xterm(), 20), (Profile::kitty(), 100)] {
        let (mut played, signals) = Played::new(profile);
        let terminal = open(&played, signals, &[], &Options::default());
        let typed = Instant::now();
        played.send(b"\x1b");
        assert_eq!(next(&terminal), key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            typed.elapsed() >= Duration::from_millis(wait),
            "{:?}",
            typed.elapsed()
        );
        terminal.close().unwrap();
    }
}

/// A frame reads the size from the terminal, since SIGWINCH reaches the foreground process group
/// alone, and a terminal that says 0x0 gets `COLUMNS` and `LINES`, or 80x24.
#[test]
fn frames_read_the_size_from_the_terminal() {
    let (played, signals) = Played::new(Profile::kitty());
    let mut terminal = open(&played, signals, &[], &Options::default());
    played.pty.resize(Size::new(90, 20));
    wait_until_written(&terminal);
    let drawn = terminal.draw(|frame| {
        assert_eq!(frame.area(), crate::layout::Rect::new(0, 0, 90, 20));
    });
    assert!(matches!(drawn, Ok(Drawn::Written(_))));
    assert_eq!(terminal.size(), Size::new(90, 20));
    terminal.close().unwrap();

    for (vars, size) in [
        (
            &[("COLUMNS", "100"), ("LINES", "40")][..],
            Size::new(100, 40),
        ),
        (&[("COLUMNS", "0")][..], Size::new(80, 24)),
    ] {
        let (played, signals) = Played::new(Profile::kitty());
        played.pty.resize(Size::ZERO);
        let options = Options::default().capabilities(Capabilities::default());
        let terminal = Terminal::open_on(played.tty(), options, &Vars(vars), signals).unwrap();
        assert_eq!(terminal.size(), size);
        terminal.close().unwrap();
    }
}
