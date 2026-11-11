//! Starts real terminals with the probe, drives them and reads back what they show.
//!
//! ```text
//! harness list
//! harness version TERMINAL
//! harness smoke TERMINAL
//! harness run TERMINAL --probe PATH [--scene NAME]... [--size COLSxROWS] [--out DIR]
//! ```
//!
//! `run` runs each scene (every scene the probe lists, by default) and writes a report, both
//! screens in the styled-text format and an HTML page per scene into the output directory. It
//! exits with 1 when a scene's screen differs or a step fails, and with 2 on a bad command line.

use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use harness::driver::{clean_command, run};
use harness::drivers::{self, TERMINALS};
use harness::run::{run_scene, smoke, write_outcome};

/// Standard output, which results go to.
#[expect(clippy::disallowed_methods, reason = "this tool prints its results")]
fn stdout() -> io::Stdout {
    io::stdout()
}

/// Standard error, which problems go to.
#[expect(clippy::disallowed_methods, reason = "this tool prints its problems")]
fn stderr() -> io::Stderr {
    io::stderr()
}

fn say(text: &str) {
    let _ = writeln!(stdout(), "{text}");
}

fn complain(text: &str) {
    let _ = writeln!(stderr(), "harness: {text}");
}

const USAGE: &str = "usage: harness list | version TERMINAL | smoke TERMINAL | \
                     run TERMINAL --probe PATH [--scene NAME]... [--size COLSxROWS] [--out DIR]";

struct RunOptions {
    terminal: String,
    probe: PathBuf,
    scenes: Vec<String>,
    size: (u16, u16),
    out: PathBuf,
}

fn parse_size(text: &str) -> Option<(u16, u16)> {
    let (cols, rows) = text.split_once('x')?;
    Some((
        cols.parse().ok().filter(|&c| c > 0)?,
        rows.parse().ok().filter(|&r| r > 0)?,
    ))
}

fn parse_run(args: &[String]) -> Result<RunOptions, String> {
    let mut args = args.iter();
    let terminal = args.next().ok_or("run needs a terminal")?.clone();
    let mut options = RunOptions {
        terminal,
        probe: PathBuf::new(),
        scenes: Vec::new(),
        size: (80, 24),
        out: PathBuf::from("harness-out"),
    };
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--probe" => options.probe = PathBuf::from(value),
            "--scene" => options.scenes.push(value.clone()),
            "--size" => {
                options.size =
                    parse_size(value).ok_or_else(|| format!("{value} isn't a size like 80x24"))?;
            }
            "--out" => options.out = PathBuf::from(value),
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    if options.probe.as_os_str().is_empty() {
        return Err("run needs --probe".to_owned());
    }
    Ok(options)
}

/// The scenes the probe has, from `probe --list`.
fn probe_scenes(probe: &PathBuf) -> Result<Vec<String>, String> {
    let mut command = clean_command(probe);
    command.arg("--list");
    let out = run(&mut command).map_err(|error| error.to_string())?;
    Ok(String::from_utf8_lossy(&out)
        .split_whitespace()
        .map(str::to_owned)
        .collect())
}

fn run_command(options: &RunOptions) -> Result<bool, String> {
    let profile = drivers::profile(&options.terminal)
        .ok_or_else(|| format!("no profile for {}", options.terminal))?;
    let mut driver = drivers::driver(&options.terminal).map_err(|error| error.to_string())?;
    let version = driver
        .version()
        .unwrap_or_else(|error| format!("unknown ({error})"));
    say(&format!("{} {version}", options.terminal));
    std::fs::create_dir_all(&options.out)
        .map_err(|error| format!("couldn't make {}: {error}", options.out.display()))?;
    let out = std::fs::canonicalize(&options.out).unwrap_or_else(|_| options.out.clone());
    let probe = std::fs::canonicalize(&options.probe).unwrap_or_else(|_| options.probe.clone());
    let scenes = if options.scenes.is_empty() {
        probe_scenes(&probe)?
    } else {
        options.scenes.clone()
    };
    let mut all_match = true;
    for scene in &scenes {
        match run_scene(
            driver.as_mut(),
            &probe,
            scene,
            options.size,
            profile.clone(),
            &out,
        ) {
            Ok(outcome) => {
                if let Err(error) =
                    write_outcome(&out, &options.terminal, &version, scene, &outcome)
                {
                    complain(&error.to_string());
                }
                if outcome.mismatches.is_empty() {
                    say(&format!("{scene}: matches"));
                } else {
                    all_match = false;
                    say(&format!(
                        "{scene}: {} differences",
                        outcome.mismatches.len()
                    ));
                    for mismatch in outcome.mismatches.iter().take(20) {
                        say(&format!("  {mismatch}"));
                    }
                }
            }
            Err(error) => {
                all_match = false;
                say(&format!("{scene}: {error}"));
            }
        }
    }
    Ok(all_match)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("list") => {
            for terminal in TERMINALS {
                say(terminal);
            }
            Ok(true)
        }
        Some("version") if args.len() == 2 => drivers::driver(&args[1])
            .and_then(|driver| driver.version())
            .map(|version| {
                say(&version);
                true
            })
            .map_err(|error| error.to_string()),
        Some("smoke") if args.len() == 2 => drivers::driver(&args[1])
            .and_then(|mut driver| smoke(driver.as_mut()))
            .map(|()| {
                say(&format!("{}: smoke test passed", args[1]));
                true
            })
            .map_err(|error| error.to_string()),
        Some("run") => match parse_run(&args[1..]) {
            Ok(options) => run_command(&options),
            Err(error) => {
                complain(&format!("{error}\n{USAGE}"));
                return ExitCode::from(2);
            }
        },
        _ => {
            complain(USAGE);
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            complain(&error);
            ExitCode::FAILURE
        }
    }
}
