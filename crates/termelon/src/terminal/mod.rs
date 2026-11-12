//! The terminal as a program has it, [`Terminal`], opened with [`Options`], and its [`Events`].

mod events;

pub use events::{Events, NextEvent};

use std::fmt;
use std::io;
use std::time::Duration;

use crate::caps::{Capabilities, KeyboardSupport, Support};
use crate::output::CompletedFrame;
use crate::restore::Enabled;

/// What [`Terminal::draw`] did.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "a held frame is drawn again once the terminal takes the last one"]
pub enum Drawn<'a> {
    /// The terminal hasn't taken the last frame yet, so this one wasn't drawn. Draw again once
    /// [`Terminal::written`] completes, and that frame carries every change since.
    Held,
    /// The frame went to the terminal.
    Written(CompletedFrame<'a>),
    /// A full frame is due, after a resize, a resume or [`Terminal::clear`], so
    /// [`Terminal::draw_regions`] drew nothing. Draw one with [`Terminal::draw`].
    FullFrameDue,
}

/// Which mouse reports the terminal sends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MouseMode {
    /// None.
    #[default]
    Off,
    /// Buttons, the wheel, and motion while a button is down, which selecting text needs (modes
    /// 1000, 1002 and 1006).
    ButtonsAndDrags,
    /// Every motion of the mouse too, which wakes the program at each move (mode 1003).
    AnyMotion,
}

/// How keys are asked to be reported, where the terminal has the kitty keyboard protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum KeyboardRequest {
    /// The legacy encodings, where Shift+Enter arrives as Enter.
    Legacy,
    /// The protocol's flags 1 and 4, which tell apart keys the legacy encodings merge, such as
    /// Shift+Enter and Enter, or Ctrl+I and Tab, and give the character a shifted key types.
    /// Where the terminal lacks the protocol and is known to take xterm's modifyOtherKeys, that
    /// is turned on instead.
    #[default]
    Disambiguate,
    /// These flags, of those the terminal has, as in kitty's `CSI > flags u`.
    Flags(u8),
}

/// The choices made at open. The defaults suit most full-screen programs, and each field has a
/// builder method of its name.
///
/// ```
/// use std::time::Duration;
/// use termelon::{MouseMode, Options};
///
/// let options = Options::default()
///     .mouse(MouseMode::Off)
///     .escape_timeout(Duration::from_millis(25));
/// assert!(options.bracketed_paste);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// Which mouse reports to turn on. `ButtonsAndDrags` by default.
    pub mouse: MouseMode,
    /// Bracketed paste, so a paste arrives as one [`Event::Paste`](crate::Event::Paste). On by
    /// default.
    pub bracketed_paste: bool,
    /// Focus reports. On by default.
    pub focus_events: bool,
    /// How keys are reported. `Disambiguate` by default.
    pub keyboard: KeyboardRequest,
    /// Saves the window's title on xterm's title stack and puts it back at close. On by default.
    pub save_title: bool,
    /// How long the startup probe waits for a terminal that doesn't answer. By default 500 ms,
    /// or 1.5 s over ssh (`SSH_CONNECTION` or `SSH_TTY` set), where a round trip takes longer.
    pub probe_timeout: Option<Duration>,
    /// Measures how the terminal draws emoji sequences and East Asian ambiguous characters, which
    /// costs a second round trip at open. Off by default.
    pub width_probes: bool,
    /// How long a lone ESC waits for a following byte before it's the Esc key. By default 20 ms,
    /// or 100 ms where the kitty keyboard protocol is on, since the Esc key then sends a sequence
    /// of its own and a lone ESC is most likely the start of one that came split.
    pub escape_timeout: Option<Duration>,
    /// Skips the probe and uses these, for tests and setups the probe gets wrong.
    pub capabilities: Option<Capabilities>,
    /// Installs [`install_panic_hook`](crate::install_panic_hook) at open, so a panic gives the
    /// terminal back before its message prints. On by default. A program with its own rules for
    /// which panics end it turns this off and builds its hook on [`RestoreHandle`](crate::RestoreHandle).
    pub panic_hook: bool,
    /// Gives the terminal back on SIGTERM, SIGHUP, SIGINT and SIGQUIT, then lets the signal end
    /// the program as it would have. Off by default, since a program usually has its own policy
    /// for these. Once the terminal closes, they act as they did before.
    pub restore_on_signals: bool,
    /// Keeps a record of every byte the terminal is sent, the setup, the probe, each frame and the
    /// out-of-band commands, in order, for [`Terminal::take_written`]. Off by default, and it costs
    /// nothing then.
    pub record_output: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mouse: MouseMode::ButtonsAndDrags,
            bracketed_paste: true,
            focus_events: true,
            keyboard: KeyboardRequest::Disambiguate,
            save_title: true,
            probe_timeout: None,
            width_probes: false,
            escape_timeout: None,
            capabilities: None,
            panic_hook: true,
            restore_on_signals: false,
            record_output: false,
        }
    }
}

