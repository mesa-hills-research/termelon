//! Runs inside a real terminal, measures it and draws test scenes, then writes a report.
//!
//! ```text
//! probe --list
//! probe --scene NAME --record FILE [--report FILE] [--size COLSxROWS] [--hold SECONDS]
//! probe --keys MODE --input FILE [--record FILE] [--hold SECONDS]
//! ```
//!
//! It opens the terminal with the framework, which asks what the terminal supports and measures
//! its widths, and checks the size is the one asked for. It draws the scene as a frame, which ends
//! with `probe-done:NAME` on the last row. Then it holds the screen, recording every event, until
//! its input ends, `q` is pressed, or `--hold` seconds pass (300 by default). It saves to
//! `--record` every byte the framework wrote, for the harness to compute the screen the terminal
//! should show, and writes its report as JSON.
//!
//! With `--keys`, it saves instead the raw bytes the terminal sends to `--input`, in a mode of
//! `legacy`, `disambiguate` (the kitty keyboard protocol's flag 1) or `modify-other-keys`
//! (xterm's level 2), each with SGR mouse reports on, and the bytes that set the mode to
//! `--record`.

mod input;
mod report;
mod scenes;

use std::io::{self, Write as _};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use termelon::{Drawn, Event, KeyCode, Options, Terminal};

use crate::report::Report;

/// How long the probe holds the screen, at most, by default.
const HOLD: Duration = Duration::from_secs(300);

/// Standard output, for `--list` and `--keys`, which don't open the terminal with the framework.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe lists its scenes and sets up `--keys` without the framework's terminal"
)]
fn stdout() -> io::Stdout {
    io::stdout()
}

/// Standard error, for a bad command line and the errors of `--keys`, without the framework's
/// terminal.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe reports a bad command line, and `--keys` its errors, without the framework's terminal"
)]
fn stderr() -> io::Stderr {
    io::stderr()
}

const USAGE: &str = "usage: probe --list | probe --scene NAME --record FILE [--report FILE] [--size COLSxROWS] \
                     [--hold SECONDS] | probe --keys MODE --input FILE [--record FILE] [--hold SECONDS]";

struct Args {
    scene: &'static scenes::Scene,
    record: String,
    report: Option<String>,
    size: Option<(u16, u16)>,
    hold: Duration,
}

fn parse(args: &[String]) -> Option<Args> {
    let (mut scene, mut record, mut report, mut size, mut hold) = (None, None, None, None, HOLD);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next()?;
        match flag.as_str() {
            "--scene" => scene = Some(scenes::find(value)?),
            "--record" => record = Some(value.clone()),
            "--report" => report = Some(value.clone()),
            "--size" => {
                let (cols, rows) = value.split_once('x')?;
                size = Some((cols.parse().ok()?, rows.parse().ok()?));
            }
            "--hold" => hold = Duration::from_secs(value.parse().ok()?),
            _ => return None,
        }
    }
    Some(Args {
        scene: scene?,
        record: record?,
        report,
        size,
        hold,
    })
}

/// Draws the scene as a frame, drawing again while the terminal holds the last one.
fn draw(terminal: &mut Terminal, scene: &scenes::Scene) -> Result<(), String> {
    for _ in 0..500 {
        let drawn = terminal
            .draw(|frame| {
                scenes::draw(scene, frame.buffer_mut());
                frame.set_cursor(scene.cursor);
            })
            .map_err(|error| error.to_string())?;
        if matches!(drawn, Drawn::Written(_)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("the terminal never took the frame".to_owned())
}

/// Runs the scene, filling in the report.
fn run(args: &Args, report: &mut Report) -> Result<(), String> {
    let options = Options::default().width_probes(true).record_output(true);
    let mut terminal = Terminal::open(options).map_err(|error| error.to_string())?;
    let size = terminal.size();
    report.size = Some(size);
    report.capabilities = Some(terminal.capabilities().clone());
    if let Some((cols, rows)) = args.size
        && (size.width, size.height) != (cols, rows)
    {
        let _ = terminal.close();
        return Err(format!(
            "the terminal is {}x{}, not {cols}x{rows}",
            size.width, size.height
        ));
    }
    draw(&mut terminal, args.scene)?;
    let mut events = terminal.events();
    let deadline = Instant::now() + args.hold;
    while Instant::now() < deadline {
        match events.next_timeout(Duration::from_millis(100)) {
            Ok(Some(event)) => {
                let quit = matches!(&event, Event::Key(key) if key.code == KeyCode::Char('q'));
                report.events.push(event);
                if quit {
                    break;
                }
            }
            Ok(None) => {}
            Err(_) => break,
        }
    }
    block_on(terminal.written(), Duration::from_secs(2));
    std::fs::write(&args.record, terminal.take_written())
        .map_err(|error| format!("couldn't write {}: {error}", args.record))?;
    terminal.close().map_err(|error| error.to_string())
}

/// Runs `future` on this thread until it completes or `timeout` passes.
fn block_on(future: impl Future<Output = ()>, timeout: Duration) {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let until = Instant::now() + timeout;
    let waker = std::task::Waker::from(std::sync::Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    while future.as_mut().poll(&mut cx).is_pending() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::park_timeout(left);
    }
}

/// The options of `--keys MODE`.
fn parse_keys(mode: &str, args: &[String]) -> Option<input::Options> {
    let mut options = input::Options {
        mode: mode.to_owned(),
        input: String::new(),
        record: None,
        hold: HOLD,
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next()?;
        match flag.as_str() {
            "--input" => options.input.clone_from(value),
            "--record" => options.record = Some(value.clone()),
            "--hold" => options.hold = Duration::from_secs(value.parse().ok()?),
            _ => return None,
        }
    }
    (!options.input.is_empty()).then_some(options)
}

/// `--keys MODE` with the options after it.
fn keys(mode: &str, args: &[String]) -> ExitCode {
    let Some(options) = parse_keys(mode, args) else {
        let _ = writeln!(stderr(), "{USAGE}");
        return ExitCode::from(2);
    };
    match input::record(&options, &mut stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr(), "probe: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--list"] {
        let names: Vec<&str> = scenes::SCENES.iter().map(|scene| scene.name).collect();
        let _ = writeln!(stdout(), "{}", names.join(" "));
        return ExitCode::SUCCESS;
    }
    if let [flag, mode, rest @ ..] = args.as_slice()
        && flag == "--keys"
    {
        return keys(mode, rest);
    }
    let Some(args) = parse(&args) else {
        let _ = writeln!(stderr(), "{USAGE}");
        return ExitCode::from(2);
    };
    let mut report = Report {
        scene: args.scene.name.to_owned(),
        asked_size: args.size,
        ..Report::default()
    };
    let result = run(&args, &mut report);
    if let Err(error) = &result {
        report.error = Some(error.clone());
    }
    if let Some(path) = &args.report {
        let _ = std::fs::write(path, report.to_json());
    }
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
