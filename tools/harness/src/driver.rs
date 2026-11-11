//! What every terminal driver does, and the pieces drivers share.

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use termelon_vt::{Emulator, Key, Mods, Mouse, Underline};

/// How long `wait_for` polls between readbacks.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// What to start a terminal with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// The columns the terminal opens with.
    pub cols: u16,
    /// The rows the terminal opens with.
    pub rows: u16,
    /// The program and its arguments, run inside the terminal.
    pub command: Vec<String>,
    /// The directory the program runs in.
    pub cwd: PathBuf,
    /// Variables the program gets beyond the clean environment (`HOME`, `PATH`, `LANG=C.UTF-8`, and
    /// the display's variables).
    pub env: Vec<(String, String)>,
}

impl Launch {
    /// A launch of `command` at a size, in the current directory.
    pub fn new(cols: u16, rows: u16, command: &[&str]) -> Launch {
        Launch {
            cols,
            rows,
            command: command.iter().map(|&arg| arg.to_owned()).collect(),
            cwd: std::env::current_dir().unwrap_or_default(),
            env: Vec::new(),
        }
    }
}

/// A colour's red, green and blue.
pub type Rgb = (u8, u8, u8);

/// How a readback reports colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Colors {
    /// It doesn't.
    #[default]
    None,
    /// As the program wrote them, named, indexed or RGB.
    AsWritten,
    /// Resolved to RGB with reverse video applied, as xterm's dump gives them, with the terminal's
    /// default foreground and background.
    Rgb {
        /// The default foreground.
        foreground: (u8, u8, u8),
        /// The default background.
        background: (u8, u8, u8),
        /// The foreground and background a blinking cell has instead of its own, before reverse
        /// video, where the readback gives blink that way, as xterm's dump does.
        blink: Option<(Rgb, Rgb)>,
        /// Whether bold text in the eight basic colours (SGR 30 to 37) shows their bright versions,
        /// as xterm's `boldColors` draws it by default.
        bold_brightens: bool,
    },
}

/// The attributes a readback reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    /// Bold.
    pub bold: bool,
    /// Dim.
    pub dim: bool,
    /// Italic.
    pub italic: bool,
    /// Underlined at all.
    pub underline: bool,
    /// The underline's style and colour.
    pub underline_style: bool,
    /// The underline styles the readback writes as no underline, as kitty's writes dotted and
    /// dashed.
    pub unreported_underlines: &'static [Underline],
    /// Blinking.
    pub blink: bool,
    /// Reverse video, as an attribute rather than swapped colours.
    pub reverse: bool,
    /// Hidden.
    pub hidden: bool,
    /// Struck through.
    pub strike: bool,
    /// Overlined.
    pub overline: bool,
    /// Hyperlinks.
    pub link: bool,
}

impl Attributes {
    /// Every attribute.
    pub const ALL: Attributes = Attributes {
        bold: true,
        dim: true,
        italic: true,
        underline: true,
        underline_style: true,
        unreported_underlines: &[],
        blink: true,
        reverse: true,
        hidden: true,
        strike: true,
        overline: true,
        link: true,
    };

    /// None.
    pub const NONE: Attributes = Attributes {
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        underline_style: false,
        unreported_underlines: &[],
        blink: false,
        reverse: false,
        hidden: false,
        strike: false,
        overline: false,
        link: false,
    };
}

/// What a terminal's readback can tell about its screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reports {
    /// Its colours.
    pub colors: Colors,
    /// Its attributes.
    pub attributes: Attributes,
    /// The cursor's place and whether it shows.
    pub cursor: bool,
    /// Text past U+00FF. GNU screen's hardcopy keeps one byte per cell.
    pub wide_text: bool,
    /// Whether it drops the blank cells at the end of each row, background colour and all, as
    /// VTE's HTML does.
    pub trims_rows: bool,
}

