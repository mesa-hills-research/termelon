//! conhost, the console window of Windows, driven and read back through the console API.
//!
//! The program runs under `harness-console launch` in a new console (`CREATE_NEW_CONSOLE`),
//! which sets the console's buffer and window to the size asked for first. A second
//! `harness-console`, attached to that console with `AttachConsole`, reads its window with
//! `ReadConsoleOutputW` and types with `WriteConsoleInputW`. Where Windows Terminal is the default
//! terminal, a new console opens in it rather than in conhost, unless the console's delegation
//! (`HKCU\Console\%%Startup`) names conhost.

use std::path::Path;
use std::process::{Child, Command, Stdio};

use termelon_vt::{Key, Mods};

use crate::driver::describe;
use crate::driver::{Driver, Error, Launch, Readback, WorkDir, work_dir};
use crate::drivers::console::{
    self, CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, attach, clean_environment, helper_program,
    kill_tree, launch_args, parse_read, require_windows, set_flags,
};
use crate::drivers::lines::Helper;
use crate::drivers::xtermjs::hex;
use crate::json::Json;

/// conhost, a console window per program.
#[derive(Debug)]
pub struct Conhost {
    dir: WorkDir,
    launcher: Option<Child>,
    helper: Option<Helper>,
    size: Option<(u16, u16)>,
}

impl Conhost {
    /// A driver for the conhost of this system.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Conhost, Error> {
        Ok(Conhost {
            dir: work_dir("conhost")?,
            launcher: None,
            helper: None,
            size: None,
        })
    }

    fn helper(&mut self) -> Result<&mut Helper, Error> {
        self.helper
            .as_mut()
            .ok_or(Error::Missing("a running conhost".to_owned()))
    }
}

/// The version `cmd /c ver` prints, `Microsoft Windows [Version 10.0.20348.4297]`, as
/// `10.0.20348.4297`, which is conhost's own.
pub fn parse_version(text: &str) -> Option<String> {
    let start = text.find("Version ")? + "Version ".len();
    let rest = &text[start..];
    let end = rest.find(']').unwrap_or(rest.len());
    Some(rest[..end].trim().to_owned())
}

impl Driver for Conhost {
    fn name(&self) -> &'static str {
        "conhost"
    }

    fn version(&self) -> Result<String, Error> {
        require_windows()?;
        let mut command = Command::new("cmd");
        command.args(["/d", "/c", "ver"]);
        set_flags(&mut command, CREATE_NO_WINDOW);
        let line = console::first_line(&mut command)?;
        parse_version(&line).ok_or_else(|| Error::Parse(format!("cmd /c ver printed {line:?}")))
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        require_windows()?;
        let mut command = Command::new(helper_program()?);
        command
            .args(launch_args(launch, Some((launch.cols, launch.rows)), None))
            .current_dir(&launch.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        clean_environment(&mut command);
        command.envs(launch.env.iter().map(|(key, value)| (key, value)));
        set_flags(&mut command, CREATE_NEW_CONSOLE);
        let described = describe(&command);
        let launcher = command
            .spawn()
            .map_err(|error| Error::Io(described, error))?;
        let pid = launcher.id();
        self.launcher = Some(launcher);
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
        let mut records = Vec::new();
        for &(key, mods) in keys {
            let pressed =
                console::key_records(key, mods).ok_or(Error::Unsupported("send that key"))?;
            records.extend(pressed.items().iter().cloned());
        }
        let reply = self
            .helper()?
            .request(&Json::object([("keys", Json::List(records))]))?;
        match reply.get("error").and_then(Json::as_str) {
            Some(error) => Err(Error::Parse(format!("the console helper said {error}"))),
            None => Ok(()),
        }
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running conhost".to_owned()))?;
        let reply = self
            .helper()?
            .request(&Json::object([("read", Json::Bool(true))]))?;
        parse_read(&reply, cols, rows)
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        console::screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        if let Some(helper) = self.helper.take() {
            helper.stop();
        }
        if let Some(mut launcher) = self.launcher.take() {
            kill_tree(launcher.id());
            let _ = launcher.kill();
            let _ = launcher.wait();
        }
        Ok(())
    }
}

impl Drop for Conhost {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_the_system_s() {
        assert_eq!(
            parse_version("Microsoft Windows [Version 10.0.20348.4297]").as_deref(),
            Some("10.0.20348.4297")
        );
        assert_eq!(parse_version("no version here"), None);
    }
}
