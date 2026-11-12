//! WezTerm, under X with its software renderer, read back with `wezterm cli get-text --escapes`.
//!
//! The driver points `XDG_RUNTIME_DIR` at its work directory, so the window's socket lands there
//! and the `cli` commands reach this window and no other.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run,
    run_with_input, work_dir, x_screenshot,
};
use crate::drivers::x11;
use crate::readback::{ansi_screen, json_field, set_cursor};

/// WezTerm in a process of its own.
#[derive(Debug)]
pub struct Wezterm {
    dir: WorkDir,
    child: Option<Child>,
    socket: Option<PathBuf>,
    /// The pane's id. With no window manager, nothing has the focus for `cli` to pick by.
    pane: Option<String>,
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
            pane: None,
            window: None,
            size: None,
        })
    }

    /// A `cli` command on the pane, such as `get-text`.
    fn pane_command(&self, subcommand: &str) -> Result<Command, Error> {
        let (Some(socket), Some(pane)) = (&self.socket, &self.pane) else {
            return Err(Error::Missing("a running WezTerm".to_owned()));
        };
        let mut command = cli(&self.dir, socket);
        command.args([subcommand, "--pane-id", pane]);
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

/// A `wezterm cli` command for the window listening on `socket`. It never starts a server of its
/// own, which it would do where the window doesn't answer.
pub fn cli(runtime_dir: &Path, socket: &Path) -> Command {
    let mut command = clean_command("wezterm");
    command
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("WEZTERM_UNIX_SOCKET", socket)
        .args(["cli", "--no-auto-start"]);
    command
}

/// The command line that starts WezTerm at a size, with no update check, no close prompt, no tab
/// bar and the software renderer, and its keyboard as it comes, without the kitty keyboard
/// protocol.
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
/// The output comes from termwiz's terminfo renderer, which writes no overline, underline style
/// or underline colour, gives the palette's first sixteen colours as the named ones, and ends a
/// row with `CSI K` while the last background is still set, where WezTerm's cells there have the
/// default one, so those `CSI K` are dropped. WezTerm draws all of these, as its screenshots show.
///
/// # Errors
///
/// When the text holds a sequence `termelon-vt` doesn't know, or the list gives no cursor.
pub fn parse(text: &[u8], list_json: &str, cols: u16, rows: u16) -> Result<Readback, Error> {
    let width = WidthRules {
        joins_emoji: true,
        ..WidthRules::default()
    };
    // The rows are drawn on a blank screen, so dropping `CSI K` keeps the rest of a row blank.
    let text = String::from_utf8_lossy(text).replace("\x1b[K", "");
    let mut screen = ansi_screen(text.as_bytes(), cols, rows, width)?;
    let number =
        |field: &str| json_field(list_json, field).and_then(|value| value.parse::<u16>().ok());
    let (Some(x), Some(y)) = (number("cursor_x"), number("cursor_y")) else {
        return Err(Error::Parse("WezTerm's pane list has no cursor".to_owned()));
    };
    let visible = json_field(list_json, "cursor_visibility") != Some("Hidden");
    set_cursor(&mut screen, x, y, visible, None)?;
    let reports = Reports {
        colors: Colors::AsWritten,
        attributes: Attributes {
            underline_style: false,
            overline: false,
            ..Attributes::ALL
        },
        cursor: true,
        wide_text: true,
        trims_rows: false,
        indexed_as_named: true,
    };
    Ok(Readback::new(screen, reports))
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
        // The socket's file comes before the window listens on it.
        let ready = x11::wait_until(&mut child, &self.dir, || {
            let socket = self.find_socket()?;
            let mut list = cli(&self.dir, &socket);
            list.args(["list", "--format", "json"]);
            let list = String::from_utf8_lossy(&run(&mut list)?).into_owned();
            let pane = json_field(&list, "pane_id")
                .ok_or_else(|| Error::Parse(format!("WezTerm listed no pane, as {list:?}")))?
                .to_owned();
            Ok((socket, pane, x11::window_of(pid)?))
        });
        self.child = Some(child);
        let (socket, pane, window) = ready?;
        self.socket = Some(socket);
        self.pane = Some(pane);
        self.window = Some(window);
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut command = self.pane_command("send-text")?;
        command.arg("--no-paste");
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
        let mut text = self.pane_command("get-text")?;
        text.arg("--escapes");
        let text = run(&mut text)?;
        let Some(socket) = &self.socket else {
            return Err(Error::Missing("a running WezTerm".to_owned()));
        };
        let mut list = cli(&self.dir, socket);
        list.args(["list", "--format", "json"]);
        let list = String::from_utf8_lossy(&run(&mut list)?).into_owned();
        Ok(parse(&text, &list, cols, rows)?
            .with_raw("ansi", text)
            .with_raw("json", list))
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        x_screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        self.socket = None;
        self.pane = None;
        self.window = None;
        x11::kill(&mut self.child);
        Ok(())
    }
}

impl Drop for Wezterm {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
