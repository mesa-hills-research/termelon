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
//! with `probe-done:NAME` on the last row, after the frames some scenes draw first. Then it holds the screen, recording every event, until
//! its input ends, `q` is pressed, or `--hold` seconds pass (300 by default). It saves to
//! `--record` every byte the framework wrote, for the harness to compute the screen the terminal
//! should show, and writes its report as JSON, both once the frame is drawn and again at the end.
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

use termelon::{Drawn, Event, Frame, KeyCode, Options, Size, Terminal};

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

/// Draws the scene's frames, the last with its marker, drawing each again while the terminal
/// holds the one before.
fn draw(terminal: &mut Terminal, scene: &scenes::Scene) -> Result<(), String> {
    for before in scene.before {
        draw_frame(terminal, |frame| before(frame.buffer_mut()))?;
    }
    draw_frame(terminal, |frame| {
        scenes::draw(scene, frame.buffer_mut());
        frame.set_cursor(scene.cursor);
    })
}

/// Draws a frame, again while the terminal holds the last one.
fn draw_frame(terminal: &mut Terminal, render: impl Fn(&mut Frame<'_>)) -> Result<(), String> {
    for _ in 0..500 {
        let drawn = terminal.draw(&render).map_err(|error| error.to_string())?;
        if matches!(drawn, Drawn::Written(_)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("the terminal never took the frame".to_owned())
}

/// How long a window may take to reach the size asked, after the program starts in it.
const RESIZE_WAIT: Duration = Duration::from_secs(2);

/// Waits for the terminal to reach `(cols, rows)`, as a window may start at another size and
/// resize a moment later, and returns the size it has.
fn wait_for_size(terminal: &Terminal, (cols, rows): (u16, u16), report: &mut Report) -> Size {
    let mut events = terminal.events();
    let deadline = Instant::now() + RESIZE_WAIT;
    while (terminal.size().width, terminal.size().height) != (cols, rows)
        && Instant::now() < deadline
    {
        match events.next_timeout(Duration::from_millis(50)) {
            Ok(Some(event)) => report.events.push(event),
            Ok(None) => {}
            Err(_) => break,
        }
    }
    terminal.size()
}

/// Writes a file whole or not at all, through a temporary file beside it, so a reader never sees
/// it half written.
fn write_whole(path: &str, contents: &[u8]) -> Result<(), String> {
    let temporary = format!("{path}.part");
    std::fs::write(&temporary, contents)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| format!("couldn't write {path}: {error}"))
}

/// Saves every byte the framework wrote so far, after those `recorded` holds already.
fn record(terminal: &mut Terminal, recorded: &mut Vec<u8>, path: &str) -> Result<(), String> {
    block_on(terminal.written(), Duration::from_secs(2));
    recorded.extend(terminal.take_written());
    write_whole(path, recorded)
}

/// Runs the scene, filling in the report. The record and the report are written once the frame
/// is drawn too, for a terminal that ends the program to read its screen, as Alacritty does.
fn run(args: &Args, report: &mut Report) -> Result<(), String> {
    let options = Options::default().width_probes(true).record_output(true);
    let mut terminal = Terminal::open(options).map_err(|error| error.to_string())?;
    report.capabilities = Some(terminal.capabilities().clone());
    let size = match args.size {
        Some(asked) => wait_for_size(&terminal, asked, report),
        None => terminal.size(),
    };
    report.size = Some(size);
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
    let mut recorded = Vec::new();
    record(&mut terminal, &mut recorded, &args.record)?;
    if let Some(path) = &args.report {
        let _ = write_whole(path, report.to_json().as_bytes());
    }
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
    record(&mut terminal, &mut recorded, &args.record)?;
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
        let _ = write_whole(path, report.to_json().as_bytes());
    }
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
