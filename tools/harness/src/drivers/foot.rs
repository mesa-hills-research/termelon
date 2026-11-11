//! foot, under a Wayland compositor (headless sway on CI), read back with `pipe-visible`.
//!
//! The driver's `foot.ini` binds `pipe-visible` to Ctrl+Shift+F12, piping the visible text into a
//! file, and presses that with `wtype`. Keys go through `wtype` too. The readback is text only.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run, work_dir,
};
use crate::drivers::x11;
use crate::keys;
use crate::readback::ansi_screen;

/// How long foot may take to write the screen.
const PIPE_TIMEOUT: Duration = Duration::from_secs(5);

/// foot in a process of its own.
#[derive(Debug)]
pub struct Foot {
    dir: WorkDir,
    child: Option<Child>,
    size: Option<(u16, u16)>,
}

impl Foot {
    /// A driver for the `foot` and `wtype` on the `PATH`.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Foot, Error> {
        Ok(Foot {
            dir: work_dir("foot")?,
            child: None,
            size: None,
        })
    }

    fn screen_file(&self) -> PathBuf {
        self.dir.join("screen.txt")
    }
}

/// The configuration: `pipe-visible` on Ctrl+Shift+F12, writing to `screen_file`.
pub fn config(screen_file: &Path) -> String {
    format!(
        "[key-bindings]\npipe-visible=[sh -c \"cat > '{}'\"] Control+Shift+F12\n",
        screen_file.display()
    )
}

/// The command line that starts foot at a size.
pub fn start_command(launch: &Launch, config_file: &Path) -> Command {
    let mut command = clean_command("foot");
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    command.arg("--config").arg(config_file);
    command.arg(format!(
        "--window-size-chars={}x{}",
        launch.cols, launch.rows
    ));
    command
        .arg("--working-directory")
        .arg(&launch.cwd)
        .args(&launch.command);
    command
}

/// The `wtype` arguments that press a key with its modifiers.
pub fn wtype_args(key: Key, mods: Mods) -> Option<Vec<String>> {
    let name = keys::x_keysym(key)?;
    let held: Vec<&str> = keys::modifier_names(mods)
        .into_iter()
        .map(|m| if m == "super" { "logo" } else { m })
        .collect();
    let mut args = Vec::new();
    for m in &held {
        args.extend(["-M".to_owned(), (*m).to_owned()]);
    }
    args.extend(["-k".to_owned(), name]);
    for m in held.iter().rev() {
        args.extend(["-m".to_owned(), (*m).to_owned()]);
    }
    Some(args)
}

/// Rebuilds the screen from `pipe-visible`'s text, one line per row.
///
/// # Errors
///
/// Never in practice, since the text holds no sequences.
pub fn parse(text: &str, cols: u16, rows: u16) -> Result<Readback, Error> {
    let clean: String = text
        .chars()
        .map(|c| if c != '\n' && c.is_control() { ' ' } else { c })
        .collect();
    let screen = ansi_screen(
        clean.as_bytes(),
        cols,
        rows,
        WidthRules {
            joins_emoji: true,
            ..WidthRules::default()
        },
    )?;
    Ok(Readback {
        raw: Vec::new(),
        screen,
        reports: Reports {
            wide_text: true,
            ..Reports::default()
        },
    })
}

fn wtype(args: &[String]) -> Result<(), Error> {
    let mut command = clean_command("wtype");
    command.args(args);
    run(&mut command).map(drop)
}

impl Driver for Foot {
    fn name(&self) -> &'static str {
        "foot"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("foot");
        command.arg("--version");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(Error::Missing(
                "a Wayland display (WAYLAND_DISPLAY, such as headless sway's)".to_owned(),
            ));
        }
        let config_file = self.dir.join("foot.ini");
        std::fs::write(&config_file, config(&self.screen_file()))
            .map_err(|error| Error::Io("foot.ini".to_owned(), error))?;
        let mut child = x11::spawn(start_command(launch, &config_file), &self.dir)?;
        self.size = Some((launch.cols, launch.rows));
        // foot has no way to say it's up, so the first readback waits for the window.
        let ready = x11::wait_until(&mut child, || {
            self.read_once(Duration::from_millis(500)).map(drop)
        });
        self.child = Some(child);
        ready
    }

    fn send_bytes(&mut self, _bytes: &[u8]) -> Result<(), Error> {
        Err(Error::Unsupported("take raw bytes as input"))
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        for &(key, mods) in keys {
            wtype(&wtype_args(key, mods).ok_or(Error::Unsupported("send that key"))?)?;
        }
        Ok(())
    }

    fn read(&mut self) -> Result<Readback, Error> {
        self.read_once(PIPE_TIMEOUT)
    }

    fn screenshot(&mut self, _path: &Path) -> Result<bool, Error> {
        Ok(false)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        x11::kill(&mut self.child);
        Ok(())
    }
}

impl Foot {
    fn read_once(&self, timeout: Duration) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running foot".to_owned()))?;
        let path = self.screen_file();
        let _ = std::fs::remove_file(&path);
        wtype(&wtype_args(Key::F(12), Mods::CTRL | Mods::SHIFT).unwrap_or_default())?;
        let deadline = Instant::now() + timeout;
        let mut last = None;
        loop {
            let now = std::fs::read_to_string(&path).ok();
            if let Some(text) = now.as_ref().filter(|_| now == last) {
                return Ok(parse(text, cols, rows)?.with_raw("txt", text.clone()));
            }
            if Instant::now() >= deadline {
                return Err(Error::Parse("foot piped no screen".to_owned()));
            }
            last = now;
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Foot {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
