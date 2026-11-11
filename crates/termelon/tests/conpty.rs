//! Programs on a Windows pseudoconsole (`ConPTY`), end to end, with termelon-vt playing the terminal
//! on its other side. Each test runs this binary again as the program, on the pseudoconsole, and
//! the program checks what it gets and exits, so a failure is its exit status and what it printed.
//!
//! portable-pty takes a `conpty.dll` beside the binary over the system's, so the same tests run
//! with the pseudoconsole Windows Terminal ships by copying its `conpty.dll` and `OpenConsole.exe`
//! there.
#![cfg(windows)]

// (In a test module, where clippy lets tests unwrap.)
#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::{Duration, Instant};

    use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
    use termelon::caps::KeyboardSupport;
    use termelon::{
        Drawn, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, Options, Size, Style, Terminal,
    };
    use termelon_vt::{Emulator, Key, Mods, Profile};

    /// The variable that makes this binary the program that runs a case.
    const CHILD: &str = "TERMELON_CONPTY_CHILD";

    const LONG: Duration = Duration::from_secs(10);

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The case this process runs as the program, if it's one.
    fn case() -> Option<String> {
        std::env::var(CHILD).ok()
    }

    /// A pseudoconsole whose terminal side the emulator plays, with this binary running a test's
    /// program on it.
    struct Played {
        emulator: Arc<Mutex<Emulator>>,
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        master: Box<dyn MasterPty + Send>,
        child: Box<dyn Child + Send + Sync>,
    }

    impl Played {
        /// Runs the program of `test` on a new 80 by 24 pseudoconsole, as the terminal `profile`
        /// plays.
        fn spawn(test: &str, profile: Profile) -> Self {
            let size = PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            };
            let pty = native_pty_system().openpty(size).unwrap();
            let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
            let path = format!("tests::{test}");
            command.args([
                path.as_str(),
                "--exact",
                "--nocapture",
                "--test-threads",
                "1",
            ]);
            command.env(CHILD, test);
            let child = pty.slave.spawn_command(command).unwrap();
            drop(pty.slave);
            let mut emulator = Emulator::with_profile(profile, 80, 24);
            // (The pseudoconsole writes sequences of its own, such as asking for win32-input-mode.)
            emulator.set_strict(false);
            let emulator = Arc::new(Mutex::new(emulator));
            let writer = Arc::new(Mutex::new(pty.master.take_writer().unwrap()));
            let mut reader = pty.master.try_clone_reader().unwrap();
            {
                let (emulator, writer) = (emulator.clone(), writer.clone());
                std::thread::spawn(move || {
                    let mut buffer = [0; 4096];
                    while let Ok(read) = reader.read(&mut buffer) {
                        if read == 0 {
                            return;
                        }
                        let answer = {
                            let mut emulator = lock(&emulator);
                            let _ = emulator.feed(&buffer[..read]);
                            emulator.take_output()
                        };
                        if !answer.is_empty() && lock(&writer).write_all(&answer).is_err() {
                            return;
                        }
                    }
                });
            }
            Self {
                emulator,
                writer,
                master: pty.master,
                child,
            }
        }

        /// Types `bytes` as the terminal sends them.
        fn send(&self, bytes: &[u8]) {
            let mut writer = lock(&self.writer);
            writer.write_all(bytes).unwrap();
            writer.flush().unwrap();
        }

        fn key(&self, key: Key, mods: Mods) {
            let bytes = lock(&self.emulator).encode_key(key, mods);
            self.send(&bytes);
        }

        /// Waits until the screen shows `text`.
        fn until_shown(&self, text: &str) {
            let started = Instant::now();
            while !lock(&self.emulator).text().contains(text) {
                assert!(
                    started.elapsed() < LONG,
                    "{text:?} never showed, only {:?}",
                    lock(&self.emulator).rows()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn resize(&self, size: Size) {
            lock(&self.emulator).resize(size.width, size.height);
            let size = PtySize {
                rows: size.height,
                cols: size.width,
                pixel_width: 0,
                pixel_height: 0,
            };
            self.master.resize(size).unwrap();
        }

        /// Waits for the program to end, and fails with what it showed unless it ended well.
        fn finish(mut self) {
            let started = Instant::now();
            let status = loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    break status;
                }
                if started.elapsed() > LONG {
                    let _ = self.child.kill();
                    panic!(
                        "the program never ended, and showed {:?}",
                        lock(&self.emulator).rows()
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(
                status.success(),
                "the program failed with {status:?}, and showed {:?}",
                lock(&self.emulator).rows()
            );
        }
    }

    /// Draws `line` alone, on the top row, waiting while the console takes the last frame.
    fn say(terminal: &mut Terminal, line: &str) {
        let started = Instant::now();
        loop {
            let drawn = terminal
                .draw(|frame| frame.buffer_mut().set_string(0, 0, line, Style::new()))
                .unwrap();
            if !matches!(drawn, Drawn::Held) {
                return;
            }
            assert!(started.elapsed() < LONG, "the console never took a frame");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The next key press, or resize, skipping releases and other events.
    fn next(terminal: &Terminal) -> Event {
        let mut events = terminal.events();
        loop {
            match events.next_timeout(LONG).unwrap() {
                Some(Event::Key(key)) if key.kind == KeyEventKind::Release => {}
                Some(event @ (Event::Key(_) | Event::Resize(_))) => return event,
                Some(_) => {}
                None => panic!("no key came"),
            }
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn open() -> Terminal {
        termelon::install_panic_hook();
        Terminal::open(Options::default()).unwrap()
    }

    /// The probe, keys in as the pseudoconsole turns the terminal's bytes into key records, a frame
    /// out, a resize, and close.
    #[test]
    fn a_program_runs_on_a_pseudoconsole() {
        const TEST: &str = "a_program_runs_on_a_pseudoconsole";
        let keys = [
            (
                (Key::Char('a'), Mods::NONE),
                key(KeyCode::Char('a'), KeyModifiers::NONE),
            ),
            (
                (Key::Char('c'), Mods::CTRL),
                key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            ),
            ((Key::Up, Mods::NONE), key(KeyCode::Up, KeyModifiers::NONE)),
            (
                (Key::Tab, Mods::SHIFT),
                key(KeyCode::BackTab, KeyModifiers::SHIFT),
            ),
            (
                (Key::Char('x'), Mods::ALT),
                key(KeyCode::Char('x'), KeyModifiers::ALT),
            ),
            (
                (Key::Enter, Mods::NONE),
                key(KeyCode::Enter, KeyModifiers::NONE),
            ),
        ];
        if case().as_deref() == Some(TEST) {
            let mut terminal = open();
            assert_eq!(
                terminal.capabilities().keyboard,
                KeyboardSupport::ConsoleRecords
            );
            assert_eq!(terminal.size(), Size::new(80, 24));
            say(&mut terminal, "ready");
            for (_, expected) in &keys {
                assert_eq!(&next(&terminal), expected);
                say(&mut terminal, &format!("got {expected:?}"));
            }
            assert_eq!(next(&terminal), Event::Resize(Size::new(100, 30)));
            assert_eq!(terminal.size(), Size::new(100, 30));
            say(&mut terminal, "done");
            terminal.close().unwrap();
            return;
        }
        let played = Played::spawn(TEST, Profile::windows_terminal());
        played.until_shown("ready");
        for ((typed, mods), expected) in &keys {
            played.key(*typed, *mods);
            played.until_shown(&format!("got {expected:?}"));
        }
        played.resize(Size::new(100, 30));
        played.until_shown("done");
        played.finish();
    }

    /// A reply that comes after the probe stopped waiting, as over a slow link, is dropped rather than
    /// typed.
    #[test]
    fn a_late_reply_on_a_pseudoconsole_is_no_keys() {
        const TEST: &str = "a_late_reply_on_a_pseudoconsole_is_no_keys";
        if case().as_deref() == Some(TEST) {
            let mut terminal = open();
            say(&mut terminal, "ready");
            assert_eq!(next(&terminal), key(KeyCode::Char('z'), KeyModifiers::NONE));
            terminal.close().unwrap();
            return;
        }
        let played = Played::spawn(TEST, Profile::windows_terminal());
        played.until_shown("ready");
        played.send(b"\x1b[?62;22c\x1b]11;rgb:0000/0000/0000\x1b\\");
        std::thread::sleep(Duration::from_millis(100));
        played.key(Key::Char('z'), Mods::NONE);
        played.finish();
    }

    /// A terminal that answers nothing leaves the probe to its timeout, and keys typed after are
    /// themselves.
    #[test]
    fn a_silent_terminal_on_a_pseudoconsole_times_the_probe_out() {
        const TEST: &str = "a_silent_terminal_on_a_pseudoconsole_times_the_probe_out";
        if case().as_deref() == Some(TEST) {
            let started = Instant::now();
            let mut terminal = open();
            assert!(started.elapsed() < Duration::from_secs(3));
            say(&mut terminal, "ready");
            assert_eq!(next(&terminal), key(KeyCode::Char('q'), KeyModifiers::NONE));
            terminal.close().unwrap();
            return;
        }
        // (The pseudoconsole itself asks its terminal where the cursor is as it starts, and waits
        // for the answer, so this terminal answers that alone.)
        let mut silent = Profile::silent();
        silent.cursor_report = true;
        let played = Played::spawn(TEST, silent);
        played.until_shown("ready");
        played.key(Key::Char('q'), Mods::NONE);
        played.finish();
    }

    /// Releasing lends the console to another program, and reacquiring takes it back, drawing again.
    #[test]
    fn release_and_reacquire_on_a_pseudoconsole() {
        const TEST: &str = "release_and_reacquire_on_a_pseudoconsole";
        if case().as_deref() == Some(TEST) {
            let mut terminal = open();
            say(&mut terminal, "ready");
            terminal.release().unwrap();
            std::thread::sleep(Duration::from_millis(200));
            terminal.reacquire().unwrap();
            say(&mut terminal, "back");
            assert_eq!(next(&terminal), key(KeyCode::Char('b'), KeyModifiers::NONE));
            terminal.close().unwrap();
            return;
        }
        let played = Played::spawn(TEST, Profile::windows_terminal());
        played.until_shown("ready");
        played.until_shown("back");
        played.key(Key::Char('b'), Mods::NONE);
        played.finish();
    }
}
