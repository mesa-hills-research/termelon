//! The terminal as a program has it, [`Terminal`], opened with [`Options`], and its [`Events`].

mod events;

pub use events::{Events, NextEvent};

use std::fmt;
use std::io;
use std::time::Duration;

use crate::caps::Capabilities;
use crate::output::CompletedFrame;

/// What [`Terminal::draw`] did.
#[derive(Debug)]
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
    /// The protocol's first flag, which tells apart keys the legacy encodings merge, such as
    /// Shift+Enter and Enter, or Ctrl+I and Tab.
    #[default]
    Disambiguate,
    /// These flags, of those the terminal has, as in kitty's `CSI > flags u`.
    Flags(u8),
}

/// The choices made at open. The defaults suit most full-screen programs.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// How long the startup probe waits for a terminal that doesn't answer. 300 ms by default.
    pub probe_timeout: Duration,
    /// Measures how the terminal draws emoji sequences and East Asian ambiguous characters, which
    /// costs a second round trip at open. Off by default.
    pub width_probes: bool,
    /// How long a lone ESC waits for a following byte before it's the Esc key. Zero by default,
    /// which makes it Esc once nothing more is ready.
    pub escape_timeout: Duration,
    /// Skips the probe and uses these, for tests and setups the probe gets wrong.
    pub capabilities: Option<Capabilities>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mouse: MouseMode::ButtonsAndDrags,
            bracketed_paste: true,
            focus_events: true,
            keyboard: KeyboardRequest::Disambiguate,
            save_title: true,
            probe_timeout: Duration::from_millis(300),
            width_probes: false,
            escape_timeout: Duration::ZERO,
            capabilities: None,
        }
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

#[cfg(unix)]
pub use unix::{Terminal, terminal_size};

#[cfg(unix)]
mod unix {
    use std::future::Future;
    use std::io;
    use std::os::fd::AsFd;
    use std::time::Duration;

