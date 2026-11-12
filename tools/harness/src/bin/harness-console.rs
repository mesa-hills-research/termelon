//! The harness's helper on Windows, for the consoles of conhost and Windows Terminal.
//!
//! ```text
//! harness-console launch [--size COLSxROWS] [--pid-file PATH] -- PROGRAM [ARGS]...
//! harness-console attach PID
//! ```
//!
//! `launch` runs in a new console, a conhost window or a Windows Terminal tab. It sets the
//! console's buffer and window to the size asked for, writes its own process id to the file
//! asked for, and runs the program with the console as its input and output, ending with its exit
//! code.
//!
//! `attach` attaches to the console of process `PID` and answers requests as JSON lines on its
//! standard input and output, after a first `{"ready":true}`. `{"read":true}` gives the window's
//! text, a line per row, its cursor and whether it shows. `{"send":HEX}` types bytes as
//! characters with no key, `{"keys":[[DOWN,VK,SCAN,UNIT,STATE],...]}` types key records, and
//! `{"quit":true}` ends it.

use std::io::{self, Write as _};
use std::process::ExitCode;

/// Standard error, which problems go to.
#[expect(
    clippy::disallowed_methods,
    reason = "this helper reports problems on standard error"
)]
fn stderr() -> io::Stderr {
    io::stderr()
}

fn complain(text: &str) {
    let _ = writeln!(stderr(), "harness-console: {text}");
}

