//! What the conhost and Windows Terminal drivers share. Both run the program through
//! `harness-console`, the helper in `src/bin`, which starts it in the new console and reports its
//! own process id, and both talk to a second `harness-console` attached to that console, which
//! reads the window with `ReadConsoleOutputW` and types with `WriteConsoleInputW`.

use std::path::{Path, PathBuf};
use std::process::Command;

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{Attributes, Colors, Error, Launch, Readback, Reports, describe, run};
use crate::drivers::lines::Helper;
use crate::json::Json;
use crate::readback::{ansi_screen, set_cursor};

/// `CREATE_NEW_CONSOLE`, which gives a process a console of its own.
pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// `DETACHED_PROCESS`, which starts a process with no console.
pub const DETACHED_PROCESS: u32 = 0x0000_0008;

/// `CREATE_NO_WINDOW`, which gives a console program a console with no window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Fails on a system that isn't Windows.
///
/// # Errors
///
/// Everywhere but Windows.
pub fn require_windows() -> Result<(), Error> {
    if cfg!(windows) {
        Ok(())
    } else {
        Err(Error::Missing("Windows".to_owned()))
    }
}

/// The helper, beside the harness's own executable.
///
/// # Errors
///
/// When it isn't there, as when only the harness was built.
pub fn helper_program() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe()
        .map_err(|error| Error::Io("the harness's own path".to_owned(), error))?;
    let helper = exe.with_file_name(format!("harness-console{}", std::env::consts::EXE_SUFFIX));
    if helper.is_file() {
        Ok(helper)
    } else {
        Err(Error::Missing(format!(
            "{} (cargo build -p harness builds it)",
            helper.display()
        )))
    }
}

/// The helper's arguments that run `launch`'s program in the console it starts in, at a size
/// where `size` asks, writing its process id to `pid_file` where given.
pub fn launch_args(
    launch: &Launch,
    size: Option<(u16, u16)>,
    pid_file: Option<&Path>,
) -> Vec<String> {
    let mut args = vec!["launch".to_owned()];
    if let Some((cols, rows)) = size {
        args.extend(["--size".to_owned(), format!("{cols}x{rows}")]);
    }
    if let Some(path) = pid_file {
        args.extend(["--pid-file".to_owned(), path.display().to_string()]);
    }
    args.push("--".to_owned());
    args.extend(launch.command.iter().cloned());
    args
}

/// Leaves out of `command`'s environment what a parent terminal sets, so the program's answers
/// are the console's own.
pub fn clean_environment(command: &mut Command) {
    for key in [
        "WT_SESSION",
        "WT_PROFILE_ID",
        "TERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "COLORTERM",
        "TERMELON_CAPS",
    ] {
        command.env_remove(key);
    }
}

/// Sets Windows process creation flags, where there are such.
#[cfg_attr(
    not(windows),
    expect(unused_variables, reason = "flags exist only on Windows")
)]
pub fn set_flags(command: &mut Command, flags: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(flags);
    }
}

/// Starts a helper attached to the console of process `pid`, its standard error going to `log`.
///
/// # Errors
///
/// When the helper can't start or attach.
pub fn attach(pid: u32, log: &Path) -> Result<Helper, Error> {
    let mut command = Command::new(helper_program()?);
    command.args(["attach", &pid.to_string()]);
    set_flags(&mut command, DETACHED_PROCESS);
    Helper::start("the console helper", command, log)
}

/// Ends process `pid` and the processes it started.
pub fn kill_tree(pid: u32) {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T", "/F"]);
    set_flags(&mut command, CREATE_NO_WINDOW);
    let _ = run(&mut command);
}

/// The text of a console window's cells, a line per row. The second cell of a wide character is
/// left out, as is anything but a low surrogate in it, and control characters become spaces.
pub fn rows_text(cols: u16, cells: &[(u16, u16)]) -> String {
    /// `COMMON_LVB_TRAILING_BYTE`, which marks the second cell of a wide character.
    const TRAILING: u16 = 0x0200;
    let mut text = String::new();
    for row in cells.chunks(usize::from(cols.max(1))) {
        let units: Vec<u16> = row
            .iter()
            .filter(|&&(unit, attributes)| {
                attributes & TRAILING == 0 || (0xdc00..=0xdfff).contains(&unit)
            })
            .map(|&(unit, _)| {
                if unit < 0x20 || unit == 0x7f {
                    0x20
                } else {
                    unit
                }
            })
            .collect();
        text.push_str(&String::from_utf16_lossy(&units));
        text.push('\n');
    }
    text
}

