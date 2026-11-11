//! GNU screen, read back with `hardcopy`.
//!
//! A detached screen keeps a window of 80 by 24 whatever it's asked, so the driver runs screen
//! attached inside a tmux pane of the size wanted, and screen takes the pane's size. Keys go to
//! that pane, so screen's own key handling turns them into what the program reads. The hardcopy
//! holds text only, one byte per cell in screen's font, so wide and non-Latin-1 characters aren't
//! read back.

use std::path::Path;
use std::time::{Duration, Instant};

use tuiframe_vt::{Key, Mods};

use crate::driver::{
    Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run, work_dir,
};
use crate::drivers::tmux::Tmux;
use crate::readback::hardcopy_screen;

/// screen's options. The startup message would hide the program until a key.
const CONFIG: &str =
    "startup_message off\ndefutf8 on\nhardstatus off\ndefscrollback 0\nvbell off\n";

/// How long a hardcopy may take to appear.
const HARDCOPY_TIMEOUT: Duration = Duration::from_secs(2);

/// GNU screen inside a tmux pane.
#[derive(Debug)]
pub struct Screen {
    program: String,
    outer: Tmux,
    dir: WorkDir,
    session: String,
    size: Option<(u16, u16)>,
}

impl Screen {
    /// A driver for the `screen` on the `PATH`, inside the `tmux` on the `PATH`.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Screen, Error> {
        let dir = work_dir("screen")?;
        let config = dir.join("screenrc");
        std::fs::write(&config, CONFIG)
            .map_err(|error| Error::Io(config.display().to_string(), error))?;
        let session = format!(
            "harness-{}",
            dir.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
        );
        Ok(Screen {
            program: "screen".to_owned(),
            outer: Tmux::new()?,
            dir,
            session,
            size: None,
        })
    }

    fn screen(&self, args: &[&str]) -> Result<Vec<u8>, Error> {
        let mut command = clean_command(&self.program);
        command.args(["-S", &self.session]).args(args);
        run(&mut command)
    }

    /// Waits until the session answers.
    fn wait_ready(&self) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.screen(&["-X", "info"]) {
                Ok(_) => return Ok(()),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}

/// Text for `stuff`, which reads `^X` as a control and `\` as an escape.
pub fn stuff_text(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == b'\\' || b == b'^' {
            out.push(b'\\');
        }
        out.push(b);
    }
    out
}

/// Rebuilds a window from its hardcopy, text only.
///
/// # Errors
///
/// Never in practice, since a hardcopy holds no sequences.
pub fn parse(hardcopy: &[u8], cols: u16, rows: u16) -> Result<Readback, Error> {
    let reports = Reports {
        wide_text: false,
        ..Reports::default()
    };
    Ok(Readback {
        screen: hardcopy_screen(hardcopy, cols, rows)?,
        reports,
    })
}

impl Driver for Screen {
    fn name(&self) -> &'static str {
        "screen"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command(&self.program);
        command.arg("--version");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        let config = self.dir.join("screenrc").display().to_string();
        // The program shouldn't see that tmux is around it.
        let mut command = vec![
            "env",
            "-u",
            "TMUX",
            "-u",
            "TMUX_PANE",
            &self.program,
            "-S",
            &self.session,
        ];
        command.extend(["-c", &config]);
        command.extend(launch.command.iter().map(String::as_str));
        let outer = Launch {
            command: command.into_iter().map(str::to_owned).collect(),
            ..launch.clone()
        };
        self.outer.start(&outer)?;
        self.size = Some((launch.cols, launch.rows));
        self.wait_ready()
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.contains(&0) {
            return Err(Error::Unsupported("send a NUL byte"));
        }
        let mut command = clean_command(&self.program);
        command.args(["-S", &self.session, "-p", "0", "-X", "stuff"]);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            command.arg(std::ffi::OsStr::from_bytes(&stuff_text(bytes)));
        }
        #[cfg(not(unix))]
        command.arg(String::from_utf8_lossy(&stuff_text(bytes)).into_owned());
        run(&mut command).map(drop)
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        // ctrl+a is screen's command key, which starts a command rather than reaching the program.
        if keys
            .iter()
            .any(|&(key, mods)| key == Key::Char('a') && mods.contains(Mods::CTRL))
        {
            return Err(Error::Unsupported("send ctrl+a, its command key"));
        }
        self.outer.send_keys(keys)
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running screen".to_owned()))?;
        let path = self.dir.join("hardcopy.txt");
        let _ = std::fs::remove_file(&path);
        let target = path.display().to_string();
        self.screen(&["-p", "0", "-X", "hardcopy", &target])?;
        // screen writes the file after the command returns.
        let deadline = Instant::now() + HARDCOPY_TIMEOUT;
        let mut last = None;
        let bytes = loop {
            let now = std::fs::read(&path).ok();
            if now.is_some() && now == last {
                break now.unwrap_or_default();
            }
            if Instant::now() >= deadline {
                return Err(Error::Parse("screen wrote no hardcopy".to_owned()));
            }
            last = now;
            std::thread::sleep(Duration::from_millis(30));
        };
        parse(&bytes, cols, rows)
    }

    fn screenshot(&mut self, _path: &Path) -> Result<bool, Error> {
        Ok(false)
    }

    fn stop(&mut self) -> Result<(), Error> {
        if self.size.take().is_some() {
            let _ = self.screen(&["-X", "quit"]);
        }
        self.outer.stop()
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stuff_escapes_backslash_and_caret() {
        assert_eq!(stuff_text(b"a^M\\b\x1b"), b"a\\^M\\\\b\x1b");
    }
}
