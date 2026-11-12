//! The Linux console, on a virtual console of its own, read back from `/dev/vcsu` and `/dev/vcsa`.
//!
//! The program runs on the console with `openvt`, as the user who runs the harness, and the
//! console's text comes from `/dev/vcsuN`, a code point per cell, with the cursor from the header
//! of `/dev/vcsaN`. Opening a console and reading those devices takes root, through `sudo -n`.
//! The readback is text only, and nothing presses keys, since the kernel no longer lets a program
//! type into a terminal (`dev.tty.legacy_tiocsti` is off).

use std::path::{Path, PathBuf};
use std::time::Duration;

use termelon_vt::{Key, Mods, WidthRules};

use crate::driver::{
    Driver, Error, Launch, Readback, Reports, WorkDir, clean_command, run, work_dir,
};
use crate::readback::{ansi_screen, set_cursor};

/// The console the driver takes, unless `HARNESS_VT` names another.
const DEFAULT_VT: u8 = 12;

/// How long the program may take to start on the console.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// The Linux console.
#[derive(Debug)]
pub struct LinuxConsole {
    vt: u8,
    dir: WorkDir,
    running: bool,
}

impl LinuxConsole {
    /// A driver for console 12, or the one `HARNESS_VT` names.
    ///
    /// # Errors
    ///
    /// When its work directory can't be made.
    pub fn new() -> Result<LinuxConsole, Error> {
        let vt = std::env::var("HARNESS_VT")
            .ok()
            .and_then(|vt| vt.parse().ok())
            .unwrap_or(DEFAULT_VT);
        Ok(LinuxConsole {
            vt,
            dir: work_dir("linux")?,
            running: false,
        })
    }

    fn pid_file(&self) -> PathBuf {
        self.dir.join("pid")
    }

    fn tty(&self) -> String {
        format!("/dev/tty{}", self.vt)
    }

    /// Runs a command as root.
    fn sudo(args: &[&str]) -> Result<Vec<u8>, Error> {
        let mut command = clean_command("sudo");
        command.arg("-n").args(args);
        run(&mut command)
    }
}

/// The user and group ids of this process, as `id` gives them.
fn ids() -> Result<(String, String), Error> {
    let id = |flag: &str| -> Result<String, Error> {
        let mut command = clean_command("id");
        command.arg(flag);
        Ok(String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned())
    };
    Ok((id("-u")?, id("-g")?))
}

/// The command that runs `launch`'s program on console `vt` as the user `uid`, writing its process
/// id to `pid_file` first.
pub fn start_command(
    launch: &Launch,
    vt: u8,
    uid: &str,
    gid: &str,
    pid_file: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "openvt",
        "-f",
        "-c",
        &vt.to_string(),
        "--",
        "setpriv",
        &format!("--reuid={uid}"),
        &format!("--regid={gid}"),
        "--init-groups",
        "--",
        "env",
        "-i",
    ]
    .iter()
    .map(|&arg| arg.to_owned())
    .collect();
    for key in ["HOME", "PATH"] {
        if let Some(value) = std::env::var_os(key) {
            args.push(format!("{key}={}", value.to_string_lossy()));
        }
    }
    args.extend(["LANG=C.UTF-8".to_owned(), "TERM=linux".to_owned()]);
    args.extend(
        launch
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}")),
    );
    args.extend([
        "sh".to_owned(),
        "-c".to_owned(),
        "echo $$ > \"$1\" && cd \"$2\" && shift 2 && exec \"$@\"".to_owned(),
        "sh".to_owned(),
        pid_file.display().to_string(),
        launch.cwd.display().to_string(),
    ]);
    args.extend(launch.command.iter().cloned());
    args
}

/// Rebuilds the console from `/dev/vcsu`, a little-endian code point per cell, and the header of
/// `/dev/vcsa`: its rows, its columns, and the cursor's column and row.
///
/// # Errors
///
/// When the header is short or the text doesn't fill the screen it gives.
pub fn parse(vcsu: &[u8], vcsa_header: &[u8]) -> Result<Readback, Error> {
    let [rows, cols, x, y, ..] = *vcsa_header else {
        return Err(Error::Parse("/dev/vcsa has no header".to_owned()));
    };
    let cells = usize::from(rows) * usize::from(cols);
    if vcsu.len() < cells * 4 {
        return Err(Error::Parse(format!(
            "/dev/vcsu has {} bytes for {cols} by {rows} cells",
            vcsu.len()
        )));
    }
    let mut text = String::with_capacity(cells + usize::from(rows));
    for (i, cell) in vcsu.as_chunks::<4>().0.iter().take(cells).enumerate() {
        if i > 0 && i % usize::from(cols) == 0 {
            text.push('\n');
        }
        let code = u32::from_le_bytes(*cell);
        // The second cell of a wide character holds U+200B, or no character at all.
        match char::from_u32(code) {
            Some('\u{200b}') | None => {}
            Some(c) if !c.is_control() => text.push(c),
            Some(_) => text.push(' '),
        }
    }
    let mut screen = ansi_screen(
        text.as_bytes(),
        u16::from(cols),
        u16::from(rows),
        WidthRules::default(),
    )?;
    set_cursor(&mut screen, u16::from(x), u16::from(y), true, None)?;
    Ok(Readback::new(
        screen,
        Reports {
            wide_text: true,
            ..Reports::default()
        },
    ))
}