/// Rebuilds the screen from the helper's answer to `{"read":true}`: the window's text, its
/// cursor and whether that shows.
///
/// # Errors
///
/// When the answer lacks a field, or the helper said it failed.
pub fn parse_read(reply: &Json, cols: u16, rows: u16) -> Result<Readback, Error> {
    if let Some(error) = reply.get("error").and_then(Json::as_str) {
        return Err(Error::Parse(format!("the console helper said {error}")));
    }
    let text = reply
        .get("text")
        .and_then(Json::as_str)
        .ok_or_else(|| Error::Parse("the console helper's answer has no text".to_owned()))?;
    let mut screen = ansi_screen(text.as_bytes(), cols, rows, WidthRules::default())?;
    let cursor = reply.get("cursor");
    let at = |i| {
        cursor
            .and_then(|cursor| cursor.at(i))
            .and_then(Json::as_i64)
            .and_then(|v| u16::try_from(v).ok())
    };
    let visible = reply.get("visible").and_then(Json::as_bool).unwrap_or(true);
    if let (Some(x), Some(y)) = (at(0), at(1)) {
        set_cursor(&mut screen, x, y, visible, None)?;
    }
    Ok(Readback {
        screen,
        // (The console API says the cursor shows on the alternate screen whatever the program
        // set, so the cursor isn't compared.)
        reports: Reports {
            colors: Colors::None,
            attributes: Attributes::NONE,
            cursor: false,
            wide_text: true,
            trims_rows: false,
        },
    })
}

/// A character's UTF-16 unit, for one in the BMP.
fn unit_of(c: char) -> u16 {
    u16::try_from(u32::from(c)).unwrap_or(0)
}

/// A key's virtual-key code and the UTF-16 unit it types with no modifier, on a US layout.
fn virtual_key(key: Key) -> Option<(u16, u16)> {
    Some(match key {
        Key::Char(c @ 'a'..='z') => (unit_of(c.to_ascii_uppercase()), unit_of(c)),
        Key::Char(c @ '0'..='9') => (unit_of(c), unit_of(c)),
        Key::Char(' ') => (0x20, 0x20),
        Key::Enter => (0x0d, 0x0d),
        Key::Tab => (0x09, 0x09),
        Key::Backspace => (0x08, 0x08),
        Key::Escape => (0x1b, 0x1b),
        Key::PageUp => (0x21, 0),
        Key::PageDown => (0x22, 0),
        Key::End => (0x23, 0),
        Key::Home => (0x24, 0),
        Key::Left => (0x25, 0),
        Key::Up => (0x26, 0),
        Key::Right => (0x27, 0),
        Key::Down => (0x28, 0),
        Key::Insert => (0x2d, 0),
        Key::Delete => (0x2e, 0),
        Key::F(n @ 1..=24) => (0x70 + u16::from(n) - 1, 0),
        _ => return None,
    })
}

/// The virtual-key codes to hold together for a key with its modifiers, the modifiers first.
pub fn chord(key: Key, mods: Mods) -> Option<Vec<u16>> {
    let (vk, _) = virtual_key(key)?;
    let mut keys = Vec::new();
    for (modifier, held) in [(Mods::CTRL, 0x11), (Mods::ALT, 0x12), (Mods::SHIFT, 0x10)] {
        if mods.contains(modifier) {
            keys.push(held);
        }
    }
    keys.push(vk);
    Some(keys)
}

/// A key's press and release as console key records, `[down, vk, scan, unit, state]` each, as a
/// keyboard gives them in a console window.
pub fn key_records(key: Key, mods: Mods) -> Option<Json> {
    let (vk, mut unit) = virtual_key(key)?;
    let mut state = 0;
    if mods.contains(Mods::SHIFT) {
        state |= 0x0010;
        if let Key::Char(c) = key
            && c.is_ascii_lowercase()
        {
            unit = unit_of(c.to_ascii_uppercase());
        }
    }
    if mods.contains(Mods::CTRL) {
        state |= 0x0008;
        if let Key::Char(c) = key
            && c.is_ascii_lowercase()
        {
            unit = unit_of(c) & 0x1f;
        }
    }
    if mods.contains(Mods::ALT) {
        state |= 0x0002;
    }
    if unit == 0 {
        // (The keys that type nothing are on the enhanced part of the keyboard.)
        state |= 0x0100;
    }
    let record = |down: bool| {
        Json::List(vec![
            Json::Bool(down),
            Json::Int(i64::from(vk)),
            Json::Int(0),
            Json::Int(i64::from(unit)),
            Json::Int(i64::from(state)),
        ])
    };
    Some(Json::List(vec![record(true), record(false)]))
}

