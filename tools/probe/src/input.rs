//! `--keys`: the raw bytes the terminal sends for the keys and mouse events the harness presses.
//!
//! The probe puts the terminal in raw mode with `stty`, turns on the mode asked for with the
//! terminal's own sequences, shows `probe-keys:ready`, and appends every byte it reads to the
//! input file as it comes, until its input ends or `--hold` passes. The framework isn't involved,
//! since its terminal decodes the bytes this saves. The bytes that set the mode go to the record
//! file, so the harness can play them to `termelon-vt` and ask it what the terminal should send.

use std::io::{self, Read as _, Write as _};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The marker the harness waits for before pressing anything.
pub(crate) const READY: &str = "probe-keys:ready";

/// How keys are asked to be reported, with the sequences that ask and undo it. Every mode also
/// asks for SGR mouse reports of presses, releases and the wheel.
pub(crate) const MODES: [(&str, &str, &str); 3] = [
    ("legacy", "", ""),
    ("disambiguate", "\x1b[>1u", "\x1b[<u"),
    ("modify-other-keys", "\x1b[>4;2m", "\x1b[>4m"),
];

const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1000l";

/// What `--keys` was asked to do.
#[derive(Debug)]
pub(crate) struct Options {
    /// The mode, a name from [`MODES`].
    pub(crate) mode: String,
    /// Where the bytes the terminal sends go.
    pub(crate) input: String,
    /// Where the bytes that set the mode go.
    pub(crate) record: Option<String>,
    /// How long to wait for input, at most.
    pub(crate) hold: Duration,
}

/// The bytes that clear the screen and set a mode, and the bytes that undo it.
pub(crate) fn setup(mode: &str) -> Option<(String, String)> {
    let (_, on, off) = MODES.iter().find(|(name, _, _)| *name == mode)?;
    Some((
        format!("\x1b[H\x1b[2J{MOUSE_ON}{on}"),
        format!("{off}{MOUSE_OFF}"),
    ))
}

fn stty(args: &[&str]) -> Result<(), String> {
    let status = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .status()
        .map_err(|error| format!("couldn't run stty: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("stty {} failed", args.join(" ")))
    }
}

/// Saves the input until it ends or the hold passes.
pub(crate) fn record(options: &Options, out: &mut impl io::Write) -> Result<(), String> {
    let (on, off) = setup(&options.mode).ok_or_else(|| {
        let names: Vec<&str> = MODES.iter().map(|(name, _, _)| *name).collect();
        format!("no mode {}, only {}", options.mode, names.join(", "))
    })?;
    let path = &options.input;
    let mut file =
        std::fs::File::create(path).map_err(|error| format!("couldn't write {path}: {error}"))?;
    if let Some(record) = &options.record {
        std::fs::write(record, &on).map_err(|error| format!("couldn't write {record}: {error}"))?;
    }
    stty(&["raw", "-echo"])?;
    let _ = write!(out, "{on}{READY}");
    let _ = out.flush();
    // The reader has a thread of its own, so the hold can end the wait.
    let (sender, chunks) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0; 256];
        while let Ok(n @ 1..) = io::stdin().read(&mut buf) {
            if sender.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + options.hold;
    let mut result = Ok(());
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match chunks.recv_timeout(left.min(Duration::from_millis(100))) {
            Ok(chunk) => {
                if let Err(error) = file.write_all(&chunk).and_then(|()| file.flush()) {
                    result = Err(format!("couldn't write {path}: {error}"));
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = write!(out, "{off}");
    let _ = out.flush();
    let _ = stty(&["sane"]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_set_and_undo() {
        let (on, off) = setup("disambiguate").unwrap();
        assert_eq!(on, "\x1b[H\x1b[2J\x1b[?1000h\x1b[?1006h\x1b[>1u");
        assert_eq!(off, "\x1b[<u\x1b[?1006l\x1b[?1000l");
        assert!(setup("legacy").is_some());
        assert!(setup("modify-other-keys").is_some());
        assert_eq!(setup("other"), None);
    }
}
