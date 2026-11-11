//! VTE, the engine of GNOME Terminal, Tilix and xfce4-terminal, through a small GTK program.
//!
//! `tools/harness/vte/harness-vte.py` puts a `Vte.Terminal` in an offscreen window, runs the
//! program in it, and answers requests as JSON lines. It reads the screen with
//! `get_text_format(HTML)`, whose colours come as RGB with dim and reverse applied, after setting
//! xterm's palette with black on white. It needs Python's GObject bindings and VTE 2.91 (VTE 0.76
//! or later, for `get_text_format`), and a display.

use std::path::{Path, PathBuf};

use termelon_vt::{Key, Mods, Mouse, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, work_dir,
};
use crate::drivers::lines::{Helper, mouse_request};
use crate::drivers::x11;
use crate::drivers::xtermjs::hex;
use crate::json::Json;
use crate::keys;
use crate::readback::{ansi_screen, html_dump};

/// The colours the program sets.
const FOREGROUND: (u8, u8, u8) = (0, 0, 0);
const BACKGROUND: (u8, u8, u8) = (0xff, 0xff, 0xff);

/// The program, in the harness's source.
pub fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vte/harness-vte.py")
}

/// VTE in an offscreen GTK window.
#[derive(Debug)]
pub struct Vte {
    dir: WorkDir,
    helper: Option<Helper>,
    size: Option<(u16, u16)>,
}

impl Vte {
    /// A driver for `python3` on the `PATH`, with its GObject bindings.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Vte, Error> {
        Ok(Vte {
            dir: work_dir("vte")?,
            helper: None,
            size: None,
        })
    }

    fn helper(&mut self) -> Result<&mut Helper, Error> {
        self.helper
            .as_mut()
            .ok_or(Error::Missing("a running VTE".to_owned()))
    }
}

/// Rebuilds the screen from the program's answer to `{"read":true}`. VTE doesn't say whether the
/// cursor shows, its underline colours lose precision, and its HTML ends each row at its last
/// text, so none of those is compared.
///
/// # Errors
///
/// When the answer has no HTML, or the HTML no `<pre>`.
pub fn parse(reply: &Json, cols: u16, rows: u16) -> Result<Readback, Error> {
    let html = reply
        .get("html")
        .and_then(Json::as_str)
        .ok_or_else(|| Error::Parse("VTE's answer has no html".to_owned()))?;
    let dump = html_dump(html)?;
    let screen = ansi_screen(&dump.ansi, cols, rows, WidthRules::default())?;
    let colors = Colors::Rgb {
        foreground: FOREGROUND,
        background: BACKGROUND,
    };
    let attributes = Attributes {
        bold: true,
        italic: true,
        underline: true,
        underline_style: true,
        blink: true,
        strike: true,
        overline: true,
        ..Attributes::NONE
    };
    Ok(Readback {
        screen,
        reports: Reports {
            colors,
            attributes,
            cursor: false,
            wide_text: true,
            trims_rows: true,
        },
    })
}

impl Driver for Vte {
    fn name(&self) -> &'static str {
        "vte"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("python3");
        command.args([
            "-c",
            "import gi; gi.require_version('Vte', '2.91'); from gi.repository import Vte; \
             print(f'VTE {Vte.get_major_version()}.{Vte.get_minor_version()}.{Vte.get_micro_version()}')",
        ]);
        Ok(String::from_utf8_lossy(&crate::driver::run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        x11::require_display()?;
        let mut command = clean_command("python3");
        command
            .arg(script())
            .arg(launch.cols.to_string())
            .arg(launch.rows.to_string());
        command.arg(&launch.cwd).args(&launch.command);
        command.envs(launch.env.iter().map(|(key, value)| (key, value)));
        self.helper = Some(Helper::start(
            "VTE's program",
            command,
            &self.dir.join("vte.log"),
        )?);
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.helper()?
            .request(&Json::object([("send", Json::Str(hex(bytes)))]))
            .map(drop)
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        for &(key, mods) in keys {
            // GTK gives a shifted letter's keysym as the capital, as X does.
            let key = match key {
                Key::Char(c) if mods.contains(Mods::SHIFT) => {
                    Key::Char(c.to_uppercase().next().unwrap_or(c))
                }
                _ => key,
            };
            let name = keys::x_keysym(key).ok_or(Error::Unsupported("send that key"))?;
            let mods = Json::List(
                keys::modifier_names(mods)
                    .into_iter()
                    .map(Json::str)
                    .collect(),
            );
            self.helper()?
                .request(&Json::object([("key", Json::Str(name)), ("mods", mods)]))?;
        }
        Ok(())
    }

    fn send_mouse(&mut self, mouse: Mouse) -> Result<(), Error> {
        let request = mouse_request(mouse)?;
        self.helper()?.request(&request).map(drop)
    }

    fn read(&mut self) -> Result<Readback, Error> {
        let (cols, rows) = self
            .size
            .ok_or(Error::Missing("a running VTE".to_owned()))?;
        let reply = self
            .helper()?
            .request(&Json::object([("read", Json::Bool(true))]))?;
        parse(&reply, cols, rows)
    }

    fn screenshot(&mut self, path: &Path) -> Result<bool, Error> {
        crate::driver::x_screenshot(path)
    }

    fn stop(&mut self) -> Result<(), Error> {
        self.size = None;
        if let Some(helper) = self.helper.take() {
            helper.stop();
        }
        Ok(())
    }
}

impl Drop for Vte {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