macro_rules! builders {
    ($($(#[$doc:meta])* $field:ident: $ty:ty,)*) => {
        impl Options {
            $(
                $(#[$doc])*
                #[must_use]
                pub fn $field(mut self, $field: $ty) -> Self {
                    self.$field = $field;
                    self
                }
            )*
        }
    };
}

builders! {
    /// Sets [`mouse`](Options::mouse).
    mouse: MouseMode,
    /// Sets [`bracketed_paste`](Options::bracketed_paste).
    bracketed_paste: bool,
    /// Sets [`focus_events`](Options::focus_events).
    focus_events: bool,
    /// Sets [`keyboard`](Options::keyboard).
    keyboard: KeyboardRequest,
    /// Sets [`save_title`](Options::save_title).
    save_title: bool,
    /// Sets [`width_probes`](Options::width_probes).
    width_probes: bool,
    /// Sets [`panic_hook`](Options::panic_hook).
    panic_hook: bool,
    /// Sets [`restore_on_signals`](Options::restore_on_signals).
    restore_on_signals: bool,
    /// Sets [`record_output`](Options::record_output).
    record_output: bool,
}

impl Options {
    /// Sets [`escape_timeout`](Options::escape_timeout).
    #[must_use]
    pub fn escape_timeout(mut self, timeout: Duration) -> Self {
        self.escape_timeout = Some(timeout);
        self
    }

    /// Sets [`probe_timeout`](Options::probe_timeout).
    #[must_use]
    pub fn probe_timeout(mut self, timeout: Duration) -> Self {
        self.probe_timeout = Some(timeout);
        self
    }

    /// Sets [`capabilities`](Options::capabilities), which skips the probe.
    #[must_use]
    pub fn capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = Some(capabilities);
        self
    }
}

/// What went wrong.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Neither standard input nor standard output is a terminal, and the process has no
    /// controlling terminal.
    NoTerminal(io::Error),
    /// The Windows console can't take escape sequences.
    NoVirtualTerminal,
    /// The events ended, as the terminal closed or stopped taking output.
    Closed,
    /// Reading or writing the terminal failed.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::NoTerminal(_) => "no terminal to open",
            Error::NoVirtualTerminal => "the console can't take escape sequences",
            Error::Closed => "the terminal closed",
            Error::Io(_) => "the terminal's input or output failed",
        })
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::NoTerminal(error) | Error::Io(error) => Some(error),
            Error::NoVirtualTerminal | Error::Closed => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

/// A result with an [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What to turn on for `options` in a terminal with `caps`. Private modes go out where support
/// is unknown, since terminals ignore modes they don't know.
pub(crate) fn enabled(options: &Options, caps: &Capabilities) -> Enabled {
    let has = |support: Support| support != Support::No;
    let keyboard = match (options.keyboard, caps.keyboard) {
        (KeyboardRequest::Disambiguate, KeyboardSupport::Kitty { flags }) => Some(flags & 0b101),
        (KeyboardRequest::Flags(asked), KeyboardSupport::Kitty { flags }) => Some(flags & asked),
        _ => None,
    };
    Enabled {
        alternate_screen: has(caps.alternate_screen),
        clear: !has(caps.alternate_screen),
        mouse: if has(caps.mouse) {
            options.mouse
        } else {
            MouseMode::Off
        },
        focus: options.focus_events && has(caps.focus_events),
        bracketed_paste: options.bracketed_paste && has(caps.bracketed_paste),
        grapheme_clusters: caps.grapheme_clustering == Support::Yes,
        theme_reports: false,
        keyboard: keyboard.filter(|flags| *flags != 0),
        modify_other_keys: options.keyboard != KeyboardRequest::Legacy
            && caps.keyboard == KeyboardSupport::ModifyOtherKeys,
        save_title: options.save_title && has(caps.title_stack),
        hyperlinks: caps.hyperlinks == Support::Yes,
        // (Until a frame asks for a shape.)
        cursor_shape: false,
    }
}