/// A terminal's screen as it reported it, rebuilt in an emulator, so it can be compared with
/// `termelon-vt`'s screen for the same bytes cell by cell.
#[derive(Debug)]
pub struct Readback {
    /// The screen. Its colours, attributes and cursor mean something only where `reports` says so.
    pub screen: Emulator,
    /// What the terminal reported.
    pub reports: Reports,
    /// What the terminal gave, as it gave it, in parts named for their files, such as `xhtml`.
    pub raw: Vec<(&'static str, Vec<u8>)>,
}

impl Readback {
    /// A readback of a screen, without what it was rebuilt from.
    pub fn new(screen: Emulator, reports: Reports) -> Readback {
        Readback {
            screen,
            reports,
            raw: Vec::new(),
        }
    }

    /// The readback with a part of what the terminal gave.
    #[must_use]
    pub fn with_raw(mut self, name: &'static str, bytes: impl Into<Vec<u8>>) -> Readback {
        self.raw.push((name, bytes.into()));
        self
    }

    /// The rows' text, with trailing spaces trimmed.
    pub fn rows(&self) -> Vec<String> {
        self.screen.rows()
    }

    /// Whether any row holds `marker`.
    pub fn contains(&self, marker: &str) -> bool {
        self.rows().iter().any(|row| row.contains(marker))
    }
}

/// Why a driver failed.
#[derive(Debug)]
pub enum Error {
    /// A program couldn't be started or talked to.
    Io(String, io::Error),
    /// A program ended with a failure.
    Failed {
        /// The command line.
        command: String,
        /// Its exit status.
        status: String,
        /// What it wrote to standard error.
        stderr: String,
    },
    /// The marker didn't show in time. The screen is what the last readback showed.
    Timeout {
        /// The marker waited for.
        marker: String,
        /// The rows of the last readback.
        screen: Vec<String>,
    },
    /// The terminal can't do what was asked.
    Unsupported(&'static str),
    /// A readback couldn't be understood.
    Parse(String),
    /// Something the driver needs is missing, such as a display.
    Missing(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(what, error) => write!(f, "couldn't run {what}: {error}"),
            Error::Failed {
                command,
                status,
                stderr,
            } => {
                write!(f, "{command} failed ({status})")?;
                if !stderr.trim().is_empty() {
                    write!(f, ": {}", stderr.trim())?;
                }
                Ok(())
            }
            Error::Timeout { marker, screen } => {
                write!(
                    f,
                    "`{marker}` didn't show in time, the screen showed {:?}",
                    screen.join("\n").trim()
                )
            }
            Error::Unsupported(what) => write!(f, "this terminal can't {what}"),
            Error::Parse(what) => write!(f, "couldn't read the screen back: {what}"),
            Error::Missing(what) => write!(f, "{what} is missing"),
        }
    }
}

impl std::error::Error for Error {}

/// A real terminal the harness starts, drives and reads back.
pub trait Driver {
    /// The terminal's name, such as `tmux`.
    fn name(&self) -> &'static str;

    /// The terminal's version, as it reports it.
    ///
    /// # Errors
    ///
    /// When the terminal's program can't be run.
    fn version(&self) -> Result<String, Error>;

    /// Starts the terminal at a size, running a command with a clean environment.
    ///
    /// # Errors
    ///
    /// When the terminal doesn't start.
    fn start(&mut self, launch: &Launch) -> Result<(), Error>;

    /// Sends bytes to the program as if typed, without the terminal's key encoder.
    ///
    /// # Errors
    ///
    /// When the terminal can't send raw bytes, or the send fails.
    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error>;

    /// Presses keys, through the terminal's own key encoder.
    ///
    /// # Errors
    ///
    /// When the terminal can't take synthetic keys, or a key has no name there.
    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error>;

    /// Clicks or turns the wheel over a cell, through the terminal's own mouse reports. Terminals
    /// whose tools can't make mouse events can't.
    ///
    /// # Errors
    ///
    /// When the terminal can't take synthetic mouse events, or the event fails.
    fn send_mouse(&mut self, _mouse: Mouse) -> Result<(), Error> {
        Err(Error::Unsupported("take mouse events"))
    }

