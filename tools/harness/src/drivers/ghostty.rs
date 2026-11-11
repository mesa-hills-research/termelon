//! Ghostty on macOS, read back with its `write_screen_file` action.
//!
//! The driver binds `write_screen_file:copy` to Ctrl+Shift+F12, which writes the screen to a file
//! and puts the file's path on the clipboard, presses it through System Events, and reads the path
//! with `pbpaste`. Keys go through System Events too, which needs Accessibility for `osascript`
//! (granted on GitHub's macOS runners). The readback is text only.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run, work_dir,
};
use crate::drivers::x11;
use crate::readback::ansi_screen;

/// Where the app's binary is, by default.
pub const APP_BINARY: &str = "/Applications/Ghostty.app/Contents/MacOS/ghostty";

/// How long Ghostty may take to write the screen.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Ghostty in a process of its own.
#[derive(Debug)]
pub struct Ghostty {
    binary: PathBuf,
    dir: WorkDir,
    child: Option<Child>,
    size: Option<(u16, u16)>,
}

impl Ghostty {
    /// A driver for the app in `/Applications`, or the binary `GHOSTTY` names.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Ghostty, Error> {
        let binary =
            std::env::var_os("GHOSTTY").map_or_else(|| PathBuf::from(APP_BINARY), PathBuf::from);
        Ok(Ghostty {
            binary,
            dir: work_dir("ghostty")?,
            child: None,
            size: None,
        })
    }
}

/// The command line that starts Ghostty at a size, with no prompts and the readback's key binding.
pub fn start_command(binary: &Path, launch: &Launch) -> Command {
    let mut command = clean_command(binary);
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    for option in [
        format!("--window-width={}", launch.cols),
        format!("--window-height={}", launch.rows),
        "--confirm-close-surface=false".to_owned(),
        "--quit-after-last-window-closed=true".to_owned(),
        "--window-save-state=never".to_owned(),
        "--keybind=ctrl+shift+f12=write_screen_file:copy".to_owned(),
        format!("--working-directory={}", launch.cwd.display()),
    ] {
        command.arg(option);
    }
    command.arg("-e").args(&launch.command);
    command
}

/// The AppleScript that presses a key in the front app, through System Events.
pub fn keystroke_script(key: Key, mods: Mods) -> Option<String> {
    let mut using = Vec::new();
    for (held, name) in [
        (Mods::CTRL, "control down"),
        (Mods::ALT, "option down"),
        (Mods::SHIFT, "shift down"),
        (Mods::SUPER, "command down"),
    ] {
        if mods.contains(held) {
            using.push(name);
        }
    }
    let using = if using.is_empty() {
        String::new()
    } else {
        format!(" using {{{}}}", using.join(", "))
    };
    let code = match key {
        Key::Char(c) => {
            let escaped = if c == '"' || c == '\\' {
                format!("\\{c}")
            } else {
                c.to_string()
            };
            return Some(format!(
                "tell application \"System Events\" to keystroke \"{escaped}\"{using}"
            ));
        }
        Key::Enter => 36,
        Key::Tab => 48,
        Key::Backspace => 51,
        Key::Escape => 53,
        Key::Left => 123,
        Key::Right => 124,
        Key::Down => 125,
        Key::Up => 126,
        Key::Home => 115,
        Key::End => 119,
        Key::PageUp => 116,
        Key::PageDown => 121,
        Key::Delete => 117,
        Key::F(n) => *[122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111]
            .get(usize::from(n).checked_sub(1)?)?,
        Key::Insert => return None,
    };
    Some(format!(
        "tell application \"System Events\" to key code {code}{using}"
    ))
}

fn osascript(script: &str) -> Result<(), Error> {
    let mut command = clean_command("osascript");
    command.args(["-e", script]);
    run(&mut command).map(drop)
}

/// Rebuilds the screen from the text `write_screen_file` wrote, one line per row.
///
/// # Errors
///
/// Never in practice, since the text holds no sequences.
pub fn parse(text: &str, cols: u16, rows: u16) -> Result<Readback, Error> {
    let clean: String = text
        .chars()
        .map(|c| if c != '\n' && c.is_control() { ' ' } else { c })
        .collect();
    let width = WidthRules {
        joins_emoji: true,
        vs16_widens: true,
        ..WidthRules::default()
    };
    let screen = ansi_screen(clean.as_bytes(), cols, rows, width)?;
    Ok(Readback {
        raw: Vec::new(),
        screen,
        reports: Reports {
            wide_text: true,
            ..Reports::default()
        },
    })
}

impl Driver for Ghostty {
    fn name(&self) -> &'static str {
        "ghostty"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command(&self.binary);
        command.arg("--version");
        let out = String::from_utf8_lossy(&run(&mut command)?).into_owned();
        Ok(out.lines().next().unwrap_or_default().trim().to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        let mut child = x11::spawn(start_command(&self.binary, launch), &self.dir)?;
        self.size = Some((launch.cols, launch.rows));
        let ready = x11::wait_until(&mut child, &self.dir, || {
            self.read_once(Duration::from_secs(1)).map(drop)
        });
        self.child = Some(child);
        ready
    }

    fn send_bytes(&mut self, _bytes: &[u8]) -> Result<(), Error> {
        Err(Error::Unsupported("take raw bytes as input"))
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        for &(key, mods) in keys {
            osascript(&keystroke_script(key, mods).ok_or(Error::Unsupported("send that key"))?)?;
        }
        Ok(())
    }

    fn read(&mut self) -> Result<Readback, Error> {
        self.read_once(WRITE_TIMEOUT)
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        let mut command = clean_command("screencapture");
        command.arg("-x").arg(path);
        run(&mut command).map(|_| true)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        x11::kill(&mut self.child);
        Ok(())
    }
}

impl Ghostty {
    fn read_once(&self, timeout: Duration) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running Ghostty".to_owned()))?;
        // An old path on the clipboard mustn't count as this screen's.
        let mut clear = clean_command("pbcopy");
        crate::driver::run_with_input(&mut clear, Some(b""))?;
        osascript(&keystroke_script(Key::F(12), Mods::CTRL | Mods::SHIFT).unwrap_or_default())?;
        let deadline = Instant::now() + timeout;
        loop {
            let mut paste = clean_command("pbpaste");
            let path = String::from_utf8_lossy(&run(&mut paste)?).trim().to_owned();
            if !path.is_empty()
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                return Ok(parse(&text, cols, rows)?.with_raw("txt", text));
            }
            if Instant::now() >= deadline {
                return Err(Error::Parse("Ghostty wrote no screen file".to_owned()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Ghostty {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
