//! Alacritty, under X, read back from the `grid.json` its `--ref-test` mode writes when it closes.
//!
//! Alacritty reports its screen only once, as its window closes, so a read ends the program and the
//! terminal with it, and `wait_for` reads once after the program has had a moment to draw. The
//! program runs under a shell wrapper that writes its process id to a file, so the driver can end it.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Attributes, Colors, Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run,
    work_dir, x_screenshot,
};
use crate::drivers::x11;
use crate::json::Json;
use crate::readback::ansi_screen;

/// How long the program gets to draw before the one read.
const SETTLE: Duration = Duration::from_millis(700);

/// How long Alacritty may take to write its grid once the program ends.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// Alacritty in `--ref-test` mode.
#[derive(Debug)]
pub struct Alacritty {
    dir: WorkDir,
    child: Option<Child>,
    window: Option<String>,
    size: Option<(u16, u16)>,
}

impl Alacritty {
    /// A driver for the `alacritty` on the `PATH`, with xdotool for keys.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<Alacritty, Error> {
        Ok(Alacritty {
            dir: work_dir("alacritty")?,
            child: None,
            window: None,
            size: None,
        })
    }

    fn pid_file(&self) -> PathBuf {
        self.dir.join("pid")
    }
}

/// The configuration: the size, no scrollback (so the grid holds the screen alone), and the
/// default font.
pub fn config(cols: u16, rows: u16) -> String {
    format!(
        "[window]\ndimensions = {{ columns = {cols}, lines = {rows} }}\n\n[scrolling]\nhistory = 0\n"
    )
}

/// The command line that starts Alacritty in its work directory, where `--ref-test` writes, with
/// the program writing its process id to `pid_file` first.
pub fn start_command(launch: &Launch, dir: &Path, pid_file: &Path) -> Command {
    let mut command = clean_command("alacritty");
    command.current_dir(dir).env("LIBGL_ALWAYS_SOFTWARE", "1");
    command
        .arg("--ref-test")
        .arg("--config-file")
        .arg(dir.join("alacritty.toml"));
    for (key, value) in &launch.env {
        command.env(key, value);
    }
    command.args([
        "-e",
        "sh",
        "-c",
        "echo $$ > \"$1\" && cd \"$2\" && shift 2 && exec \"$@\"",
        "sh",
    ]);
    command.arg(pid_file).arg(&launch.cwd).args(&launch.command);
    command
}

/// The SGR for one of Alacritty's colours, `{"Named": "Red"}`, `{"Indexed": 5}` or
/// `{"Spec": {"r": 1, "g": 2, "b": 3}}`. The default colours write nothing.
fn color(json: &Json, extended: u8, base: u8, bright: u8) -> Option<String> {
    if let Some(name) = json.get("Named").and_then(Json::as_str) {
        const NAMES: [&str; 8] = [
            "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White",
        ];
        let (is_bright, plain) = name
            .strip_prefix("Bright")
            .map_or((false, name), |rest| (true, rest));
        // Foreground and Background are the defaults, which write nothing.
        let index = u8::try_from(NAMES.iter().position(|&n| n == plain)?).ok()?;
        if base == 0 {
            // The underline colour has no named form.
            return Some(format!(
                "{extended};5;{}",
                index + if is_bright { 8 } else { 0 }
            ));
        }
        return Some(
            (if is_bright {
                bright + index
            } else {
                base + index
            })
            .to_string(),
        );
    }
    if let Some(index) = json.get("Indexed").and_then(Json::as_i64) {
        return Some(format!("{extended};5;{index}"));
    }
    let spec = json.get("Spec")?;
    let channel = |name: &str| spec.get(name).and_then(Json::as_i64);
    Some(format!(
        "{extended};2;{};{};{}",
        channel("r")?,
        channel("g")?,
        channel("b")?
    ))
}

/// The SGR of one cell of `grid.json`.
fn cell_sgr(cell: &Json) -> String {
    let flags: Vec<&str> = cell
        .get("flags")
        .and_then(Json::as_str)
        .unwrap_or("")
        .split(" | ")
        .collect();
    let has = |name: &str| flags.contains(&name);
    let mut params = vec!["0".to_owned()];
    for (on, code) in [
        (has("BOLD") || has("BOLD_ITALIC") || has("DIM_BOLD"), "1"),
        (has("DIM") || has("DIM_BOLD"), "2"),
        (has("ITALIC") || has("BOLD_ITALIC"), "3"),
        (has("UNDERLINE"), "4"),
        (has("DOUBLE_UNDERLINE"), "4:2"),
        (has("UNDERCURL"), "4:3"),
        (has("DOTTED_UNDERLINE"), "4:4"),
        (has("DASHED_UNDERLINE"), "4:5"),
        (has("INVERSE"), "7"),
        (has("HIDDEN"), "8"),
        (has("STRIKEOUT"), "9"),
    ] {
        if on {
            params.push(code.to_owned());
        }
    }
    params.extend(cell.get("fg").and_then(|fg| color(fg, 38, 30, 90)));
    params.extend(cell.get("bg").and_then(|bg| color(bg, 48, 40, 100)));
    let extra = cell.get("extra");
    params.extend(
        extra
            .and_then(|extra| extra.get("underline_color"))
            .and_then(|ul| color(ul, 58, 0, 0)),
    );
    let uri = extra
        .and_then(|extra| extra.get("hyperlink"))
        .and_then(|link| link.get("inner").or(Some(link)))
        .and_then(|link| link.get("uri"))
        .and_then(Json::as_str)
        .unwrap_or("");
    format!("\x1b]8;;{uri}\x1b\\\x1b[{}m", params.join(";"))
}