const USAGE: &str = "usage: harness-console launch [--size COLSxROWS] [--pid-file PATH] -- PROGRAM [ARGS]... \
                     | harness-console attach PID";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("launch") => imp::launch(&args[1..]),
        Some("attach") => imp::attach(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            complain(&error);
            ExitCode::from(2)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::ExitCode;

    pub(crate) fn launch(_args: &[String]) -> Result<ExitCode, String> {
        Err("it runs only on Windows".to_owned())
    }

    pub(crate) fn attach(_args: &[String]) -> Result<ExitCode, String> {
        Err("it runs only on Windows".to_owned())
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::{BufRead as _, Write as _};
    use std::process::{Command, ExitCode};
    use std::time::{Duration, Instant};

    use harness::drivers::console::rows_text;
    use harness::json::Json;
    use harness::windows::{KeyRecord, attach_console, read_window, set_console_size, write_input};

    /// Standard output, which answers go to.
    #[expect(
        clippy::disallowed_methods,
        reason = "this helper answers on standard output"
    )]
    fn stdout() -> std::io::Stdout {
        std::io::stdout()
    }

    fn open(name: &str) -> Result<File, String> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
            .map_err(|error| format!("couldn't open {name}: {error}"))
    }

    pub(crate) fn launch(args: &[String]) -> Result<ExitCode, String> {
        let (mut size, mut pid_file) = (None, None);
        let mut args = args.iter();
        let program = loop {
            match args.next().map(String::as_str) {
                Some("--size") => {
                    let value = args.next().ok_or("--size needs a value")?;
                    let (cols, rows) = value
                        .split_once('x')
                        .and_then(|(c, r)| Some((c.parse::<u16>().ok()?, r.parse::<u16>().ok()?)))
                        .ok_or_else(|| format!("{value} isn't a size like 80x24"))?;
                    size = Some((cols, rows));
                }
                Some("--pid-file") => {
                    pid_file = Some(args.next().ok_or("--pid-file needs a value")?);
                }
                Some("--") => break args.next().ok_or("it needs a program")?,
                Some(other) => return Err(format!("unknown option {other}")),
                None => return Err("it needs a program".to_owned()),
            }
        };
        let (input, output) = (open("CONIN$")?, open("CONOUT$")?);
        if let Some((cols, rows)) = size {
            set_console_size(&output, cols, rows)
                .map_err(|error| format!("couldn't make the console {cols}x{rows}: {error}"))?;
        }
        if let Some(path) = pid_file {
            std::fs::write(path, std::process::id().to_string())
                .map_err(|error| format!("couldn't write {path}: {error}"))?;
        }
        let clone = |file: &File| {
            file.try_clone()
                .map_err(|error| format!("couldn't share the console: {error}"))
        };
        let status = Command::new(program)
            .args(args)
            .stdin(input)
            .stdout(clone(&output)?)
            .stderr(output)
            .status()
            .map_err(|error| format!("couldn't run {program}: {error}"))?;
        let code = status.code().unwrap_or(1);
        Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)))
    }

    /// Bytes from the hex the drivers send.
    fn unhex(text: &str) -> Option<Vec<u8>> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
            .collect()
    }

    /// A key press and release for each UTF-16 unit of `text`, with no key, as a terminal's bytes
    /// arrive in a console.
    fn characters(text: &str) -> Vec<KeyRecord> {
        text.encode_utf16()
            .flat_map(|unit| {
                [true, false].map(|down| KeyRecord {
                    down,
                    virtual_key: 0,
                    scan_code: 0,
                    unicode: unit,
                    control_state: 0,
                })
            })
            .collect()
    }

    /// Key records from `[down, vk, scan, unit, state]` lists.
    fn records(keys: &Json) -> Option<Vec<KeyRecord>> {
        keys.items()
            .iter()
            .map(|key| {
                let number = |i: usize| key.at(i).and_then(Json::as_i64);
                Some(KeyRecord {
                    down: key.at(0).and_then(Json::as_bool)?,
                    virtual_key: u16::try_from(number(1)?).ok()?,
                    scan_code: u16::try_from(number(2)?).ok()?,
                    unicode: u16::try_from(number(3)?).ok()?,
                    control_state: u32::try_from(number(4)?).ok()?,
                })
            })
            .collect()
    }

    fn answer(request: &Json) -> Json {
        let failed = |error: String| Json::object([("error", Json::Str(error))]);
        let ok = || Json::object([("ok", Json::Bool(true))]);
        if request.get("read").is_some() {
            // (Opened again each time, since the program may have switched screen buffers.)
            let cells = match open("CONOUT$").and_then(|output| {
                read_window(&output).map_err(|error| format!("couldn't read the window: {error}"))
            }) {
                Ok(cells) => cells,
                Err(error) => return failed(error),
            };
            return Json::object([
                ("text", Json::Str(rows_text(cells.cols, &cells.cells))),
                (
                    "cursor",
                    Json::List(vec![
                        Json::Int(i64::from(cells.cursor.0)),
                        Json::Int(i64::from(cells.cursor.1)),
                    ]),
                ),
                ("visible", Json::Bool(cells.visible)),
                (
                    "size",
                    Json::List(vec![
                        Json::Int(i64::from(cells.cols)),
                        Json::Int(i64::from(cells.rows)),
                    ]),
                ),
            ]);
        }
        let typed = if let Some(hex) = request.get("send").and_then(Json::as_str) {
            match unhex(hex) {
                Some(bytes) => characters(&String::from_utf8_lossy(&bytes)),
                None => return failed(format!("{hex} isn't hex")),
            }
        } else if let Some(keys) = request.get("keys") {
            match records(keys) {
                Some(keys) => keys,
                None => {
                    return failed(
                        "the keys aren't [down, vk, scan, unit, state] lists".to_owned(),
                    );
                }
            }
        } else {
            return failed(format!("no such request as {}", request.to_text().trim()));
        };
        match open("CONIN$").and_then(|input| {
            write_input(&input, &typed).map_err(|error| format!("couldn't type: {error}"))
        }) {
            Ok(()) => ok(),
            Err(error) => failed(error),
        }
    }

    pub(crate) fn attach(args: &[String]) -> Result<ExitCode, String> {
        let pid: u32 = args
            .first()
            .and_then(|pid| pid.parse().ok())
            .ok_or("it needs a process id")?;
        let mut out = stdout();
        // (The program's console may not have come up yet.)
        let started = Instant::now();
        while let Err(error) = attach_console(pid) {
            if started.elapsed() > Duration::from_secs(10) {
                let reply = Json::object([
                    ("ready", Json::Bool(false)),
                    (
                        "error",
                        Json::Str(format!("couldn't attach to {pid}'s console: {error}")),
                    ),
                ]);
                let _ = writeln!(out, "{}", reply.to_text().replace('\n', ""));
                return Ok(ExitCode::FAILURE);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let say = |out: &mut std::io::Stdout, reply: &Json| {
            let _ = writeln!(out, "{}", reply.to_text().replace('\n', ""));
            let _ = out.flush();
        };
        say(&mut out, &Json::object([("ready", Json::Bool(true))]));
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let request = match Json::parse(&line) {
                Ok(request) => request,
                Err(error) => {
                    say(&mut out, &Json::object([("error", Json::Str(error))]));
                    continue;
                }
            };
            if request.get("quit").is_some() {
                break;
            }
            say(&mut out, &answer(&request));
        }
        Ok(ExitCode::SUCCESS)
    }
}
