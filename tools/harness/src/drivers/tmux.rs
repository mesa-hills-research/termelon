//! tmux, headless, driven with `send-keys` and read back with `capture-pane -p -e -N`.

use std::path::{Path, PathBuf};
use std::process::Command;

use tuiframe_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, clean_command, run, work_dir,
};
use crate::keys;
use crate::readback::{ansi_screen, set_cursor};

/// The session's name on the harness's own server.
const TARGET: &str = "harness";

/// The server's options. The pane stays when its program ends, so its last screen can still be
/// read, and there's no status line or history.
const CONFIG: &str =
    "set -g remain-on-exit on\nset -g status off\nset -g history-limit 0\nset -g escape-time 0\n";

/// tmux on a server of the harness's own, through a socket in its work directory.
///
/// The readback keeps trailing spaces (`-N`), since without it tmux drops the cells an erase
/// filled with a background colour. It leaves out `-J`, which joins wrapped lines and so changes
/// the rows.
#[derive(Debug)]
pub struct Tmux {
    program: String,
    dir: PathBuf,
    size: Option<(u16, u16)>,
    width: WidthRules,
}

impl Tmux {
    /// A driver for the `tmux` on the `PATH`.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Tmux, Error> {
        Tmux::with_program("tmux")
    }

    /// A driver for a tmux at a path of its own.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn with_program(program: &str) -> Result<Tmux, Error> {
        let dir = work_dir("tmux")?;
        let config = dir.join("tmux.conf");
        std::fs::write(&config, CONFIG)
            .map_err(|error| Error::Io(config.display().to_string(), error))?;
        Ok(Tmux {
            program: program.to_owned(),
            dir,
            size: None,
            width: WidthRules::default(),
        })
    }

    /// Measures readbacks by these width rules, the terminal's.
    pub fn set_width_rules(&mut self, width: WidthRules) {
        self.width = width;
    }

    /// A tmux command on the harness's server.
    pub fn command(&self) -> Command {
        let mut command = clean_command(&self.program);
        command
            .arg("-S")
            .arg(self.dir.join("socket"))
            .arg("-f")
            .arg(self.dir.join("tmux.conf"));
        command
    }

    /// Runs a tmux command on the session and returns its output.
    fn tmux(&self, args: &[&str]) -> Result<Vec<u8>, Error> {
        let mut command = self.command();
        command.args(args);
        run(&mut command)
    }

    /// The pane's terminal device, such as `/dev/pts/7`.
    ///
    /// # Errors
    ///
    /// When tmux can't say.
    pub fn pane_tty(&self) -> Result<String, Error> {
        let out = self.tmux(&["display-message", "-p", "-t", TARGET, "#{pane_tty}"])?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// Whether the pane's program has ended.
    ///
    /// # Errors
    ///
    /// When tmux can't say.
    pub fn pane_dead(&self) -> Result<bool, Error> {
        let out = self.tmux(&["display-message", "-p", "-t", TARGET, "#{pane_dead}"])?;
        Ok(String::from_utf8_lossy(&out).trim() == "1")
    }
}

/// The format of the cursor line [`parse`] reads, for `display-message -p`.
pub const CURSOR_FORMAT: &str =
    "#{cursor_x} #{cursor_y} #{cursor_flag} #{cursor_shape} #{cursor_blinking}";

/// Rebuilds a pane from `capture-pane -p -e -N` and the cursor line [`CURSOR_FORMAT`] gives.
///
/// # Errors
///
/// When the capture holds a sequence `tuiframe-vt` doesn't know, or the cursor line is malformed.
pub fn parse(
    capture: &[u8],
    cursor_line: &str,
    cols: u16,
    rows: u16,
    width: WidthRules,
) -> Result<Readback, Error> {
    let mut screen = ansi_screen(capture, cols, rows, width)?;
    let (x, y, visible, shape) = cursor(cursor_line)?;
    set_cursor(&mut screen, x, y, visible, Some(shape))?;
    let reports = Reports {
        colors: Colors::AsWritten,
        attributes: Attributes::ALL,
        cursor: true,
        wide_text: true,
    };
    Ok(Readback { screen, reports })
}

/// The DECSCUSR number for tmux's cursor shape and blinking.
fn shape(name: &str, blinking: bool) -> u16 {
    let steady = match name {
        "block" => 2,
        "underline" => 4,
        "bar" => 6,
        _ => return 0,
    };
    if blinking { steady - 1 } else { steady }
}

/// Reads `#{cursor_x} #{cursor_y} #{cursor_flag} #{cursor_shape} #{cursor_blinking}`.
fn cursor(info: &str) -> Result<(u16, u16, bool, u16), Error> {
    let fields: Vec<&str> = info.split_whitespace().collect();
    let number = |i: usize| fields.get(i).and_then(|field| field.parse::<u16>().ok());
    match (number(0), number(1), fields.get(2), fields.get(3)) {
        (Some(x), Some(y), Some(flag), Some(name)) => {
            Ok((x, y, *flag == "1", shape(name, fields.get(4) == Some(&"1"))))
        }
        _ => Err(Error::Parse(format!("tmux gave the cursor as {info:?}"))),
    }
}

impl Driver for Tmux {
    fn name(&self) -> &'static str {
        "tmux"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command(&self.program);
        command.arg("-V");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        let mut command = self.command();
        command.args(["new-session", "-d", "-s", TARGET]);
        command
            .arg("-x")
            .arg(launch.cols.to_string())
            .arg("-y")
            .arg(launch.rows.to_string());
        command.arg("-c").arg(&launch.cwd);
        for (key, value) in &launch.env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command.arg("--").args(&launch.command);
        run(&mut command)?;
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for chunk in bytes.chunks(256) {
            let mut command = self.command();
            command.args(["send-keys", "-t", TARGET, "-H"]);
            command.args(chunk.iter().map(|b| format!("{b:02x}")));
            run(&mut command)?;
        }
        Ok(())
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        for &(key, mods) in keys {
            let mut command = self.command();
            command.args(["send-keys", "-t", TARGET]);
            match (keys::tmux(key, mods), key) {
                (Some(name), _) => command.arg(name),
                (None, Key::Char(c)) => command.arg("-l").arg(c.to_string()),
                (None, _) => return Err(Error::Unsupported("send that key")),
            };
            run(&mut command)?;
        }
        Ok(())
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running tmux".to_owned()))?;
        let capture = self.tmux(&["capture-pane", "-p", "-e", "-N", "-t", TARGET])?;
        let info = self.tmux(&["display-message", "-p", "-t", TARGET, CURSOR_FORMAT])?;
        parse(
            &capture,
            &String::from_utf8_lossy(&info),
            cols,
            rows,
            self.width,
        )
    }

    fn screenshot(&mut self, _path: &Path) -> Result<bool, Error> {
        Ok(false)
    }

    fn stop(&mut self) -> Result<(), Error> {
        if self.size.take().is_some() {
            // The server may have ended with its last pane.
            let _ = self.tmux(&["kill-server"]);
        }
        Ok(())
    }
}

impl Drop for Tmux {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_info() {
        assert_eq!(cursor("6 3 1 bar 1").unwrap(), (6, 3, true, 5));
        assert_eq!(cursor("0 0 0 block 0").unwrap(), (0, 0, false, 2));
        assert_eq!(cursor("1 2 1 default 0").unwrap(), (1, 2, true, 0));
        assert!(cursor("x").is_err());
    }
}