/// Rebuilds the screen from `grid.json`. The grid keeps its rows in a ring, bottom row first from
/// `zero`, and a wide character's second cell is a `WIDE_CHAR_SPACER`. Alacritty keeps no blink or
/// overline, and the grid has no cursor.
///
/// # Errors
///
/// When the text isn't JSON or lacks the grid's fields.
pub fn parse(grid_json: &str) -> Result<Readback, Error> {
    let json =
        Json::parse(grid_json).map_err(|error| Error::Parse(format!("grid.json: {error}")))?;
    let field = |name: &str| {
        json.get(name)
            .and_then(Json::as_i64)
            .and_then(|v| u16::try_from(v).ok())
    };
    let missing = |what: &str| Error::Parse(format!("grid.json has no {what}"));
    let cols = field("columns")
        .or_else(|| field("cols"))
        .ok_or_else(|| missing("columns"))?;
    let rows = field("lines").ok_or_else(|| missing("lines"))?;
    let raw = json.get("raw").ok_or_else(|| missing("raw"))?;
    let inner = raw.get("inner").map(Json::items).unwrap_or_default();
    let zero = raw
        .get("zero")
        .and_then(Json::as_i64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0);
    let visible = raw
        .get("visible_lines")
        .and_then(Json::as_i64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(usize::from(rows));
    if inner.is_empty() {
        return Err(missing("rows"));
    }
    let mut ansi = String::new();
    for y in 0..usize::from(rows) {
        if y > 0 {
            ansi.push('\n');
        }
        let index = (zero + visible.saturating_sub(1 + y)) % inner.len();
        let mut last = String::new();
        for cell in inner[index]
            .get("inner")
            .map(Json::items)
            .unwrap_or_default()
        {
            let flags = cell.get("flags").and_then(Json::as_str).unwrap_or("");
            if flags.split(" | ").any(|flag| flag == "WIDE_CHAR_SPACER") {
                continue;
            }
            let style = cell_sgr(cell);
            if style != last {
                ansi.push_str(&style);
                last = style;
            }
            ansi.push_str(cell.get("c").and_then(Json::as_str).unwrap_or(" "));
            let zerowidth = cell
                .get("extra")
                .and_then(|extra| extra.get("zerowidth"))
                .map(Json::items)
                .unwrap_or_default();
            ansi.extend(zerowidth.iter().filter_map(Json::as_str));
        }
        ansi.push_str("\x1b]8;;\x1b\\\x1b[0m");
    }
    let screen = ansi_screen(ansi.as_bytes(), cols, rows, WidthRules::default())?;
    let attributes = Attributes {
        blink: false,
        overline: false,
        ..Attributes::ALL
    };
    let reports = Reports {
        colors: Colors::AsWritten,
        attributes,
        cursor: false,
        wide_text: true,
        trims_rows: false,
        indexed_as_named: false,
    };
    Ok(Readback::new(screen, reports))
}

impl Driver for Alacritty {
    fn name(&self) -> &'static str {
        "alacritty"
    }

    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("alacritty");
        command.arg("--version");
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        x11::require_display()?;
        let path = self.dir.join("alacritty.toml");
        std::fs::write(&path, config(launch.cols, launch.rows))
            .map_err(|error| Error::Io(path.display().to_string(), error))?;
        let _ = std::fs::remove_file(self.dir.join("grid.json"));
        let mut child = x11::spawn(
            start_command(launch, &self.dir, &self.pid_file()),
            &self.dir,
        )?;
        let pid = child.id();
        let pid_file = self.pid_file();
        let ready = x11::wait_until(&mut child, &self.dir, || {
            std::fs::read_to_string(&pid_file)
                .map_err(|error| Error::Io("the pid file".to_owned(), error))
        });
        self.child = Some(child);
        ready?;
        self.window = x11::window_of(pid).ok();
        self.size = Some((launch.cols, launch.rows));
        Ok(())
    }

    fn send_bytes(&mut self, _bytes: &[u8]) -> Result<(), Error> {
        Err(Error::Unsupported("take raw bytes as input"))
    }

    fn send_keys(&mut self, keys: &[(Key, Mods)]) -> Result<(), Error> {
        let window = self
            .window
            .as_deref()
            .ok_or(Error::Missing("Alacritty's window".to_owned()))?;
        x11::press(window, keys)
    }

    /// Ends the program, so Alacritty closes and writes its grid, and reads that.
    fn read(&mut self) -> Result<Readback, Error> {
        self.size
            .ok_or(Error::Missing("a running Alacritty".to_owned()))?;
        let pid = std::fs::read_to_string(self.pid_file())
            .map_err(|error| Error::Io("the pid file".to_owned(), error))?;
        let mut kill = clean_command("kill");
        kill.arg(pid.trim());
        run(&mut kill)?;
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        if let Some(child) = &mut self.child {
            while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        self.stop()?;
        let path = self.dir.join("grid.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|error| Error::Io(path.display().to_string(), error))?;
        Ok(parse(&text)?.with_raw("json", text))
    }

    fn wait_for(&mut self, marker: &str, _timeout: Duration) -> Result<Readback, Error> {
        std::thread::sleep(SETTLE);
        let readback = self.read()?;
        if readback.contains(marker) {
            Ok(readback)
        } else {
            Err(Error::Timeout {
                marker: marker.to_owned(),
                screen: readback.rows(),
            })
        }
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

impl Drop for Alacritty {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