#[cfg(any(unix, windows))]
pub use platform::{Terminal, terminal_size};

#[cfg(any(unix, windows))]
mod platform {
    use std::future::Future;
    use std::io;
    #[cfg(unix)]
    use std::os::fd::AsFd;
    use std::time::Duration;

    use super::{Drawn, Error, Events, MouseMode, Options, Result, base64, crlf, enabled};
    use crate::caps::env::Environment;
    use crate::caps::{self, Capabilities, Support, probe};
    use crate::event::decode::{Decoder, DecoderOptions, Query, Reply};
    use crate::frame::{Frame, Frames};
    use crate::layout::Rect;
    use crate::layout::Size;
    use crate::output::writer::Hold;
    use crate::output::{Cursor, CursorShape};
    #[cfg(unix)]
    use crate::restore::screen::Screen;
    use crate::restore::{CLOSE_WAIT, Enabled, RestoreHandle, mouse_switch};
    #[cfg(unix)]
    use crate::sys::unix::{Signals, Tty, window_size};
    #[cfg(windows)]
    use crate::{
        caps::KeyboardSupport,
        sys::windows::{Console, InputMode, WinConsole, screen::Screen},
    };

    /// How long the probe waits for a terminal that doesn't answer, by default.
    const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

    /// The same over ssh, where a round trip takes longer.
    const SSH_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

    /// How long a lone ESC waits for more before it's Esc, by default.
    const ESCAPE_TIMEOUT: Duration = Duration::from_millis(20);

    /// The same where the kitty keyboard protocol is on, under which the Esc key sends a sequence.
    const KITTY_ESCAPE_TIMEOUT: Duration = Duration::from_millis(100);

    /// The terminal while a program has it.
    ///
    /// Opening it puts the terminal in raw mode, asks what it supports, enters the alternate
    /// screen and turns on what [`Options`] asks for. Closing it, or dropping it, gives all of it
    /// back. Two threads of its own read and write the terminal, so nothing the program does
    /// waits for the terminal.
    pub struct Terminal {
        screen: Screen,
        caps: Capabilities,
        options: Options,
        enabled: Enabled,
        frames: Frames,
        /// The size to take when the terminal says 0x0 or nothing.
        fallback: Size,
    }

