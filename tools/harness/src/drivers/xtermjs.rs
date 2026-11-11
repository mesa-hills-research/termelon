//! xterm.js, the terminal of VS Code and most web terminals, through its headless build on a pty.
//!
//! `tools/harness/xtermjs/run.mjs` runs the program with `node-pty`, feeds what it writes to an
//! `@xterm/headless` terminal, and answers requests as JSON lines. It needs `npm ci` in that
//! directory first. xterm.js takes keys as browser events, so the driver sends bytes, and mouse
//! events through xterm.js's own mouse service.

use std::path::{Path, PathBuf};

use termelon_vt::{Key, Mods, Mouse, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, work_dir,
};
use crate::drivers::lines::{Helper, mouse_request};
use crate::json::Json;
use crate::readback::{ansi_screen, set_cursor};

/// The script's directory, in the harness's source.
pub fn script_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("xtermjs")
}

/// xterm.js headless under node.
#[derive(Debug)]
pub struct XtermJs {
    dir: WorkDir,
    helper: Option<Helper>,
    size: Option<(u16, u16)>,
}

impl XtermJs {
    /// A driver for the `node` on the `PATH`.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<XtermJs, Error> {
        Ok(XtermJs {
            dir: work_dir("xtermjs")?,
            helper: None,
            size: None,
        })
    }

    fn helper(&mut self) -> Result<&mut Helper, Error> {
        self.helper
            .as_mut()
            .ok_or(Error::Missing("a running xterm.js".to_owned()))
    }
}

/// Rebuilds the screen from the script's answer to `{"read":true}`: rows with SGR and OSC 8,
/// the cursor, and whether it shows.
///
/// # Errors
///
/// When the answer lacks a field or its rows hold a sequence `termelon-vt` doesn't know.
pub fn parse(reply: &Json, cols: u16, rows: u16) -> Result<Readback, Error> {
    let missing = |field: &str| Error::Parse(format!("xterm.js's answer has no {field}"));
    let ansi = reply
        .get("ansi")
        .and_then(Json::as_str)
        .ok_or_else(|| missing("ansi"))?;
    let (x, y) = cursor(reply).ok_or_else(|| missing("cursor"))?;
    let visible = reply.get("visible").and_then(Json::as_bool).unwrap_or(true);
    let mut screen = ansi_screen(ansi.as_bytes(), cols, rows, WidthRules::default())?;
    set_cursor(&mut screen, x, y, visible, None)?;
    let reports = Reports {
        colors: Colors::AsWritten,
        attributes: Attributes::ALL,
        cursor: true,
        wide_text: true,
        trims_rows: false,
        indexed_as_named: false,
    };
    Ok(Readback::new(screen, reports))
}

/// The `[x, y]` of an answer's `cursor`.
pub fn cursor(reply: &Json) -> Option<(u16, u16)> {
    let cursor = reply.get("cursor")?;
    let coordinate = |i: usize| {
        cursor
            .at(i)
            .and_then(Json::as_i64)
            .and_then(|v| u16::try_from(v).ok())
    };
    Some((coordinate(0)?, coordinate(1)?))
}

/// Bytes as the helpers take them, in hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

impl Driver for XtermJs {
    fn name(&self) -> &'static str {
        "xtermjs"
    }

    fn version(&self) -> Result<String, Error> {
        let path = script_dir().join("node_modules/@xterm/headless/package.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|error| Error::Io(path.display().to_string(), error))?;
        let json = Json::parse(&text).map_err(Error::Parse)?;
        let version = json
            .get("version")
            .and_then(Json::as_str)
            .unwrap_or("unknown");
        Ok(format!("@xterm/headless {version}"))
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        let dir = script_dir();
        if !dir.join("node_modules").is_dir() {
            return Err(Error::Missing(format!(
                "xterm.js's packages (npm ci in {})",
                dir.display()
            )));
        }
        let mut command = clean_command("node");
        command
            .arg(dir.join("run.mjs"))
            .arg(launch.cols.to_string())
            .arg(launch.rows.to_string());
        command.arg(&launch.cwd).args(&launch.command);
        command.envs(launch.env.iter().map(|(key, value)| (key, value)));
        self.helper = Some(Helper::start(
            "xterm.js's script",
            command,
            &self.dir.join("node.log"),
        )?);
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.helper()?
            .request(&Json::object([("send", Json::Str(hex(bytes)))]))
            .map(drop)
    }

    fn send_keys(&mut self, _keys: &[(Key, Mods)]) -> Result<(), Error> {
        Err(Error::Unsupported("take keys without a browser"))
    }

    fn send_mouse(&mut self, mouse: Mouse) -> Result<(), Error> {
        let request = mouse_request(mouse)?;
        self.helper()?.request(&request).map(drop)
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running xterm.js".to_owned()))?;
        let reply = self
            .helper()?
            .request(&Json::object([("read", Json::Bool(true))]))?;
        Ok(parse(&reply, cols, rows)?.with_raw("json", reply.to_text()))
    }

    fn screenshot(&mut self, _path: &Path) -> Result<bool, Error> {
        Ok(false)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        if let Some(helper) = self.helper.take() {
            helper.stop();
        }
        Ok(())
    }
}

impl Drop for XtermJs {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
