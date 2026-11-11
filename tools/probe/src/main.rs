//! Runs inside a real terminal, measures it and draws test scenes, then writes a report.
//!
//! This is a stand-in until the probe can use the framework's `Terminal`. It draws a fixed scene,
//! saves the bytes it wrote, and waits until its input ends, so the harness can read the screen.
//!
//! ```text
//! probe --list
//! probe --scene NAME --record FILE [--hold SECONDS]
//! ```

mod scenes;

use std::io::{self, Read as _, Write as _};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long the probe waits for its input to end, at most, by default.
const HOLD: Duration = Duration::from_secs(300);

/// Standard output, which is the terminal.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe draws its scenes on the terminal"
)]
fn stdout() -> io::Stdout {
    io::stdout()
}

/// Standard error, for a bad command line.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe reports a bad command line"
)]
fn stderr() -> io::Stderr {
    io::stderr()
}

const USAGE: &str = "usage: probe --list | probe --scene NAME --record FILE [--hold SECONDS]";

/// Waits until standard input ends or `hold` passes.
fn hold(hold: Duration) {
    let done = Arc::new(AtomicBool::new(false));
    let reader = Arc::clone(&done);
    std::thread::spawn(move || {
        let mut buf = [0; 256];
        while matches!(io::stdin().read(&mut buf), Ok(n) if n > 0) {}
        reader.store(true, Ordering::Relaxed);
    });
    let deadline = Instant::now() + hold;
    while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--list"] {
        let names: Vec<&str> = scenes::SCENES.iter().map(|scene| scene.name).collect();
        let _ = writeln!(stdout(), "{}", names.join(" "));
        return ExitCode::SUCCESS;
    }
    let mut scene = None;
    let mut record = None;
    let mut seconds = HOLD;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next();
        match (flag.as_str(), value) {
            ("--scene", Some(name)) => scene = scenes::find(name),
            ("--record", Some(path)) => record = Some(path.clone()),
            ("--hold", Some(value)) => match value.parse() {
                Ok(n) => seconds = Duration::from_secs(n),
                Err(_) => scene = None,
            },
            _ => scene = None,
        }
    }
    let (Some(scene), Some(record)) = (scene, record) else {
        let _ = writeln!(stderr(), "{USAGE}");
        return ExitCode::from(2);
    };
    let bytes = scenes::bytes(scene);
    // The recording is complete before the marker can show.
    if let Err(error) = std::fs::write(&record, &bytes) {
        let _ = writeln!(stderr(), "probe: couldn't write {record}: {error}");
        return ExitCode::FAILURE;
    }
    let mut out = stdout().lock();
    if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
        return ExitCode::FAILURE;
    }
    drop(out);
    hold(seconds);
    ExitCode::SUCCESS
}