    /// Reads the screen back.
    ///
    /// # Errors
    ///
    /// When the readback fails or can't be understood.
    fn read(&mut self) -> Result<Readback, Error>;

    /// Saves a screenshot as PNG. Returns false where there's no display to take one of.
    ///
    /// # Errors
    ///
    /// When taking it fails.
    fn screenshot(&mut self, path: &Path) -> Result<bool, Error>;

    /// Stops the terminal. Stopping one that isn't running does nothing.
    ///
    /// # Errors
    ///
    /// When it can't be stopped.
    fn stop(&mut self) -> Result<(), Error>;

    /// Reads the screen back every 100 ms until a row holds `marker`, and returns that readback.
    ///
    /// # Errors
    ///
    /// When `timeout` passes first, with what the screen showed last, or a readback fails.
    fn wait_for(&mut self, marker: &str, timeout: Duration) -> Result<Readback, Error> {
        let deadline = Instant::now() + timeout;
        loop {
            let readback = self.read()?;
            if readback.contains(marker) {
                return Ok(readback);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout {
                    marker: marker.to_owned(),
                    screen: readback.rows(),
                });
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

/// A command with a clean environment: `HOME`, `PATH`, `LANG=C.UTF-8`, and the display's
/// variables where the harness has them. Variables leak from a parent terminal otherwise
/// (`TERM_PROGRAM`, `KITTY_WINDOW_ID`, `TMUX`), and a terminal's answers then look like another's.
pub fn clean_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    for key in [
        "HOME",
        "PATH",
        "DISPLAY",
        "XDG_RUNTIME_DIR",
        "WAYLAND_DISPLAY",
        "LIBGL_ALWAYS_SOFTWARE",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("LANG", "C.UTF-8");
    command
}

/// How a command reads in an error.
pub fn describe(command: &Command) -> String {
    let mut text = command.get_program().to_string_lossy().into_owned();
    for arg in command.get_args() {
        text.push(' ');
        text.push_str(&arg.to_string_lossy());
    }
    text
}

/// Runs a command to its end with no input, and returns its standard output.
///
/// # Errors
///
/// When it can't start or ends with a failure.
pub fn run(command: &mut Command) -> Result<Vec<u8>, Error> {
    run_with_input(command, None)
}

/// Runs a command to its end with bytes on its standard input, and returns its standard output.
///
/// # Errors
///
/// When it can't start or ends with a failure.
pub fn run_with_input(command: &mut Command, input: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    let described = describe(command);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .spawn()
        .map_err(|error| Error::Io(described.clone(), error))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        use std::io::Write as _;
        stdin
            .write_all(input)
            .map_err(|error| Error::Io(described.clone(), error))?;
    }
    let output: Output = child
        .wait_with_output()
        .map_err(|error| Error::Io(described.clone(), error))?;
    if !output.status.success() {
        return Err(Error::Failed {
            command: described,
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(output.stdout)
}

/// Whether a program is on the `PATH`.
pub fn installed(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// A directory of the harness's own for a driver's sockets and files. It's removed when dropped.
#[derive(Debug)]
pub struct WorkDir(PathBuf);

impl std::ops::Deref for WorkDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for WorkDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<OsStr> for WorkDir {
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Makes a fresh [`WorkDir`] in the system's temporary directory.
///
/// # Errors
///
/// When it can't be made.
pub fn work_dir(name: &str) -> Result<WorkDir, Error> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("harness-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|error| Error::Io(dir.display().to_string(), error))?;
    // WezTerm takes it as its runtime directory, which must be the user's alone.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(WorkDir(dir))
}

/// Takes a screenshot of the X display with ImageMagick's `import`, where there is a display.
///
/// # Errors
///
/// When `import` fails.
pub fn x_screenshot(path: &Path) -> Result<bool, Error> {
    if std::env::var_os("DISPLAY").is_none() || !installed("import") {
        return Ok(false);
    }
    let mut command = clean_command("import");
    command.args(["-window", "root"]).arg(path);
    run(&mut command)?;
    Ok(true)
}
