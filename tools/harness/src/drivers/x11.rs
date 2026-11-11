//! What the drivers of windowed terminals share: the display, spawning and waiting, and for X,
//! finding the window and pressing keys through xdotool.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tuiframe_vt::{Key, Mods};

use crate::driver::{Error, clean_command, describe, run};
use crate::keys;

/// How long a terminal may take to open its window.
pub const START_TIMEOUT: Duration = Duration::from_secs(20);

/// Fails unless there's a display, on systems where terminals need one.
///
/// # Errors
///
/// When neither `DISPLAY` nor `WAYLAND_DISPLAY` is set on Linux and the BSDs.
pub fn require_display() -> Result<(), Error> {
    let display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|key| std::env::var_os(key).is_some());
    if cfg!(any(target_os = "macos", windows)) || display {
        Ok(())
    } else {
        Err(Error::Missing(
            "a display (DISPLAY, such as Xvfb's, or WAYLAND_DISPLAY)".to_owned(),
        ))
    }
}

/// Starts a terminal in the background, its output going to a log in `dir`.
///
/// # Errors
///
/// When it can't start.
pub fn spawn(mut command: Command, dir: &Path) -> Result<Child, Error> {
    let described = describe(&command);
    let log = std::fs::File::create(dir.join("terminal.log"))
        .map_err(|error| Error::Io(described.clone(), error))?;
    let log2 = log
        .try_clone()
        .map_err(|error| Error::Io(described.clone(), error))?;
    command.stdin(Stdio::null()).stdout(log).stderr(log2);
    command.spawn().map_err(|error| Error::Io(described, error))
}

/// Polls `ready` every 100 ms until it succeeds or `START_TIMEOUT` passes.
///
/// # Errors
///
/// The last error, when time runs out or the terminal ends.
pub fn wait_until<T>(
    child: &mut Child,
    mut ready: impl FnMut() -> Result<T, Error>,
) -> Result<T, Error> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let error = match ready() {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        if let Ok(Some(status)) = child.try_wait() {
            return Err(Error::Failed {
                command: "the terminal".to_owned(),
                status: status.to_string(),
                stderr: String::new(),
            });
        }
        if Instant::now() >= deadline {
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The X window of a process, from xdotool.
///
/// # Errors
///
/// When xdotool finds none.
pub fn window_of(pid: u32) -> Result<String, Error> {
    let mut command = clean_command("xdotool");
    command.args(["search", "--onlyvisible", "--pid", &pid.to_string()]);
    let out = run(&mut command)?;
    String::from_utf8_lossy(&out)
        .lines()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| Error::Missing(format!("a window of process {pid}")))
}

/// Presses keys in a window through XTEST, after focusing it.
///
/// # Errors
///
/// When a key has no X name or xdotool fails.
pub fn press(window: &str, keys: &[(Key, Mods)]) -> Result<(), Error> {
    let names = keys
        .iter()
        .map(|&(key, mods)| keys::xdotool(key, mods).ok_or(Error::Unsupported("send that key")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut focus = clean_command("xdotool");
    focus.args(["windowfocus", "--sync", window]);
    run(&mut focus)?;
    let mut command = clean_command("xdotool");
    command.args(["key", "--clearmodifiers"]).args(names);
    run(&mut command).map(drop)
}

/// Ends a terminal that was spawned.
pub fn kill(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
