//! kitty, under X with software rendering, driven and read back through its remote control.

use std::path::Path;
use std::process::{Child, Command};

use tuiframe_vt::{Key, Mods, Profile, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run,
    run_with_input, work_dir, x_screenshot,
};
use crate::drivers::x11;
use crate::keys;
use crate::readback::ansi_screen;

/// kitty, listening for remote control on a socket in its work directory.
#[derive(Debug)]
pub struct Kitty {
    dir: WorkDir,
    child: Option<Child>,
    size: Option<(u16, u16)>,
    width: WidthRules,
}

impl Kitty {
    /// A driver for the `kitty` and `kitten` on the `PATH`.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Kitty, Error> {
        Ok(Kitty {
            dir: work_dir("kitty")?,
            child: None,
            size: None,
            width: Profile::kitty().width,
        })
    }

    fn socket(&self) -> String {
        format!("unix:{}", self.dir.join("kitty.sock").display())
    }

    fn kitten(&self) -> Command {
        let mut command = clean_command("kitten");
        command.args(["@", "--to", &self.socket()]);
        command
    }
}

/// The command line that starts kitty at a size, with remote control on `socket`.
pub fn start_command(launch: &Launch, socket: &str) -> Command {
    let mut command = clean_command("kitty");
    command
        .args(["--config", "NONE", "--listen-on", socket, "--directory"])
        .arg(&launch.cwd);
    for option in [
        "allow_remote_control=yes".to_owned(),
        format!("initial_window_width={}c", launch.cols),
        format!("initial_window_height={}c", launch.rows),
        "remember_window_size=no".to_owned(),
        "confirm_os_window_close=0".to_owned(),
        "enable_audio_bell=no".to_owned(),
        "placement_strategy=top-left".to_owned(),
        "window_padding_width=0".to_owned(),
    ] {
        command.arg("-o").arg(option);
    }
    command.env("LIBGL_ALWAYS_SOFTWARE", "1");
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    command.args(&launch.command);
    command
}

/// The arguments of the readback: the screen with colours and attributes, then the cursor.
pub const GET_TEXT: [&str; 5] = ["get-text", "--extent", "screen", "--ansi", "--add-cursor"];

/// Rebuilds kitty's `get-text --ansi --add-cursor` output, whose last line ends with the cursor's
/// place, visibility and shape as escape sequences.
///
/// # Errors
///
/// When the output holds a sequence `tuiframe-vt` doesn't know.
pub fn parse(text: &[u8], cols: u16, rows: u16, width: WidthRules) -> Result<Readback, Error> {
    let reports = Reports {
        colors: Colors::AsWritten,
        attributes: Attributes::ALL,
        cursor: true,
        wide_text: true,
        trims_rows: false,
    };
    Ok(Readback {
        screen: ansi_screen(text, cols, rows, width)?,
        reports,
    })
}

impl Driver for Kitty {
    fn name(&self) -> &'static str {
        "kitty"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("kitty");
        command.arg("--version");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        x11::require_display()?;
        let mut child = x11::spawn(start_command(launch, &self.socket()), &self.dir)?;
        let ready = x11::wait_until(&mut child, || {
            let mut command = self.kitten();
            command.arg("ls");
            run(&mut command)
        });
        self.child = Some(child);
        ready?;
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut command = self.kitten();
        command.args(["send-text", "--stdin"]);
        run_with_input(&mut command, Some(bytes)).map(drop)
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        let names = keys
            .iter()
            .map(|&(key, mods)| keys::kitty(key, mods).ok_or(Error::Unsupported("send that key")))
            .collect::<Result<Vec<_>, _>>()?;
        let mut command = self.kitten();
        command.arg("send-key").args(names);
        run(&mut command).map(drop)
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running kitty".to_owned()))?;
        let mut command = self.kitten();
        command.args(GET_TEXT);
        parse(&run(&mut command)?, cols, rows, self.width)
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        x_screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        x11::kill(&mut self.child);
        Ok(())
    }
}

impl Drop for Kitty {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
