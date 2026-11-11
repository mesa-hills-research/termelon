//! Starts real terminals with the probe, drives them and reads back what they show.
//!
//! ```text
//! harness list
//! harness version TERMINAL
//! harness smoke TERMINAL
//! harness run TERMINAL --probe PATH [--scene NAME]... [--size COLSxROWS] [--out DIR]
//! harness keys TERMINAL --probe PATH [--mode NAME]... [--out DIR] [--fixtures DIR] [--update]
//! ```
//!
//! `run` runs each scene (every scene the probe lists, by default) and writes a report, both
//! screens in the styled-text format and an HTML page per scene into the output directory.
//!
//! `keys` presses the scripted keys and mouse events in each mode (every mode, by default), holds
//! what the terminal sent against what `tuiframe-vt` says it sends, and names each change since
//! the input fixture of the terminal's version, in `crates/tuiframe/tests/fixtures/input` by
//! default. `--update` writes the fixtures as they are now.
//!
//! Both exit with 1 when something differs or a step fails, and with 2 on a bad command line.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use harness::driver::{Error, clean_command, run};
use harness::drivers::{self, TERMINALS};
use harness::input::{self, MODES, run_input};
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
                     run TERMINAL --probe PATH [--scene NAME]... [--size COLSxROWS] [--out DIR] | \
                     keys TERMINAL --probe PATH [--mode NAME]... [--out DIR] [--fixtures DIR] [--update]";

struct RunOptions {
    terminal: String,
    probe: PathBuf,
    /// The scenes, or for `keys` the modes.
    scenes: Vec<String>,
    size: (u16, u16),
    out: PathBuf,
    /// Where earlier runs kept what they found, the fixtures of `keys`.
    kept: PathBuf,
    update: bool,
}

fn parse_size(text: &str) -> Option<(u16, u16)> {
    let (cols, rows) = text.split_once('x')?;
    Some((
        cols.parse().ok().filter(|&c| c > 0)?,
        rows.parse().ok().filter(|&r| r > 0)?,
    ))
}

fn parse_run(args: &[String], keys: bool) -> Result<RunOptions, String> {
    let mut args = args.iter();
    let terminal = args.next().ok_or("it needs a terminal")?.clone();
    let mut options = RunOptions {
        terminal,
        probe: PathBuf::new(),
        scenes: Vec::new(),
        size: (80, 24),
        out: PathBuf::from("harness-out"),
        kept: input::default_fixtures(),
        update: false,
    };
    while let Some(flag) = args.next() {
        if flag == "--update" && keys {
            options.update = true;
            continue;
        }
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--probe" => options.probe = PathBuf::from(value),
            "--scene" if !keys => options.scenes.push(value.clone()),
            "--mode" if keys && MODES.contains(&value.as_str()) => {
                options.scenes.push(value.clone());
            }
            "--mode" if keys => {
                return Err(format!("{value} isn't a mode, only {}", MODES.join(", ")));
            }
            "--fixtures" if keys => options.kept = PathBuf::from(value),
            "--size" if !keys => {
                options.size =
                    parse_size(value).ok_or_else(|| format!("{value} isn't a size like 80x24"))?;
            }
            "--out" => options.out = PathBuf::from(value),
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    if options.probe.as_os_str().is_empty() {
        return Err("it needs --probe".to_owned());
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

fn keys_command(options: &RunOptions) -> Result<bool, String> {
    let profile = drivers::profile(&options.terminal)
        .ok_or_else(|| format!("no profile for {}", options.terminal))?;
    let mut driver = drivers::driver(&options.terminal).map_err(|error| error.to_string())?;
    let version = driver.version().map_err(|error| error.to_string())?;
    say(&format!("{} {version}", options.terminal));
    std::fs::create_dir_all(&options.out)
        .map_err(|error| format!("couldn't make {}: {error}", options.out.display()))?;
    let out = std::fs::canonicalize(&options.out).unwrap_or_else(|_| options.out.clone());
    let probe = std::fs::canonicalize(&options.probe).unwrap_or_else(|_| options.probe.clone());
    let modes: Vec<String> = if options.scenes.is_empty() {
        MODES.iter().map(|&mode| mode.to_owned()).collect()
    } else {
        options.scenes.clone()
    };
    let mut all_match = true;
    for mode in &modes {
        let recording = match run_input(driver.as_mut(), &probe, mode, profile.clone(), &out) {
            Ok(recording) => recording,
            Err(Error::Unsupported(what)) => {
                say(&format!(
                    "{mode}: skipped, since this terminal can't {what}"
                ));
                continue;
            }
            Err(error) => {
                all_match = false;
                say(&format!("{mode}: {error}"));
                continue;
            }
        };
        let report = out.join(format!("{}-keys-{mode}.json", options.terminal));
        if let Err(error) = std::fs::write(
            &report,
            recording.report(&options.terminal, &version).to_text(),
        ) {
            complain(&format!("couldn't write {}: {error}", report.display()));
        }
        let mismatches: Vec<_> = recording.mismatches().collect();
        if mismatches.is_empty() {
            say(&format!(
                "{mode}: {} events as tuiframe-vt sends them",
                recording.entries.len()
            ));
        } else {
            all_match = false;
            say(&format!(
                "{mode}: {} of {} events differ from tuiframe-vt",
                mismatches.len(),
                recording.entries.len()
            ));
            for entry in mismatches {
                say(&format!(
                    "  {}: sent {}, tuiframe-vt sends {}",
                    entry.name,
                    input::shown(&entry.sent),
                    input::shown(&entry.expected)
                ));
            }
        }
        let path = input::fixture_path(&options.kept, &options.terminal, &version, mode);
        let found = recording.fixture(&options.terminal, &version);
        all_match &=
            check_kept(
                &path,
                &found,
                options.update,
                mode,
                |kept| match input::parse_fixture(kept) {
                    Ok(fixture) => input::changes(&fixture, &recording),
                    Err(error) => vec![format!("the fixture doesn't read: {error}")],
                },
            );
    }
    Ok(all_match)
}

/// Holds what a run found against what an earlier run kept at `path`, and names each change, or
/// with `update` keeps what it found there instead. `changes` compares the kept text with what
/// was found. Whether nothing changed.
fn check_kept(
    path: &Path,
    found: &str,
    update: bool,
    what: &str,
    changes: impl FnOnce(&str) -> Vec<String>,
) -> bool {
    if update {
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, found));
        return match written {
            Ok(()) => {
                say(&format!("{what}: wrote {}", path.display()));
                true
            }
            Err(error) => {
                say(&format!(
                    "{what}: couldn't write {}: {error}",
                    path.display()
                ));
                false
            }
        };
    }
    let Ok(kept) = std::fs::read_to_string(path) else {
        say(&format!(
            "{what}: nothing kept at {} yet, which --update writes",
            path.display()
        ));
        return true;
    };
    let changes = changes(&kept);
    if changes.is_empty() {
        say(&format!("{what}: as kept in {}", path.display()));
        return true;
    }
    say(&format!(
        "{what}: {} changes since {}",
        changes.len(),
        path.display()
    ));
    for change in changes {
        say(&format!("  {change}"));
    }
    false
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
        Some(command @ ("run" | "keys")) => match parse_run(&args[1..], command == "keys") {
            Ok(options) if command == "keys" => keys_command(&options),
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