/// Saves a screenshot of the whole screen as PNG with .NET's `CopyFromScreen`, through Windows
/// PowerShell.
///
/// # Errors
///
/// When PowerShell fails.
pub fn screenshot(path: &Path) -> Result<bool, Error> {
    const SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
        $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
        $image = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height; \
        $graphics = [System.Drawing.Graphics]::FromImage($image); \
        $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size); \
        $image.Save($env:HARNESS_SCREENSHOT, [System.Drawing.Imaging.ImageFormat]::Png)";
    if !cfg!(windows) {
        return Ok(false);
    }
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("HARNESS_SCREENSHOT", plain_path(path));
    set_flags(&mut command, CREATE_NO_WINDOW);
    run(&mut command)?;
    Ok(true)
}

/// A path without the `\\?\` prefix that `canonicalize` gives on Windows, which .NET Framework
/// doesn't take.
pub fn plain_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        text
    }
}

/// What a program prints, as text, with its first line trimmed.
///
/// # Errors
///
/// When it can't run or fails.
pub fn first_line(command: &mut Command) -> Result<String, Error> {
    let described = describe(command);
    let out = run(command)?;
    let text = String::from_utf8_lossy(&out);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error::Parse(format!("{described} printed nothing")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_one_cell_of_a_wide_character_and_both_halves_of_a_pair() {
        let cells = [
            (u16::from(b'a'), 0x07),
            (0x65e5, 0x0107),
            (0x65e5, 0x0207),
            (0x1b, 0x07),
            (0xd83d, 0x0107),
            (0xde00, 0x0207),
            (u16::from(b'z'), 0x07),
            (0x20, 0x07),
        ];
        assert_eq!(rows_text(4, &cells), "a日 \n😀z \n");
    }

    #[test]
    fn a_read_becomes_a_screen_with_its_cursor() {
        let reply = Json::parse(r#"{"text":"ab\ncd\n","cursor":[1,1],"visible":false}"#).unwrap();
        let readback = parse_read(&reply, 4, 2).unwrap();
        assert_eq!(readback.rows(), ["ab", "cd"]);
        let cursor = readback.screen.cursor();
        assert_eq!((cursor.x, cursor.y, cursor.visible), (1, 1, false));
        let failed = Json::parse(r#"{"error":"no console"}"#).unwrap();
        assert!(parse_read(&failed, 4, 2).is_err());
    }

    #[test]
    fn keys_become_records_and_chords() {
        let enter = key_records(Key::Enter, Mods::SHIFT).unwrap();
        assert_eq!(
            enter.to_text().replace(['\n', ' '], ""),
            "[[true,13,0,13,16],[false,13,0,13,16]]"
        );
        let ctrl_c = key_records(Key::Char('c'), Mods::CTRL).unwrap();
        assert_eq!(
            ctrl_c.at(0).and_then(|r| r.at(3)).and_then(Json::as_i64),
            Some(3)
        );
        let up = key_records(Key::Up, Mods::NONE).unwrap();
        assert_eq!(
            up.at(0).and_then(|r| r.at(4)).and_then(Json::as_i64),
            Some(0x100)
        );
        assert_eq!(
            chord(Key::F(9), Mods::CTRL | Mods::SHIFT),
            Some(vec![0x11, 0x10, 0x78])
        );
        assert_eq!(chord(Key::Char('é'), Mods::NONE), None);
    }

    #[test]
    fn paths_lose_the_verbatim_prefix() {
        assert_eq!(plain_path(Path::new(r"\\?\D:\out\a.png")), r"D:\out\a.png");
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\host\share\a.png")),
            r"\\host\share\a.png"
        );
        assert_eq!(plain_path(Path::new("out/a.png")), "out/a.png");
    }

    #[test]
    fn the_launcher_gets_the_program_after_its_own_options() {
        let launch = Launch::new(80, 24, &["probe", "--scene", "text"]);
        assert_eq!(
            launch_args(&launch, Some((80, 24)), Some(Path::new("pid"))),
            [
                "launch",
                "--size",
                "80x24",
                "--pid-file",
                "pid",
                "--",
                "probe",
                "--scene",
                "text"
            ]
        );
    }
}
