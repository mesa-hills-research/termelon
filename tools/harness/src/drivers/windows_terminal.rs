//! Windows Terminal, started with `--size` and read back through its `exportBuffer` action.
//!
//! The driver writes Windows Terminal's settings with a key for `exportBuffer` and puts back the
//! ones it found when it's dropped. It opens a new window running `harness-console launch`,
//! which reports its process id and runs the program. To read the screen it brings the window to
//! the front, presses that key with `SendInput`, and reads the file the action writes. Keys go
//! the same way, through Windows Terminal's own encoder, and bytes through a `harness-console`
//! attached to the tab's console, as characters with no key.
//!
//! `HARNESS_WINDOWS_TERMINAL` names the `WindowsTerminal.exe` of a portable build, whose settings
//! are in the `settings` folder beside it. Without it the driver runs the installed one, `wt.exe`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, WorkDir, describe, work_dir,
};
use crate::drivers::console::{
    self, CREATE_NO_WINDOW, attach, chord, clean_environment, helper_program, kill_tree,
    launch_args, require_windows, set_flags,
};
use crate::drivers::lines::Helper;
use crate::drivers::xtermjs::hex;
use crate::json::Json;
use crate::readback::ansi_screen;

/// The variable that names a portable build's `WindowsTerminal.exe`.
pub const PORTABLE: &str = "HARNESS_WINDOWS_TERMINAL";

/// The class of Windows Terminal's windows.
const WINDOW_CLASS: &str = "CASCADIA_HOSTING_WINDOW_CLASS";

/// The key the settings bind `exportBuffer` to, and the virtual keys that press it.
const EXPORT_KEYS: &str = "ctrl+shift+f9";
const EXPORT_CHORD: [u16; 3] = [0x11, 0x10, 0x78];

/// How long the program may take to start in the new window, and the export to be written.
const START_TIMEOUT: Duration = Duration::from_secs(20);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(3);

/// Windows Terminal, a window per program.
#[derive(Debug)]
pub struct WindowsTerminal {
    dir: WorkDir,
    terminal: Option<Child>,
    launcher: Option<u32>,
    helper: Option<Helper>,
    size: Option<(u16, u16)>,
    /// The settings file, and what it held before the driver wrote its own (`None` for none).
    settings: Option<(PathBuf, Option<Vec<u8>>)>,
}

impl WindowsTerminal {
    /// A driver for the portable build `HARNESS_WINDOWS_TERMINAL` names, or the installed one.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<WindowsTerminal, Error> {
        Ok(WindowsTerminal {
            dir: work_dir("windows-terminal")?,
            terminal: None,
            launcher: None,
            helper: None,
            size: None,
            settings: None,
        })
    }

    fn export_path(&self) -> PathBuf {
        self.dir.join("export.txt")
    }

    /// Writes the driver's settings, keeping what the file held the first time.
    fn write_settings(&mut self) -> Result<(), Error> {
        let path = settings_path()?;
        if self.settings.is_none() {
            self.settings = Some((path.clone(), std::fs::read(&path).ok()));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| Error::Io(parent.display().to_string(), error))?;
        }
        std::fs::write(&path, settings(&self.export_path()))
            .map_err(|error| Error::Io(path.display().to_string(), error))
    }

    fn helper(&mut self) -> Result<&mut Helper, Error> {
        self.helper
            .as_mut()
            .ok_or(Error::Missing("a running Windows Terminal".to_owned()))
    }
}

/// The portable build's `WindowsTerminal.exe`, if `HARNESS_WINDOWS_TERMINAL` names one.
fn portable() -> Option<PathBuf> {
    std::env::var_os(PORTABLE).map(PathBuf::from)
}