    use super::{Drawn, Error, Events, KeyboardRequest, MouseMode, Options, Result, base64};
    use crate::buffer::Buffer;
    use crate::caps::env::Environment;
    use crate::caps::{self, Capabilities, KeyboardSupport, Support, probe};
    use crate::event::decode::{Decoder, DecoderOptions, Query, Reply};
    use crate::frame::Frame;
    use crate::layout::Rect;
    use crate::layout::Size;
    use crate::output::encode::Encoder;
    use crate::output::{CompletedFrame, Cursor};
    use crate::restore::screen::Screen;
    use crate::restore::{CLOSE_WAIT, Enabled, RestoreHandle, mouse_switch};
    use crate::sys::unix::{Signals, Tty, window_size};

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
        encoder: Encoder,
        /// What the terminal shows once every packet so far is written.
        front: Buffer,
        /// What the next frame draws into.
        back: Buffer,
        /// Where the last frame put the cursor.
        cursor: Option<Cursor>,
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
            let tty = Tty::open().map_err(Error::NoTerminal)?;
            let terminal = Self::open_on(tty, options, &caps::env::Process, Signals::register()?)?;
            // (Text measures as the terminal draws it from now on.)
            crate::text::set_width_profile(terminal.caps.width_profile);
            Ok(terminal)
        }

        /// [`open`](Self::open), on `tty`, with `env` for the environment and `signals` for the
        /// reader.
        pub(crate) fn open_on(
            tty: Tty,
            options: Options,
            env: &dyn Environment,
            signals: Signals,
        ) -> Result<Self> {
            let screen = Screen::open_with(
                tty,
                |erase| Decoder::new(DecoderOptions { erase }),
                options.escape_timeout,
                signals,
            )?;
            let overrides = env.var("TUIFRAME_CAPS");
            let (mut caps, linux_console) = if let Some(caps) = &options.capabilities {
                (caps.clone(), false)
            } else {
                let hints = caps::env::hints(env);
                let queries = probe::queries(hints.linux_console);
                let replies = ask(&screen, &queries, 0, options.probe_timeout)?;
                let answers = probe::answers(&replies);
                let caps = caps::detect(&answers, &hints, overrides.as_deref());
                (caps, hints.linux_console)
            };
            let enabled = enabled(&options, &caps);
            screen.enable(&enabled)?;
            // (So the first frame isn't held behind the setup.)
            let _ = screen
                .writer()
                .wait(screen.writer().ticket(), options.probe_timeout);
            if options.width_probes && options.capabilities.is_none() && !linux_console {
                let samples = probe::WIDTH_SAMPLES.len();
                let replies = ask(
                    &screen,
                    &probe::width_queries(),
                    samples,
                    options.probe_timeout,
                )?;
                if let Some(profile) = probe::width_profile(&replies) {
                    caps.width_profile = profile;
                    if let Some(overrides) = &overrides {
                        caps::apply_overrides(&mut caps, overrides);
                    }
                }
            }
            let area = Rect::from(screen.shared().size());
            Ok(Self {
                front: Buffer::with_width_profile(area, caps.width_profile),
                back: Buffer::with_width_profile(area, caps.width_profile),
                encoder: Encoder::new(),
                cursor: None,
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
            let area = Rect::from(self.screen.shared().size());
            if area != self.front.area() {
                self.front.resize(area);
                self.back.resize(area);
                self.screen.shared().request_redraw();
            }
            self.back.reset();
            let mut frame = Frame::new(&mut self.back, None);
            render(&mut frame);
            let cursor = frame.into_cursor();
            let full = self.screen.shared().take_redraw();
            if full {
                self.encoder.redraw();
            }
            let mut packet = writer.spare_buffer();
            let stats = self.encoder.encode(
                &self.front,
                &self.back,
                cursor.as_ref(),
                &self.caps,
                &mut packet,
            );
            if !packet.is_empty()
                && let Err(failure) = writer.send(packet)
            {
                if full {
                    self.screen.shared().request_redraw();
                }
                return Err(Error::Io(failure.into()));
            }
            std::mem::swap(&mut self.front, &mut self.back);
            self.cursor = cursor;
            Ok(Drawn::Written(CompletedFrame::new(
                &self.front,
                cursor,
                stats,
            )))
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
            let shared = self.screen.shared();
            if shared.redraw_pending() || Rect::from(shared.size()) != self.front.area() {
                return Ok(Drawn::FullFrameDue);
            }
            self.screen.draw_here();
            // (The back buffer keeps the cells as they were, and the next full frame resets it.)
            self.back.clone_from(&self.front);
            self.front.set_clip(regions);
            let mut frame = Frame::new(&mut self.front, self.cursor);
            render(&mut frame);
            let cursor = frame.into_cursor();
            self.front.clear_clip();
            let mut packet = writer.spare_buffer();
            let stats = self.encoder.encode_regions(
                &self.back,
                &self.front,
                regions,
                cursor.as_ref(),
                &self.caps,
                &mut packet,
            );
            if !packet.is_empty()
                && let Err(failure) = writer.send(packet)
            {
                // (The terminal didn't get what the front buffer now holds.)
                self.screen.shared().request_redraw();
                return Err(Error::Io(failure.into()));
            }
            self.cursor = cursor;
            Ok(Drawn::Written(CompletedFrame::new(
                &self.front,
                cursor,
                stats,
            )))
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

        /// Sets the window's title with OSC 0, where the terminal is known to take it, and does
        /// nothing elsewhere. Control characters in `title` are left out.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn set_title(&mut self, title: &str) -> Result<()> {
            if self.caps.title != Support::Yes {
                return Ok(());
            }
            let title: String = title.chars().filter(|c| !c.is_control()).collect();
            self.send(format!("\x1b]0;{title}\x1b\\").into_bytes())
        }

        /// Rings the bell.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn bell(&mut self) -> Result<()> {
            self.send(vec![0x07])
        }

        /// Copies `text` to the clipboard with OSC 52, where the terminal is known to take it, and
        /// does nothing elsewhere. Inside tmux, the copy reaches the outer terminal only with
        /// tmux's `set-clipboard` on.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] once the terminal stopped taking output.
        pub fn copy_to_clipboard(&mut self, text: &str) -> Result<()> {
            if self.caps.clipboard_write != Support::Yes {
                return Ok(());
            }
            let encoded = base64(text.as_bytes());
            self.send(format!("\x1b]52;c;{encoded}\x1b\\").into_bytes())
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

        /// Why the terminal takes no more output, once it doesn't. A write that failed gives its
        /// error's kind, and a writer thread that ended gives [`io::ErrorKind::Other`]. The events
        /// end then, so the program ends as when its terminal goes away.
        pub fn failure(&self) -> Option<io::ErrorKind> {
            self.screen
                .writer()
                .failure()
                .map(|failure| io::Error::from(failure).kind())
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
        /// screen, such as a program's last words.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] when the terminal stopped taking output, so the reset couldn't be
        /// written.
        pub fn close_with_tail(self, tail: &[u8]) -> Result<()> {
            let failure = self.screen.writer().failure();
            if self.screen.close(tail, CLOSE_WAIT) {
                return Ok(());
            }
            match failure {
                Some(failure) => Err(Error::Io(failure.into())),
                None => Ok(()),
            }
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
        let tty = Tty::open().map_err(Error::NoTerminal)?;
        Ok(window_size(tty.output.as_fd())?.cells)
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
        slot.wait(timeout, |replies| replies.iter().any(probe::is_end));
        Ok(slot.close())
    }

    /// What to turn on for `options` in a terminal with `caps`. Private modes go out where support
    /// is unknown, since terminals ignore modes they don't know.
    fn enabled(options: &Options, caps: &Capabilities) -> Enabled {
        let has = |support: Support| support != Support::No;
        let keyboard = match (options.keyboard, caps.keyboard) {
            (KeyboardRequest::Disambiguate, KeyboardSupport::Kitty { flags }) => Some(flags & 1),
            (KeyboardRequest::Flags(asked), KeyboardSupport::Kitty { flags }) => {
                Some(flags & asked)
            }
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
            save_title: options.save_title && has(caps.title_stack),
            hyperlinks: caps.hyperlinks == Support::Yes,
            cursor_shape: has(caps.cursor_shape),
        }
    }
}

/// `bytes` in base64, with padding, for OSC 52.
#[cfg(any(unix, test))]
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
    use super::base64;

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