    impl std::fmt::Debug for Terminal {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Terminal")
                .field("size", &self.size())
                .field("capabilities", &self.caps)
                .field("options", &self.options)
                .finish_non_exhaustive()
        }
    }

    impl Terminal {
        /// Opens the terminal, standard input and output where they're terminals and `/dev/tty`
        /// otherwise.
        ///
        /// Before it returns, it asks the terminal what it supports with one batch of queries,
        /// which takes one round trip, or [`Options::probe_timeout`] with a terminal that doesn't
        /// answer. Replies that come later are dropped rather than taken as keys.
        ///
        /// # Errors
        ///
        /// [`Error::NoTerminal`] when there's no terminal, and [`Error::Io`] when setting it up
        /// failed. The terminal is then as it was.
        pub fn open(options: Options) -> Result<Self> {
            if options.panic_hook {
                crate::install_panic_hook();
            }
            #[cfg(unix)]
            let terminal = {
                let tty = Tty::open().map_err(Error::NoTerminal)?;
                let signals = Signals::register(options.restore_on_signals)?;
                Self::open_on(tty, options, &caps::env::Process, signals)?
            };
            #[cfg(windows)]
            let terminal = {
                let console = WinConsole::open().map_err(Error::NoTerminal)?;
                Self::open_on(console, options, &caps::env::Process)?
            };
            // (Text measures as the terminal draws it from now on.)
            crate::text::set_width_profile(terminal.caps.width_profile);
            Ok(terminal)
        }

        /// [`open`](Self::open), on `tty`, with `env` for the environment and `signals` for the
        /// reader.
        #[cfg(unix)]
        pub(crate) fn open_on(
            tty: Tty,
            options: Options,
            env: &dyn Environment,
            signals: Signals,
        ) -> Result<Self> {
            let screen = Screen::open_with(
                tty,
                decoder,
                // (The probe's replies may come split, so its lone ESC waits longer.)
                options.escape_timeout.unwrap_or(KITTY_ESCAPE_TIMEOUT),
                signals,
            )?;
            Self::start(screen, options, env)
        }

        /// [`open`](Self::open), on `console`, with `env` for the environment. A console without
        /// virtual terminal processing gives [`Error::NoVirtualTerminal`].
        #[cfg(windows)]
        pub(crate) fn open_on(
            console: WinConsole,
            options: Options,
            env: &dyn Environment,
        ) -> Result<Self> {
            let timeout = options.escape_timeout.unwrap_or(KITTY_ESCAPE_TIMEOUT);
            let screen = Screen::open(console, decoder, timeout).map_err(|error| {
                if error.kind() == io::ErrorKind::Unsupported {
                    Error::NoVirtualTerminal
                } else {
                    Error::Io(error)
                }
            })?;
            // (The console answers the probe in its input, where the answers decode as replies
            // only from bytes.)
            screen.set_input(InputMode::Vt)?;
            Self::start(screen, options, env)
        }

        /// Asks the terminal on `screen` what it supports, turns on what `options` ask for, and
        /// makes the terminal the program has.
        fn start(screen: Screen, options: Options, env: &dyn Environment) -> Result<Self> {
            screen.writer().record(options.record_output);
            let overrides = env.var("TERMELON_CAPS");
            let hints = caps::env::hints(env);
            // (A Windows console with virtual terminal processing, which opening needs, draws
            // 24-bit colour, and conhost sets no variable that says so.)
            #[cfg(windows)]
            let hints = caps::env::Hints {
                colors: Some(caps::ColorDepth::TrueColor),
                ..hints
            };
            let probe_timeout = options.probe_timeout.unwrap_or(if hints.ssh {
                SSH_PROBE_TIMEOUT
            } else {
                PROBE_TIMEOUT
            });
            // (The alternate screen first, so what a terminal prints of a query it can't parse lands
            // on a screen the first frame clears. The Linux console has none.)
            let early = Enabled {
                alternate_screen: !hints.linux_console,
                ..Enabled::default()
            };
            if early.alternate_screen {
                screen.enable(&early)?;
            }
            // (Whether the terminal answered every query, which DA1's reply says.)
            let mut answered = true;
            let mut caps = if let Some(caps) = &options.capabilities {
                caps.clone()
            } else {
                let queries = probe::queries(hints.linux_console);
                let replies = ask(&screen, &queries, 0, probe_timeout)?;
                answered = replies.iter().any(probe::is_end);
                let answers = probe::answers(&replies);
                caps::detect(&answers, &hints, overrides.as_deref())
            };
            // (Keys come as key records once the probe is done, where the kitty protocol isn't
            // pushed.)
            #[cfg(windows)]
            {
                caps.keyboard = KeyboardSupport::ConsoleRecords;
            }
            let mut enabled = enabled(&options, &caps);
            if early.alternate_screen {
                enabled.alternate_screen = true;
                enabled.clear = false;
            }
            screen.set_enabled(&enabled);
            screen.set_escape_timeout(options.escape_timeout.unwrap_or(
                if enabled.keyboard.is_some() {
                    KITTY_ESCAPE_TIMEOUT
                } else {
                    ESCAPE_TIMEOUT
                },
            ));
            screen
                .writer()
                .send(enabled.setup_after(&early))
                .map_err(|failure| Error::Io(failure.into()))?;
            // (So the first frame isn't held behind the setup.)
            let _ = screen
                .writer()
                .wait(screen.writer().ticket(), probe_timeout);
            if options.width_probes && options.capabilities.is_none() && !hints.linux_console {
                let samples = probe::WIDTH_SAMPLES.len();
                let replies = ask(&screen, &probe::width_queries(), samples, probe_timeout)?;
                answered &= replies.iter().any(probe::is_end);
                if let Some(profile) = probe::width_profile(&replies) {
                    caps.width_profile = profile;
                    if let Some(overrides) = &overrides {
                        caps::apply_overrides(&mut caps, overrides);
                    }
                }
            }
            #[cfg(windows)]
            {
                if !answered {
                    // (Replies that come late decode as replies a moment longer, and go.)
                    std::thread::sleep(LATE_REPLIES);
                }
                screen.set_input(InputMode::Records)?;
            }
            #[cfg(not(windows))]
            let _ = answered;
            let fallback = fallback_size(env);
            let size = screen.window_size().unwrap_or(fallback);
            screen.shared().set_size(size);
            let area = Rect::from(size);
            Ok(Self {
                fallback,
                frames: Frames::new(area, caps.width_profile),
                screen,
                caps,
                options,
                enabled,
            })
        }

        /// Draws a frame. It calls `render` with a [`Frame`] over a blank buffer the terminal's
        /// size, and writes what changed since the last frame the terminal took, with the cursor,
        /// as one packet. While the terminal hasn't taken the last frame, it returns
        /// [`Drawn::Held`] without calling `render`.
        ///
        /// A resize, a resume and [`clear`](Self::clear) make the next frame that's written redraw
        /// everything, also when no cell changed.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn draw<F: FnOnce(&mut Frame<'_>)>(&mut self, render: F) -> Result<Drawn<'_>> {
            let writer = self.screen.writer();
            if let Some(failure) = writer.failure() {
                return Err(Error::Io(failure.into()));
            }
            if !writer.is_idle() {
                return Ok(Drawn::Held);
            }
            self.screen.draw_here();
            if self.frames.resize(Rect::from(self.size_now())) {
                self.screen.shared().request_redraw();
            }
            let cursor = self.frames.render(render);
            note_shape(&mut self.enabled, &self.caps, &self.screen, cursor);
            let full = self.screen.shared().take_redraw();
            let mut packet = writer.spare_buffer();
            let stats = self.frames.encode(full, cursor, &self.caps, &mut packet);
            if !packet.is_empty()
                && let Err(failure) = writer.send(packet)
            {
                if full {
                    self.screen.shared().request_redraw();
                }
                return Err(Error::Io(failure.into()));
            }
            Ok(Drawn::Written(self.frames.commit(cursor, stats)))
        }

        /// Draws a partial frame, which changes `regions` alone, for something that moves while
        /// the rest stays, such as a spinner. `render` draws over the last frame's cells with every
        /// write outside `regions` dropped, so it can repaint the regions whole or patch them, and
        /// only what changed in them is written. The cursor stays where the last frame put it,
        /// unless `render` sets it.
        ///
        /// It returns [`Drawn::Held`] while the terminal hasn't taken the last frame, and
        /// [`Drawn::FullFrameDue`] when a full frame is due, both without calling `render`.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn draw_regions<F: FnOnce(&mut Frame<'_>)>(
            &mut self,
            regions: &[Rect],
            render: F,
        ) -> Result<Drawn<'_>> {
            let writer = self.screen.writer();
            if let Some(failure) = writer.failure() {
                return Err(Error::Io(failure.into()));
            }
            if !writer.is_idle() {
                return Ok(Drawn::Held);
            }
            if Rect::from(self.size_now()) != self.frames.area()
                || self.screen.shared().redraw_pending()
            {
                return Ok(Drawn::FullFrameDue);
            }
            self.screen.draw_here();
            let cursor = self.frames.render_regions(regions, render);
            note_shape(&mut self.enabled, &self.caps, &self.screen, cursor);
            let mut packet = writer.spare_buffer();
            let stats = self
                .frames
                .encode_regions(regions, cursor, &self.caps, &mut packet);
            if !packet.is_empty()
                && let Err(failure) = writer.send(packet)
            {
                // (The terminal didn't get what the front buffer now holds.)
                self.screen.shared().request_redraw();
                return Err(Error::Io(failure.into()));
            }
            Ok(Drawn::Written(self.frames.commit_regions(cursor, stats)))
        }

        /// Completes once the terminal took every frame and command so far, so a frame
        /// [`draw`](Self::draw) held can be drawn. It may complete early, when the writer stopped
        /// taking output.
        pub fn written(&self) -> impl Future<Output = ()> + '_ {
            self.screen.writer().written()
        }

        /// The program's end of the events. Every call gives a handle on the same queue.
        pub fn events(&self) -> Events {
            Events::new(self.screen.shared().clone())
        }

        /// What the terminal supports, as found at open.
        pub fn capabilities(&self) -> &Capabilities {
            &self.caps
        }

        /// The terminal's size, as last reported, read without a system call.
        pub fn size(&self) -> Size {
            self.screen.shared().size()
        }

        /// What gives the terminal back from any thread, for a panic hook or a thread that
        /// handles signals.
        pub fn restore_handle(&self) -> RestoreHandle {
            self.screen.restore_handle()
        }

        /// Makes a console control event (the console window closing, Ctrl+Break, logoff or
        /// shutdown) give the console back before Windows ends the process, as a program's own
        /// signal thread does with [`restore_handle`](Self::restore_handle) on Unix. Windows calls
        /// it on a thread of its own, and the process ends once it returns.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] when the handler couldn't be installed.
        #[cfg(windows)]
        pub fn restore_on_console_events(&self) -> Result<()> {
            Ok(self.screen.restore_on_console_events()?)
        }

        /// Sets the window's title with OSC 0, where the terminal is known to take it, and does
        /// nothing elsewhere. Control characters in `title` are left out. Like the bell and a
        /// copy, it goes with the next packet the terminal takes, at once while it's idle, and a
        /// later title replaces one still waiting.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn set_title(&mut self, title: &str) -> Result<()> {
            if self.caps.title != Support::Yes {
                return Ok(());
            }
            let title: String = title.chars().filter(|c| !c.is_control()).collect();
            self.hold(Hold::Title, format!("\x1b]0;{title}\x1b\\").into_bytes())
        }

        /// Rings the bell, once however many times it's rung while the terminal is busy.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn bell(&mut self) -> Result<()> {
            self.hold(Hold::Bell, vec![0x07])
        }

        /// Copies `text` to the clipboard with OSC 52, where the terminal is known to take it, and
        /// does nothing elsewhere. Returns whether the copy went out, so a program says it copied
        /// only then. Inside tmux, the copy reaches the outer terminal only with tmux's
        /// `set-clipboard` on, which can't be seen from inside.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn copy_to_clipboard(&mut self, text: &str) -> Result<bool> {
            if self.caps.clipboard_write != Support::Yes {
                return Ok(false);
            }
            let encoded = base64(text.as_bytes());
            self.hold(
                Hold::Copy,
                format!("\x1b]52;c;{encoded}\x1b\\").into_bytes(),
            )?;
            Ok(true)
        }

        /// Switches the mouse reports to `mode`. A terminal known to have no mouse reports gets
        /// none.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn set_mouse(&mut self, mode: MouseMode) -> Result<()> {
            let mode = if self.caps.mouse == Support::No {
                MouseMode::Off
            } else {
                mode
            };
            let switch = mouse_switch(self.enabled.mouse, mode);
            self.enabled.mouse = mode;
            self.screen.set_enabled(&self.enabled);
            if switch.is_empty() {
                return Ok(());
            }
            self.send(switch)
        }

        /// The terminal's size as it says now. SIGWINCH reaches only the terminal's foreground
        /// process group, so a frame asks rather than counting on it.
        fn size_now(&self) -> Size {
            let size = self.screen.window_size().unwrap_or(self.fallback);
            let shared = self.screen.shared();
            if size != shared.size() {
                shared.set_size(size);
                shared.request_redraw();
            }
            size
        }

        /// Makes the next frame redraw everything, for a screen something else wrote over. Nothing
        /// asks the terminal where its cursor is.
        pub fn clear(&mut self) {
            self.screen.shared().request_redraw();
        }

        /// Suspends the program as Ctrl+Z does in a shell, which raw mode turns into a key. It
        /// gives the terminal back and stops the process. When the shell runs it again, it takes
        /// the terminal back, the events get [`Event::Resumed`](crate::Event::Resumed), and the
        /// next frame redraws everything.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] when the terminal couldn't be given back or taken back.
        pub fn suspend(&mut self) -> Result<()> {
            Ok(self.screen.suspend()?)
        }

        /// Gives the terminal back as closing does and stops reading it, for running another
        /// program on it, such as an editor or a pager, until [`reacquire`](Self::reacquire).
        /// Frames drawn meanwhile reach the terminal, so draw none.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] when the terminal couldn't be given back.
        pub fn release(&mut self) -> Result<()> {
            Ok(self.screen.release()?)
        }

        /// Takes the terminal back after [`release`](Self::release). The next frame redraws
        /// everything.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] when the terminal couldn't be taken back.
        pub fn reacquire(&mut self) -> Result<()> {
            Ok(self.screen.reacquire()?)
        }

        /// Why the terminal takes no more output, once it doesn't. A write that failed gives its
        /// error's kind, and a writer thread that ended gives [`io::ErrorKind::Other`]. The events
        /// end then, so the program ends as when its terminal goes away.
        pub fn failure(&self) -> Option<io::ErrorKind> {
            self.screen
                .writer()
                .failure()
                .map(|failure| io::Error::from(failure).kind())
        }

        /// The bytes the terminal took since open or the last call, in order, where
        /// [`Options::record_output`] is on, and none otherwise. A frame is in it once the
        /// terminal took it, which [`written`](Self::written) waits for.
        pub fn take_written(&mut self) -> Vec<u8> {
            self.screen.writer().take_recorded()
        }

        /// How long the terminal has kept the write going on waiting, for a program that tells
        /// its user when the terminal stopped taking output.
        pub fn writer_waiting(&self) -> Option<Duration> {
            self.screen
                .writer()
                .busy_since()
                .map(|since| since.elapsed())
        }

        /// Gives the terminal back, waiting at most a second for it to take the reset.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] when the terminal stopped taking output, so the reset couldn't be
        /// written.
        pub fn close(self) -> Result<()> {
            self.close_with_tail(&[])
        }

        /// Gives the terminal back as [`close`](Self::close) does, then writes `tail` on the main
        /// screen, such as a program's last words. Each line break in it starts its line again
        /// at the left, as it does once the terminal's modes are back.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] when the terminal stopped taking output, so the reset couldn't be
        /// written.
        pub fn close_with_tail(self, tail: &[u8]) -> Result<()> {
            let failure = self.screen.writer().failure();
            // (The tail goes out before raw mode ends, where a line feed moves down alone.)
            if self.screen.close(&crlf(tail), CLOSE_WAIT) {
                return Ok(());
            }
            match failure {
                Some(failure) => Err(Error::Io(failure.into())),
                None => Ok(()),
            }
        }

        fn hold(&self, kind: Hold, command: Vec<u8>) -> Result<()> {
            self.screen
                .writer()
                .hold(kind, command)
                .map_err(|failure| Error::Io(failure.into()))
        }

        fn send(&self, packet: Vec<u8>) -> Result<()> {
            self.screen
                .writer()
                .send(packet)
                .map_err(|failure| Error::Io(failure.into()))
        }
    }

    /// The size of the terminal a program would open, without opening it.
    ///
    /// # Errors
    ///
    /// [`Error::NoTerminal`] when there's no terminal, and [`Error::Io`] when it doesn't report a
    /// size.
    pub fn terminal_size() -> Result<Size> {
        #[cfg(unix)]
        {
            let tty = Tty::open().map_err(Error::NoTerminal)?;
            Ok(window_size(tty.output.as_fd())?.cells)
        }
        #[cfg(windows)]
        {
            let console = WinConsole::open().map_err(Error::NoTerminal)?;
            Ok(console.window()?.size())
        }
    }

    /// The library's decoder, for a terminal whose Backspace sends `erase`.
    fn decoder(erase: u8) -> Decoder {
        Decoder::new(DecoderOptions {
            erase,
            ..DecoderOptions::default()
        })
    }

    /// How long late replies to the probe still decode as replies on Windows, after a probe the
    /// console didn't answer in time, and at most after DA1 for the answers of the terminal behind
    /// a pseudoconsole.
    #[cfg(windows)]
    const LATE_REPLIES: Duration = Duration::from_millis(200);

    /// How long the probe waits on Windows, after DA1 and after each reply that follows it, for
    /// the answers of the terminal behind a pseudoconsole.
    #[cfg(windows)]
    const PASSED_ON_QUIET: Duration = Duration::from_millis(50);

    /// The size to take when the terminal gives none, from `COLUMNS` and `LINES`, or 80x24.
    fn fallback_size(env: &dyn Environment) -> Size {
        let number = |name| {
            env.var(name)
                .and_then(|value| value.trim().parse::<u16>().ok())
                .filter(|&n| n > 0)
        };
        match (number("COLUMNS"), number("LINES")) {
            (Some(width), Some(height)) => Size::new(width, height),
            _ => Size::new(80, 24),
        }
    }

    /// Makes the reset put the cursor's default shape back once a frame asks for another, and not
    /// before, since the shape the user's shell set is theirs.
    fn note_shape(
        enabled: &mut Enabled,
        caps: &Capabilities,
        screen: &Screen,
        cursor: Option<Cursor>,
    ) {
        let shaped = cursor.is_some_and(|cursor| cursor.shape != CursorShape::Default);
        if shaped && !enabled.cursor_shape && caps.cursor_shape != Support::No {
            enabled.cursor_shape = true;
            screen.set_enabled(enabled);
        }
    }

    /// Writes `queries` and waits at most `timeout` for the replies, which DA1's ends. The decoder
    /// first learns of `cursor_reports` cursor reports to expect.
    fn ask(
        screen: &Screen,
        queries: &[u8],
        cursor_reports: usize,
        timeout: Duration,
    ) -> io::Result<Vec<Reply>> {
        let slot = &screen.shared().replies;
        slot.open();
        for _ in 0..cursor_reports {
            screen.expect(Query::CursorPosition);
        }
        screen.writer().send(queries.to_vec())?;
        let answered = slot.wait(timeout, |replies| replies.iter().any(probe::is_end));
        if !answered {
            // (A cursor report that comes later must read as the key it looks like.)
            screen.cancel_expected();
        }
        #[cfg(windows)]
        if answered {
            // (The system's pseudoconsole answers DA1 itself and passes some queries on to the
            // terminal, whose answers come after DA1's.)
            slot.settle(PASSED_ON_QUIET, LATE_REPLIES);
        }
        Ok(slot.close())
    }
}

