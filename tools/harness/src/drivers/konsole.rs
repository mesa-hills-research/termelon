//! Konsole, under X, read back over D-Bus with `getAllDisplayedText`.
//!
//! The method exists from Konsole 24.02, and from 25.12 it answers only with
//! `EnableSecuritySensitiveDBusAPI` on, which the driver's own `konsolerc` sets. Konsole needs a
//! D-Bus session bus (`dbus-run-session` on CI). The readback is text only.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use tuiframe_vt::{Key, Mods};

use crate::driver::{
    Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run, work_dir, x_screenshot,
};
use crate::drivers::x11;
use crate::readback::ansi_screen;

/// The settings the driver gives Konsole, in its own configuration directory.
pub const KONSOLERC: &str = "[General]\nEnableSecuritySensitiveDBusAPI=true\n\n[KonsoleWindow]\nEnableSecuritySensitiveDBusAPI=true\n\
                             ShowWindowTitleOnTitleBar=false\n\n[Notification Messages]\nCloseAllTabs=true\n";

/// Konsole in a process of its own.
#[derive(Debug)]
pub struct Konsole {
    dir: WorkDir,
    child: Option<Child>,
    window: Option<String>,
    size: Option<(u16, u16)>,
}

impl Konsole {
    /// A driver for the `konsole` and `gdbus` on the `PATH`, with xdotool for keys.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Konsole, Error> {
        Ok(Konsole {
            dir: work_dir("konsole")?,
            child: None,
            window: None,
            size: None,
        })
    }

    fn config_home(&self) -> PathBuf {
        self.dir.join("config")
    }

    /// A call to a method of the first session, with GVariant arguments.
    fn call(&self, method: &str, args: &[&str]) -> Result<String, Error> {
        let pid = self
            .child
            .as_ref()
            .ok_or(Error::Missing("a running Konsole".to_owned()))?
            .id();
        let mut command = clean_command("gdbus");
        command.args([
            "call",
            "--session",
            "--dest",
            &format!("org.kde.konsole-{pid}"),
            "--object-path",
            "/Sessions/1",
        ]);
        command
            .arg("--method")
            .arg(format!("org.kde.konsole.Session.{method}"))
            .args(args);
        Ok(String::from_utf8_lossy(&run(&mut command)?).into_owned())
    }
}

/// The command line that starts Konsole at a size, in a process of its own.
pub fn start_command(launch: &Launch, config_home: &Path) -> Command {
    let mut command = clean_command("konsole");
    command.env("XDG_CONFIG_HOME", config_home);
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    command
        .args([
            "--separate",
            "--nofork",
            "--hide-menubar",
            "--hide-tabbar",
            "--workdir",
        ])
        .arg(&launch.cwd);
    command
        .arg("-p")
        .arg(format!("TerminalColumns={}", launch.cols));
    command
        .arg("-p")
        .arg(format!("TerminalRows={}", launch.rows));
    command.arg("-e").args(&launch.command);
    command
}

/// The string in a `gdbus call` result such as `('text\n',)`, in GVariant's text format.
///
/// # Errors
///
/// When the result holds no string.
pub fn gvariant_string(result: &str) -> Result<String, Error> {
    let bad = || Error::Parse(format!("gdbus gave {result:?}"));
    let body = result.trim().strip_prefix('(').ok_or_else(bad)?;
    let quote = body
        .chars()
        .next()
        .filter(|&c| c == '\'' || c == '"')
        .ok_or_else(bad)?;
    let mut out = String::new();
    let mut chars = body[1..].chars();
    while let Some(c) = chars.next() {
        match c {
            c if c == quote => return Ok(out),
            '\\' => match chars.next().ok_or_else(bad)? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'a' => out.push('\u{7}'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'v' => out.push('\u{b}'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(
                        u32::from_str_radix(&hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or_else(bad)?,
                    );
                }
                'U' => {
                    let hex: String = chars.by_ref().take(8).collect();
                    out.push(
                        u32::from_str_radix(&hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or_else(bad)?,
                    );
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    Err(bad())
}

/// A string as a GVariant text-format argument.
pub fn gvariant_quote(text: &str) -> String {
    let mut out = String::from("'");
    for c in text.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Rebuilds the screen from `getAllDisplayedText`'s text, one line per row.
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
        tuiframe_vt::WidthRules::default(),
    )?;
    Ok(Readback {
        screen,
        reports: Reports {
            wide_text: true,
            ..Reports::default()
        },
    })
}

impl Driver for Konsole {
    fn name(&self) -> &'static str {
        "konsole"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("konsole");
        command.arg("--version");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        x11::require_display()?;
        let config = self.config_home();
        std::fs::create_dir_all(&config)
            .map_err(|error| Error::Io(config.display().to_string(), error))?;
        std::fs::write(config.join("konsolerc"), KONSOLERC)
            .map_err(|error| Error::Io("konsolerc".to_owned(), error))?;
        let child = x11::spawn(start_command(launch, &config), &self.dir)?;
        let pid = child.id();
        self.child = Some(child);
        let ready = {
            let probe = || self.call("getAllDisplayedText", &[]).map(drop);
            let mut tries = 0;
            loop {
                match probe() {
                    Ok(()) => break Ok(()),
                    Err(error) if tries >= 200 => break Err(error),
                    Err(_) => {
                        tries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
        };
        ready?;
        self.window = x11::window_of(pid).ok();
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.call(
            "sendText",
            &[&gvariant_quote(&String::from_utf8_lossy(bytes))],
        )
        .map(drop)
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        let window = self
            .window
            .as_deref()
            .ok_or(Error::Missing("Konsole's window".to_owned()))?;
        x11::press(window, keys)
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running Konsole".to_owned()))?;
        parse(
            &gvariant_string(&self.call("getAllDisplayedText", &[])?)?,
            cols,
            rows,
        )
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        x_screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        self.window = None;
        x11::kill(&mut self.child);
        Ok(())
    }
}

impl Drop for Konsole {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