/// Where the settings of the Windows Terminal the driver runs are.
fn settings_path() -> Result<PathBuf, Error> {
    if let Some(exe) = portable() {
        let dir = exe
            .parent()
            .ok_or_else(|| Error::Missing(format!("the folder of {}", exe.display())))?;
        return Ok(dir.join("settings").join("settings.json"));
    }
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Error::Missing("LOCALAPPDATA".to_owned()))?;
    Ok(PathBuf::from(local)
        .join("Packages")
        .join("Microsoft.WindowsTerminal_8wekyb3d8bbwe")
        .join("LocalState")
        .join("settings.json"))
}

/// The settings the driver runs Windows Terminal with: tabs that close with their program, no
/// animations or confirmations, and `exportBuffer` to `export` on a key.
pub fn settings(export: &Path) -> String {
    Json::object([
        (
            "$help",
            Json::str(
                "Written by termelon's harness, which puts back what was here when it's done.",
            ),
        ),
        ("confirmCloseAllTabs", Json::Bool(false)),
        ("disableAnimations", Json::Bool(true)),
        ("firstWindowPreference", Json::str("defaultProfile")),
        ("startOnUserLogin", Json::Bool(false)),
        ("windowingBehavior", Json::str("useNew")),
        (
            "profiles",
            Json::object([(
                "defaults",
                Json::object([
                    ("bellStyle", Json::str("none")),
                    ("closeOnExit", Json::str("always")),
                    ("padding", Json::str("0")),
                    ("scrollbarState", Json::str("hidden")),
                ]),
            )]),
        ),
        (
            "actions",
            Json::List(vec![Json::object([
                (
                    "command",
                    Json::object([
                        ("action", Json::str("exportBuffer")),
                        ("path", Json::str(export.display().to_string())),
                    ]),
                ),
                ("keys", Json::str(EXPORT_KEYS)),
            ])]),
        ),
    ])
    .to_text()
}

/// Rebuilds the screen from what `exportBuffer` wrote, the buffer's text a line per row. The
/// last `rows` lines are the screen, where there are more.
///
/// # Errors
///
/// Never in practice, since the text holds no sequences.
pub fn parse_export(text: &str, cols: u16, rows: u16) -> Result<Readback, Error> {
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            line.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect()
        })
        .collect();
    let screen = lines
        .iter()
        .skip(lines.len().saturating_sub(usize::from(rows)))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Readback {
        screen: ansi_screen(screen.as_bytes(), cols, rows, WidthRules::default())?,
        reports: Reports {
            colors: Colors::None,
            attributes: Attributes::NONE,
            cursor: false,
            wide_text: true,
            trims_rows: true,
        },
    })
}

