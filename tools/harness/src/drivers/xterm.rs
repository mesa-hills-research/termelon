//! xterm, under X, read back with its `CSI 10 i` screen dump.
//!
//! The program runs under a small shell wrapper that writes its terminal device to a file, so the
//! driver can write `CSI 10 i` there. xterm then writes `xterm.<date>.xhtml` into its working
//! directory, the driver's own, with the screen's colours as RGB.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run,
    work_dir, x_screenshot,
};
use crate::drivers::x11;
use crate::readback::{ansi_screen, html_dump};

/// How long a dump may take to appear.
const DUMP_TIMEOUT: Duration = Duration::from_secs(5);

/// xterm's colours when it's started with `-fg black -bg white`, which the driver does.
const FOREGROUND: (u8, u8, u8) = (0, 0, 0);
const BACKGROUND: (u8, u8, u8) = (0xff, 0xff, 0xff);

/// xterm with a bitmap font, so its cells don't depend on FreeType.
#[derive(Debug)]
pub struct Xterm {
    dir: WorkDir,
    child: Option<Child>,
    size: Option<(u16, u16)>,
    tty: Option<PathBuf>,
    window: Option<String>,
}

impl Xterm {
    /// A driver for the `xterm` on the `PATH`, with xdotool for keys.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Xterm, Error> {
        Ok(Xterm {
            dir: work_dir("xterm")?,
            child: None,
            size: None,
            tty: None,
            window: None,
        })
    }
}

/// The command line that starts xterm at a size, its program writing its terminal device to
/// `tty_file` first.
pub fn start_command(launch: &Launch, dir: &Path, tty_file: &Path) -> Command {
    let mut command = clean_command("xterm");
    command.current_dir(dir);
    command
        .arg("-geometry")
        .arg(format!("{}x{}", launch.cols, launch.rows));
    command.args(["-fn", "9x15", "-fg", "black", "-bg", "white", "+sb", "-u8"]);
    // Alt sends ESC before the key, rather than setting its eighth bit, as termelon-vt's xterm
    // profile has it.
    for resource in [
        "allowWindowOps: true",
        "allowSendEvents: true",
        "metaSendsEscape: true",
    ] {
        command.arg("-xrm").arg(format!("XTerm*{resource}"));
    }
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    command.args([
        "-e",
        "sh",
        "-c",
        "tty > \"$1\" && cd \"$2\" && shift 2 && exec \"$@\"",
        "sh",
    ]);
    command.arg(tty_file).arg(&launch.cwd).args(&launch.command);
    command
}

/// Rebuilds a screen from xterm's XHTML dump, which `html.c` in its source writes. Every cell has
/// its colours as `rgb()`, bold, italic, underline and strike are classes, each space is a U+00A0
/// (so a U+00A0 on the screen reads back as a space), and a blinking cell is white on red. Bold
/// text in the eight basic colours shows their bright versions.
///
/// # Errors
///
/// When the dump has no `<pre>`.
pub fn parse(html: &str, cols: u16, rows: u16) -> Result<Readback, Error> {
    let dump = html_dump(&html.replace('\u{a0}', " "))?;
    let screen = ansi_screen(&dump.ansi, cols, rows, WidthRules::default())?;
    let colors = Colors::Rgb {
        foreground: dump.foreground.unwrap_or(FOREGROUND),
        background: dump.background.unwrap_or(BACKGROUND),
        blink: Some(((0xff, 0xff, 0xff), (0xff, 0, 0))),
        bold_brightens: true,
    };
    let attributes = Attributes {
        bold: true,
        italic: true,
        underline: true,
        strike: true,
        ..Attributes::NONE
    };
    Ok(Readback {
        raw: Vec::new(),
        screen,
        reports: Reports {
            colors,
            attributes,
            cursor: false,
            wide_text: true,
            trims_rows: false,
        },
    })
}

/// The newest dump in a directory, once its size has settled.
fn take_dump(dir: &Path, before: &[PathBuf]) -> Result<String, Error> {
    let deadline = Instant::now() + DUMP_TIMEOUT;
    let mut last: Option<u64> = None;
    loop {
        let dumps: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|error| Error::Io(dir.display().to_string(), error))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "xhtml") && !before.contains(path)
            })
            .collect();
        if let Some(path) = dumps.first() {
            let size = std::fs::metadata(path).map(|meta| meta.len()).ok();
            if size.is_some() && size == last {
                let html = std::fs::read_to_string(path)
                    .map_err(|error| Error::Io(path.display().to_string(), error))?;
                let _ = std::fs::remove_file(path);
                return Ok(html);
            }
            last = size;
        }
        if Instant::now() >= deadline {
            return Err(Error::Parse("xterm wrote no screen dump".to_owned()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Driver for Xterm {
    fn name(&self) -> &'static str {
        "xterm"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("xterm");
        command.arg("-version");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        x11::require_display()?;
        let tty_file = self.dir.join("tty");
        let _ = std::fs::remove_file(&tty_file);
        let mut child = x11::spawn(start_command(launch, &self.dir, &tty_file), &self.dir)?;
        let pid = child.id();
        let ready = x11::wait_until(&mut child, &self.dir, || {
            let tty = std::fs::read_to_string(&tty_file)
                .map_err(|error| Error::Io("the tty file".to_owned(), error))?;
            let window = x11::window_of(pid)?;
            Ok((PathBuf::from(tty.trim()), window))
        });
        self.child = Some(child);
        let (tty, window) = ready?;
        self.tty = Some(tty);
        self.window = Some(window);
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, _bytes: &[u8]) -> Result<(), Error> {
        Err(Error::Unsupported("take raw bytes as input"))
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        let window = self
            .window
            .as_deref()
            .ok_or(Error::Missing("a running xterm".to_owned()))?;
        x11::press(window, keys)
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running xterm".to_owned()))?;
        let tty = self
            .tty
            .as_ref()
            .ok_or(Error::Missing("xterm's terminal device".to_owned()))?;
        let before: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .collect()
            })
            .unwrap_or_default();
        // Written to the terminal device, the request reaches xterm as output from the program.
        std::fs::OpenOptions::new()
            .write(true)
            .open(tty)
            .and_then(|mut file| std::io::Write::write_all(&mut file, b"\x1b[10i"))
            .map_err(|error| Error::Io(tty.display().to_string(), error))?;
        let html = take_dump(&self.dir, &before)?;
        Ok(parse(&html, cols, rows)?.with_raw("xhtml", html))
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        x_screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        self.tty = None;
        self.window = None;
        x11::kill(&mut self.child);
        Ok(())
    }
}

impl Drop for Xterm {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