/// `text` with each line feed that has no carriage return before it given one.
#[cfg(any(unix, windows, test))]
fn crlf(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut previous = None;
    for &byte in text {
        if byte == b'\n' && previous != Some(b'\r') {
            out.push(b'\r');
        }
        out.push(byte);
        previous = Some(byte);
    }
    out
}

/// `bytes` in base64, with padding, for OSC 52.
#[cfg(any(unix, windows, test))]
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|at| chunk.get(at).copied().unwrap_or(0));
        let digits = [
            a >> 2,
            (a & 0b11) << 4 | b >> 4,
            (b & 0b1111) << 2 | c >> 6,
            c & 0b11_1111,
        ];
        for (at, digit) in digits.into_iter().enumerate() {
            out.push(if at <= chunk.len() {
                char::from(ALPHABET[usize::from(digit)])
            } else {
                '='
            });
        }
    }
    out
}

#[cfg(test)]
#[cfg(unix)]
mod tests;

#[cfg(test)]
mod base64_tests {
    use super::{base64, crlf};

    #[test]
    fn line_feeds_get_carriage_returns() {
        assert_eq!(
            crlf(b"one\ntwo\r\nthree\n\n"),
            b"one\r\ntwo\r\nthree\r\n\r\n"
        );
        assert_eq!(crlf(b""), b"");
    }

    #[test]
    fn base64_matches_the_rfc_examples() {
        for (text, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(text.as_bytes()), encoded);
        }
        assert_eq!(base64("é ✓".as_bytes()), "w6kg4pyT");
    }
}