/// Waits until `path` exists and has stopped growing, at most `timeout`, and reads it.
fn read_when_written(path: &Path, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    let mut last = None;
    while Instant::now() < deadline {
        let size = std::fs::metadata(path).ok().map(|meta| meta.len());
        if size.is_some() && size == last {
            return std::fs::read(path)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        }
        last = size;
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Presses `chord` in Windows Terminal's window.
fn press(chord: &[u16]) -> Result<(), Error> {
    #[cfg(windows)]
    {
        let pressed = crate::windows::press_in_window(WINDOW_CLASS, chord)
            .map_err(|error| Error::Io("SendInput".to_owned(), error))?;
        if pressed {
            Ok(())
        } else {
            Err(Error::Missing(
                "Windows Terminal's window, in front".to_owned(),
            ))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (WINDOW_CLASS, chord);
        require_windows()
    }
}

impl Driver for WindowsTerminal {
    fn name(&self) -> &'static str {
        "windows-terminal"
    }

    fn version(&self) -> Result<String, Error> {
        require_windows()?;
        let script = if portable().is_some() {
            format!("(Get-Item $env:{PORTABLE}).VersionInfo.ProductVersion")
        } else {
            "(Get-AppxPackage Microsoft.WindowsTerminal).Version".to_owned()
        };
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        set_flags(&mut command, CREATE_NO_WINDOW);
        console::first_line(&mut command)
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        require_windows()?;
        self.write_settings()?;
        let pid_file = self.dir.join("launcher.pid");
        let _ = std::fs::remove_file(&pid_file);
        let mut command = Command::new(portable().unwrap_or_else(|| PathBuf::from("wt.exe")));
        command
            .args(["-w", "new", "--size"])
            .arg(format!("{},{}", launch.cols, launch.rows))
            .arg("-d")
            .arg(&launch.cwd)
            .arg(helper_program()?)
            .args(launch_args(launch, None, Some(&pid_file)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        clean_environment(&mut command);
        command.envs(launch.env.iter().map(|(key, value)| (key, value)));
        let described = describe(&command);
        self.terminal = Some(
            command
                .spawn()
                .map_err(|error| Error::Io(described, error))?,
        );
        let started = Instant::now();
        let pid = loop {
            let written = std::fs::read_to_string(&pid_file).ok();
            if let Some(pid) = written.and_then(|text| text.trim().parse::<u32>().ok()) {
                break pid;
            }
            if started.elapsed() > START_TIMEOUT {
                return Err(Error::Missing(
                    "the program in Windows Terminal's new window".to_owned(),
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        self.launcher = Some(pid);
        self.helper = Some(attach(pid, &self.dir.join("helper.log"))?);
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let reply = self
            .helper()?
            .request(&Json::object([("send", Json::Str(hex(bytes)))]))?;
        match reply.get("error").and_then(Json::as_str) {
            Some(error) => Err(Error::Parse(format!("the console helper said {error}"))),
            None => Ok(()),
        }
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        for &(key, mods) in keys {
            press(&chord(key, mods).ok_or(Error::Unsupported("send that key"))?)?;
        }
        Ok(())
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running Windows Terminal".to_owned()))?;
        let export = self.export_path();
        let _ = std::fs::remove_file(&export);
        press(&EXPORT_CHORD)?;
        let text = read_when_written(&export, EXPORT_TIMEOUT).ok_or_else(|| {
            Error::Missing(format!(
                "the buffer exportBuffer writes to {}",
                export.display()
            ))
        })?;
        parse_export(&text, cols, rows)
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        console::screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        if let Some(helper) = self.helper.take() {
            helper.stop();
        }
        if let Some(pid) = self.launcher.take() {
            kill_tree(pid);
        }
        if let Some(mut terminal) = self.terminal.take() {
            let _ = terminal.kill();
            let _ = terminal.wait();
        }
        Ok(())
    }
}

impl Drop for WindowsTerminal {
    fn drop(&mut self) {
        let _ = self.stop();
        match self.settings.take() {
            Some((path, Some(found))) => {
                let _ = std::fs::write(path, found);
            }
            Some((path, None)) => {
                let _ = std::fs::remove_file(path);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_bind_the_export_to_its_key() {
        let text = settings(Path::new("C:\\Users\\ada\\export.txt"));
        let json = Json::parse(&text).unwrap();
        let action = json
            .get("actions")
            .and_then(|actions| actions.at(0))
            .unwrap();
        assert_eq!(action.get("keys").and_then(Json::as_str), Some(EXPORT_KEYS));
        let command = action.get("command").unwrap();
        assert_eq!(
            command.get("action").and_then(Json::as_str),
            Some("exportBuffer")
        );
        assert_eq!(
            command.get("path").and_then(Json::as_str),
            Some("C:\\Users\\ada\\export.txt")
        );
        assert_eq!(
            chord(Key::F(9), Mods::CTRL | Mods::SHIFT).as_deref(),
            Some(&EXPORT_CHORD[..])
        );
    }

    #[test]
    fn an_export_s_last_lines_are_the_screen() {
        let readback = parse_export("scrolled away\r\nfirst\r\nsecond\r\n", 10, 2).unwrap();
        assert_eq!(readback.rows(), ["first", "second"]);
        let short = parse_export("only\r\n", 10, 3).unwrap();
        assert_eq!(short.rows(), ["only", "", ""]);
    }
}
