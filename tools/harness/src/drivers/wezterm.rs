//! WezTerm, under X with its software renderer, read back with `wezterm cli get-text --escapes`.
//!
//! The driver points `XDG_RUNTIME_DIR` at its work directory, so the window's socket lands there
//! and the `cli` commands reach this window and no other.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use tuiframe_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, clean_command, run,
    run_with_input, work_dir, x_screenshot,
};
use crate::drivers::x11;
use crate::readback::{ansi_screen, json_field, set_cursor};

/// WezTerm in a process of its own.
#[derive(Debug)]
pub struct Wezterm {
    dir: PathBuf,
    child: Option<Child>,
    socket: Option<PathBuf>,
    window: Option<String>,
    size: Option<(u16, u16)>,
}

impl Wezterm {
    /// A driver for the `wezterm` on the `PATH`, with xdotool for keys.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Wezterm, Error> {
        Ok(Wezterm {
            dir: work_dir("wezterm")?,
            child: None,
            socket: None,
            window: None,
            size: None,
        })
    }

    fn cli(&self) -> Result<Command, Error> {
        let socket = self
            .socket
            .as_ref()
            .ok_or(Error::Missing("a running WezTerm".to_owned()))?;
        let mut command = clean_command("wezterm");
        command
            .env("XDG_RUNTIME_DIR", &self.dir)
            .env("WEZTERM_UNIX_SOCKET", socket)
            .arg("cli");
        Ok(command)
    }

    /// The window's socket, once WezTerm has made it.
    fn find_socket(&self) -> Result<PathBuf, Error> {
        let dir = self.dir.join("wezterm");
        std::fs::read_dir(&dir)
            .map_err(|error| Error::Io(dir.display().to_string(), error))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("gui-sock-"))
            })
            .ok_or_else(|| Error::Missing("WezTerm's socket".to_owned()))
    }
}

/// The command line that starts WezTerm at a size, with no update check, no close prompt, no tab
/// bar, the software renderer, and the kitty keyboard protocol on.
pub fn start_command(launch: &Launch, runtime_dir: &Path) -> Command {
    let mut command = clean_command("wezterm");
    command.env("XDG_RUNTIME_DIR", runtime_dir);
    for option in [
        format!("initial_cols={}", launch.cols),
        format!("initial_rows={}", launch.rows),
        "check_for_updates=false".to_owned(),
        "window_close_confirmation=\"NeverPrompt\"".to_owned(),
        "front_end=\"Software\"".to_owned(),
        "enable_tab_bar=false".to_owned(),
        "enable_kitty_keyboard=true".to_owned(),
        "audible_bell=\"Disabled\"".to_owned(),
    ] {
        command.arg("--config").arg(option);
    }
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    command
        .args(["start", "--always-new-process", "--cwd"])
        .arg(&launch.cwd)
        .arg("--")
        .args(&launch.command);
    command
}

/// Rebuilds WezTerm's `get-text --escapes` output, with the cursor from `cli list --format json`.
///
/// # Errors
///
/// When the text holds a sequence `tuiframe-vt` doesn't know, or the list gives no cursor.
pub fn parse(text: &[u8], list_json: &str, cols: u16, rows: u16) -> Result<Readback, Error> {
    let width = WidthRules {
        joins_emoji: true,
        ..WidthRules::default()
    };
    let mut screen = ansi_screen(text, cols, rows, width)?;
    let number =
        |field: &str| json_field(list_json, field).and_then(|value| value.parse::<u16>().ok());
    let (Some(x), Some(y)) = (number("cursor_x"), number("cursor_y")) else {
        return Err(Error::Parse("WezTerm's pane list has no cursor".to_owned()));
    };
    let visible = json_field(list_json, "cursor_visibility") != Some("Hidden");
    set_cursor(&mut screen, x, y, visible, None)?;
    let reports = Reports {
        colors: Colors::AsWritten,
        attributes: Attributes::ALL,
        cursor: true,
        wide_text: true,
    };
    Ok(Readback { screen, reports })
}

impl Driver for Wezterm {
    fn name(&self) -> &'static str {
        "wezterm"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("wezterm");
        command.arg("--version");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        x11::require_display()?;
        let mut child = x11::spawn(start_command(launch, &self.dir), &self.dir)?;
        let pid = child.id();
        let ready = x11::wait_until(&mut child, || {
            Ok((self.find_socket()?, x11::window_of(pid).ok()))
        });
        self.child = Some(child);
        let (socket, window) = ready?;
        self.socket = Some(socket);
        self.window = window;
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut command = self.cli()?;
        command.args(["send-text", "--no-paste"]);
        run_with_input(&mut command, Some(bytes)).map(drop)
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        let window = self
            .window
            .as_deref()
            .ok_or(Error::Missing("WezTerm's window".to_owned()))?;
        x11::press(window, keys)
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running WezTerm".to_owned()))?;
        let mut text = self.cli()?;
        text.args(["get-text", "--escapes"]);
        let text = run(&mut text)?;
        let mut list = self.cli()?;
        list.args(["list", "--format", "json"]);
        let list = String::from_utf8_lossy(&run(&mut list)?).into_owned();
        parse(&text, &list, cols, rows)
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        x_screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        self.socket = None;
        self.window = None;
        x11::kill(&mut self.child);
        Ok(())
    }
}

impl Drop for Wezterm {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
