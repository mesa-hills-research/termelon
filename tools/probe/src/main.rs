//! Runs inside a real terminal, measures it and draws test scenes, then writes a report.
//!
//! ```text
//! probe --list
//! probe --scene NAME --record FILE [--report FILE] [--size COLSxROWS] [--hold SECONDS]
//! ```
//!
//! It opens the terminal with the framework, which asks what the terminal supports and measures
//! its widths, and checks the size is the one asked for. It draws the scene as a frame, which ends
//! with `probe-done:NAME` on the last row, after saving to `--record` the bytes that show the
//! frame's cells and cursor on a cleared screen, for the harness to compute the screen the
//! terminal should show. Then it holds the screen, recording every event, until its input ends,
//! `q` is pressed, or `--hold` seconds pass (300 by default), and writes its report as JSON.

mod record;
mod report;
mod scenes;

use std::io::{self, Write as _};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tuiframe::{Buffer, Drawn, Event, KeyCode, Options, Rect, Terminal};

use crate::report::Report;

/// How long the probe holds the screen, at most, by default.
const HOLD: Duration = Duration::from_secs(300);

/// Standard output, for `--list`, before the terminal is open.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe lists its scenes before it opens the terminal"
)]
fn stdout() -> io::Stdout {
    io::stdout()
}

/// Standard error, for a bad command line, before the terminal is open.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe reports a bad command line before it opens the terminal"
)]
fn stderr() -> io::Stderr {
    io::stderr()
}

const USAGE: &str = "usage: probe --list | probe --scene NAME --record FILE [--report FILE] [--size COLSxROWS] [--hold SECONDS]";

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
    let mut terminal =
        Terminal::open(Options::default().width_probes(true)).map_err(|error| error.to_string())?;
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
    // The recording is complete before the marker can show.
    let mut buffer =
        Buffer::with_width_profile(Rect::from(size), terminal.capabilities().width_profile);
    scenes::draw(args.scene, &mut buffer);
    let allowed = record::Allowed::new(terminal.capabilities());
    std::fs::write(
        &args.record,
        record::render(&buffer, args.scene.cursor, allowed),
    )
    .map_err(|error| format!("couldn't write {}: {error}", args.record))?;
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
    terminal.close().map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--list"] {
        let names: Vec<&str> = scenes::SCENES.iter().map(|scene| scene.name).collect();
        let _ = writeln!(stdout(), "{}", names.join(" "));
        return ExitCode::SUCCESS;
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