impl Driver for LinuxConsole {
    fn name(&self) -> &'static str {
        "linux"
    }

    /// The kernel's version to its minor number, such as `Linux 7.0`.
    fn version(&self) -> Result<String, Error> {
        let mut command = clean_command("uname");
        command.arg("-r");
        let release = String::from_utf8_lossy(&run(&mut command)?)
            .trim()
            .to_owned();
        let numbers: Vec<&str> = release.split(['.', '-']).take(2).collect();
        Ok(format!("Linux {}", numbers.join(".")))
    }

    fn start(&mut self, launch: &Launch) -> Result<(), Error> {
        self.stop()?;
        let tty = self.tty();
        // A full reset clears what an earlier program left, and the console takes the size
        // asked where its driver lets it.
        Self::sudo(&["sh", "-c", "printf '\\033c' > \"$1\"", "sh", &tty])?;
        let (cols, rows) = (launch.cols.to_string(), launch.rows.to_string());
        let _ = Self::sudo(&["stty", "-F", &tty, "cols", &cols, "rows", &rows]);
        let pid_file = self.pid_file();
        let _ = std::fs::remove_file(&pid_file);
        let (uid, gid) = ids()?;
        let args = start_command(launch, self.vt, &uid, &gid, &pid_file);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        Self::sudo(&args)?;
        self.running = true;
        let deadline = std::time::Instant::now() + START_TIMEOUT;
        while !pid_file.exists() {
            if std::time::Instant::now() >= deadline {
                return Err(Error::Missing(format!(
                    "the program on {tty}, which writes {} as it starts,",
                    pid_file.display()
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    fn send_bytes(&mut self, _bytes: &[u8]) -> Result<(), Error> {
        Err(Error::Unsupported("take input from another program"))
    }

    fn send_keys(&mut self, _keys: &[(Key, Mods)]) -> Result<(), Error> {
        Err(Error::Unsupported("take synthetic keys"))
    }

    fn read(&mut self) -> Result<Readback, Error> {
        if !self.running {
            return Err(Error::Missing("a program on the console".to_owned()));
        }
        let vcsu = Self::sudo(&["cat", &format!("/dev/vcsu{}", self.vt)])?;
        let vcsa = Self::sudo(&["head", "-c", "4", &format!("/dev/vcsa{}", self.vt)])?;
        Ok(parse(&vcsu, &vcsa)?
            .with_raw("vcsu", vcsu)
            .with_raw("vcsa", vcsa))
    }

    fn screenshot(&mut self, _path: &Path) -> Result<bool, Error> {
        Ok(false)
    }

    fn stop(&mut self) -> Result<(), Error> {
        if std::mem::take(&mut self.running)
            && let Ok(pid) = std::fs::read_to_string(self.pid_file())
        {
            let _ = Self::sudo(&["kill", pid.trim()]);
        }
        Ok(())
    }
}

impl Drop for LinuxConsole {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_console_reads_back() {
        // Two rows of four cells, where the second cell of 字 holds U+200B, as the kernel writes
        // it, and of 幕 0xFFFFFFFF, no character.
        let mut cells: Vec<u8> = "ab  é字\u{200b}x幕"
            .chars()
            .flat_map(|c| u32::from(c).to_le_bytes())
            .collect();
        cells.extend_from_slice(&u32::MAX.to_le_bytes());
        cells.extend_from_slice(&u32::from('y').to_le_bytes());
        cells.extend_from_slice(&u32::from(' ').to_le_bytes());
        let readback = parse(&cells, &[3, 4, 1, 1]).unwrap();
        assert_eq!(readback.rows(), ["ab", "é字x", "幕y"]);
        let cursor = readback.screen.cursor();
        assert_eq!((cursor.x, cursor.y), (1, 1));
        assert!(parse(&cells[..8], &[2, 4, 0, 0]).is_err());
        assert!(parse(&cells, &[2]).is_err());
    }
}
