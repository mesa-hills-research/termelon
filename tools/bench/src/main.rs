//! Workloads that compare the framework's output layer with ratatui and crossterm.
//!
//! Each workload is a sequence of frames. A stack draws them into memory, and the command reports
//! what each frame cost to build, to diff and to encode, how many bytes it wrote, and how many
//! allocations it made. `bench --help` says how to run it.

mod alloc;
mod content;
mod format;
mod model;
mod ratatui_stack;
mod report;
mod run;
mod scene;
mod stack;
mod stats;
mod termelon_stack;
mod workload;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::model::Frame;
use crate::report::Row;
use crate::run::Sample;
use crate::workload::Workload;

const USAGE: &str = "\
Usage: bench [WORKLOAD] [OPTIONS]

WORKLOAD is one of idle, streaming, scrolling, typing, full-redraw, or all (the default). With
--recordings, idle-rec, streaming-rec, scrolling-rec, typing-rec and full-redraw-rec are the same
five replayed from frames recorded from Arko, and all includes them.

Options:
  --frames N       timed frames per workload, instead of its own count
  --repeat N       how many times to play each workload on each stack (default 1)
  --stack NAME     a stack to measure, ratatui, termelon or termelon-scroll, or all (default)
  --recordings DIR the directory with idle.styled, streaming.styled, scrolling.styled,
                   typing.styled and redraw.styled
  --file PATH      play the frames in a styled-text file, the first one untimed, in place of WORKLOAD
  --dump           print the frames of WORKLOAD or --file in the styled-text format and stop
  --help           print this text
";

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    workloads: Vec<String>,
    frames: Option<usize>,
    repeat: usize,
    stacks: Vec<String>,
    file: Option<PathBuf>,
    recordings: Option<PathBuf>,
    dump: bool,
    help: bool,
}

fn value(flag: &str, args: &mut std::slice::Iter<'_, String>) -> Result<String, String> {
    args.next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn count(flag: &str, text: &str) -> Result<usize, String> {
    match text.parse::<usize>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(format!("{flag} needs a whole number above 0, not `{text}`")),
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        workloads: Vec::new(),
        frames: None,
        repeat: 1,
        stacks: Vec::new(),
        file: None,
        recordings: None,
        dump: false,
        help: false,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => options.help = true,
            "--dump" => options.dump = true,
            "--frames" => options.frames = Some(count("--frames", &value("--frames", &mut args)?)?),
            "--repeat" => options.repeat = count("--repeat", &value("--repeat", &mut args)?)?,
            "--stack" => {
                let name = value("--stack", &mut args)?;
                if name != "all" {
                    options.stacks.push(name);
                }
            }
            "--file" => options.file = Some(PathBuf::from(value("--file", &mut args)?)),
            "--recordings" => {
                options.recordings = Some(PathBuf::from(value("--recordings", &mut args)?));
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option `{flag}`")),
            name => {
                if name != "all" {
                    options.workloads.push(name.to_string());
                }
            }
        }
    }
    if options.stacks.is_empty() {
        options.stacks = stack::NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
    }
    if let Some(unknown) = options
        .stacks
        .iter()
        .find(|name| !stack::NAMES.contains(&name.as_str()))
    {
        return Err(format!(
            "unknown stack `{unknown}`, the stacks are {}",
            stack::NAMES.join(", ")
        ));
    }
    if options.file.is_some() {
        if !options.workloads.is_empty() || options.recordings.is_some() {
            return Err("--file takes the place of a workload and of --recordings".to_string());
        }
    } else if options.workloads.is_empty() {
        options.workloads = workload::NAMES
            .iter()
            .map(|name| (*name).to_string())
            .chain(options.recordings.iter().flat_map(|_| {
                workload::RECORDED
                    .iter()
                    .map(|(name, _)| (*name).to_string())
            }))
            .collect();
    }
    for name in &options.workloads {
        let recorded = workload::recorded_file(name).is_some();
        if !recorded && !workload::NAMES.contains(&name.as_str()) {
            return Err(format!(
                "unknown workload `{name}`, the workloads are {}, and with --recordings {}",
                workload::NAMES.join(", "),
                workload::RECORDED.map(|(name, _)| name).join(", ")
            ));
        }
        if recorded && options.recordings.is_none() {
            return Err(format!("{name} needs --recordings DIR"));
        }
    }
    Ok(options)
}

/// The workloads to play.
fn load(options: &Options) -> Result<Vec<Workload>, String> {
    if let Some(path) = &options.file {
        return Ok(vec![read_file(path, options.frames, false)?]);
    }
    options
        .workloads
        .iter()
        .map(|name| {
            if let (Some(file), Some(directory)) =
                (workload::recorded_file(name), &options.recordings)
            {
                let repaint = name.starts_with("full-redraw");
                let mut recorded = read_file(&directory.join(file), options.frames, repaint)?;
                recorded.name.clone_from(name);
                return Ok(recorded);
            }
            let frames = options
                .frames
                .unwrap_or_else(|| workload::default_frames(name));
            workload::build(name, frames).ok_or_else(|| format!("unknown workload `{name}`"))
        })
        .collect()
}

/// The frames in the file at `path` as a workload named after the file. `frames` limits the timed
/// ones. With `repaint` each is drawn after a resize.
fn read_file(path: &Path, frames: Option<usize>, repaint: bool) -> Result<Workload, String> {
    let input = std::fs::read_to_string(path)
        .map_err(|error| format!("couldn't read {}: {error}", path.display()))?;
    let mut frames_in_file =
        format::parse(&input).map_err(|error| format!("{}: {error}", path.display()))?;
    if frames_in_file.is_empty() {
        return Err(format!("{} has no frames", path.display()));
    }
    if let Some(frames) = frames {
        // The first frame is untimed unless every frame is repainted.
        frames_in_file.truncate(if repaint { frames } else { frames + 1 });
    }
    let name = path.file_stem().map_or_else(
        || "file".to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    Ok(workload::from_frames(&name, frames_in_file, repaint))
}

/// The standard output, which the report goes to.
#[expect(clippy::disallowed_methods, reason = "this tool prints its report")]
fn stdout() -> io::Stdout {
    io::stdout()
}

/// The standard error, which problems go to.
#[expect(clippy::disallowed_methods, reason = "this tool prints its problems")]
fn stderr() -> io::Stderr {
    io::stderr()
}

fn play(options: &Options, workloads: &[Workload]) -> Result<String, String> {
    // The samples of each repeat, by workload and then stack.
    let mut results: Vec<Vec<Vec<Vec<Sample>>>> =
        vec![vec![Vec::new(); options.stacks.len()]; workloads.len()];
    for repeat in 0..options.repeat {
        for (at, workload) in workloads.iter().enumerate() {
            // Each repeat starts from a different stack, so none is always first.
            for turn in 0..options.stacks.len() {
                let stack = (repeat + turn) % options.stacks.len();
                let samples = run::run(&options.stacks[stack], workload)?;
                results[at][stack].push(samples);
            }
        }
    }
    let mut rows = Vec::new();
    for (workload, by_stack) in workloads.iter().zip(&results) {
        for (name, repeats) in options.stacks.iter().zip(by_stack) {
            rows.push(Row {
                workload: &workload.name,
                stack: name,
                summary: stats::summarize(repeats),
                spread: stats::spread(repeats),
            });
        }
    }
    let stacks: Vec<&str> = options
        .stacks
        .iter()
        .map(|name| stack::describe(name))
        .collect();
    Ok(format!(
        "# {}, {} build, {} repeat{}\n{}",
        stacks.join("; "),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        options.repeat,
        if options.repeat == 1 { "" } else { "s" },
        report::tables(&rows)
    ))
}

fn dump(workloads: &[Workload]) -> String {
    let frames: Vec<Frame> = workloads
        .iter()
        .flat_map(|workload| workload.warmup.iter().chain(&workload.frames).cloned())
        .collect();
    format::write(&frames)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = parse_args(&args).and_then(|options| {
        if options.help {
            return Ok(USAGE.to_string());
        }
        let workloads = load(&options)?;
        if options.dump {
            Ok(dump(&workloads))
        } else {
            play(&options, &workloads)
        }
    });
    match result {
        Ok(text) => {
            if stdout().lock().write_all(text.as_bytes()).is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(message) => {
            let _ = writeln!(stderr().lock(), "bench: {message}\nTry `bench --help`.");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn no_arguments_run_everything_once_on_every_stack() {
        let options = parse_args(&[]).unwrap();
        assert_eq!(options.workloads, workload::NAMES);
        assert_eq!(options.stacks, stack::NAMES);
        assert_eq!(options.repeat, 1);
        assert_eq!(options.frames, None);
        assert!(options.file.is_none() && !options.dump && !options.help);
    }

    #[test]
    fn options_are_read() {
        let options =
            parse_args(&args("typing --frames 7 --repeat 3 --stack ratatui --dump")).unwrap();
        assert_eq!(options.workloads, ["typing"]);
        assert_eq!(options.frames, Some(7));
        assert_eq!(options.repeat, 3);
        assert_eq!(options.stacks, ["ratatui"]);
        assert!(options.dump);
        let options = parse_args(&args("--file frames.txt")).unwrap();
        assert_eq!(options.file, Some(PathBuf::from("frames.txt")));
        assert!(options.workloads.is_empty());
        assert_eq!(
            parse_args(&args("all --stack all"))
                .unwrap()
                .workloads
                .len(),
            5
        );
        assert!(parse_args(&args("--help")).unwrap().help);
    }

    #[test]
    fn bad_arguments_say_why() {
        for (line, wanted) in [
            ("sprint", "unknown workload `sprint`"),
            ("--stack foo", "unknown stack `foo`"),
            ("--frames", "--frames needs a value"),
            ("--frames 0", "above 0"),
            ("--repeat x", "not `x`"),
            ("--loud", "unknown option"),
            ("idle --file a.txt", "takes the place"),
        ] {
            let error = parse_args(&args(line)).unwrap_err();
            assert!(error.contains(wanted), "{line}: {error}");
        }
    }

    #[test]
    fn workloads_load_with_their_own_frame_counts() {
        let workloads = load(&parse_args(&args("idle typing")).unwrap()).unwrap();
        assert_eq!(workloads.len(), 2);
        assert_eq!(workloads[0].frames.len(), 100);
        let few = load(&parse_args(&args("typing --frames 3")).unwrap()).unwrap();
        assert_eq!(few[0].frames.len(), 3);
    }

    #[test]
    fn a_report_has_a_row_for_each_workload_and_stack() {
        let options = parse_args(&args("typing idle --frames 4 --repeat 2")).unwrap();
        let report = play(&options, &load(&options).unwrap()).unwrap();
        let lines: Vec<&str> = report.lines().collect();
        assert!(lines[0].starts_with("# ratatui 0.30"), "{report}");
        assert!(lines[0].ends_with("2 repeats"), "{report}");
        assert!(lines[1].starts_with("workload"));
        let stacks = stack::NAMES.len();
        assert!(
            lines[2..2 + stacks]
                .iter()
                .all(|line| line.starts_with("typing")),
            "{report}"
        );
        assert!(lines[2 + stacks].starts_with("idle"), "{report}");
        assert!(
            lines.iter().any(|line| line.contains("encode_pct")),
            "{report}"
        );
    }

    #[test]
    fn a_file_is_played_and_dumped() {
        let directory = std::env::temp_dir().join(format!("bench-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("session.txt");
        let source = dump(&[workload::build("typing", 3).unwrap()]);
        std::fs::write(&path, &source).unwrap();
        let options = parse_args(&["--file".to_string(), path.display().to_string()]).unwrap();
        let workloads = load(&options).unwrap();
        assert_eq!(workloads[0].name, "session");
        assert_eq!(workloads[0].warmup.len(), 1);
        assert_eq!(workloads[0].frames.len(), 3);
        let report = play(&options, &workloads).unwrap();
        assert!(
            report.lines().nth(2).unwrap().starts_with("session"),
            "{report}"
        );
        // The recorded frames are the generated ones.
        assert_eq!(dump(&workloads), source);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
        assert!(load(&options).unwrap_err().contains("couldn't read"));
    }

    /// A directory with the five recordings, each a few frames of the workload it is named for.
    fn recordings(tag: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("bench-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for ((_, file), name) in workload::RECORDED.iter().zip(workload::NAMES) {
            let frames = dump(&[workload::build(name, 3).unwrap()]);
            std::fs::write(directory.join(file), frames).unwrap();
        }
        directory
    }

    #[test]
    fn recordings_add_five_workloads_beside_the_generated_ones() {
        let directory = recordings("recorded");
        let line = format!("--recordings {} --repeat 2 --frames 3", directory.display());
        let options = parse_args(&args(&line)).unwrap();
        assert_eq!(options.workloads.len(), 10);
        let workloads = load(&options).unwrap();
        let names: Vec<&str> = workloads.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(&names[..5], workload::NAMES);
        assert_eq!(
            &names[5..],
            [
                "idle-rec",
                "streaming-rec",
                "scrolling-rec",
                "typing-rec",
                "full-redraw-rec"
            ]
        );
        // The first frame of a recording is untimed.
        assert_eq!(workloads[5].warmup.len(), 1);
        assert_eq!(workloads[5].frames.len(), 3);
        assert!(!workloads[5].repaint);
        // The full redraw is repainted, every frame timed.
        assert!(workloads[9].repaint);
        assert_eq!(workloads[9].frames.len(), 3);
        let report = play(&options, &workloads).unwrap();
        assert!(
            report
                .lines()
                .any(|line| line.starts_with("full-redraw-rec ")),
            "{report}"
        );
        // One recording alone, cut short.
        let line = format!("typing-rec --frames 2 --recordings {}", directory.display());
        let options = parse_args(&args(&line)).unwrap();
        let workloads = load(&options).unwrap();
        assert_eq!(workloads.len(), 1);
        assert_eq!(workloads[0].frames.len(), 2);
        let line = format!(
            "full-redraw-rec --frames 2 --recordings {}",
            directory.display()
        );
        let workloads = load(&parse_args(&args(&line)).unwrap()).unwrap();
        assert_eq!(workloads[0].frames.len(), 2);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn recordings_need_the_option_and_the_files() {
        let error = parse_args(&args("idle-rec")).unwrap_err();
        assert!(error.contains("needs --recordings"), "{error}");
        let error = parse_args(&args("--file a.txt --recordings d")).unwrap_err();
        assert!(error.contains("takes the place"), "{error}");
        let options = parse_args(&args("idle-rec --recordings /nowhere/at/all")).unwrap();
        let error = load(&options).unwrap_err();
        assert!(
            error.contains("couldn't read /nowhere/at/all/idle.styled"),
            "{error}"
        );
        // Without the option, only the generated ones run.
        assert_eq!(parse_args(&[]).unwrap().workloads.len(), 5);
    }

    #[test]
    fn a_file_with_a_mistake_names_it() {
        let directory = std::env::temp_dir().join(format!("bench-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("bad.txt");
        std::fs::write(&path, "size 4 2\nstyle 0 0 9 bold\n").unwrap();
        let error = read_file(&path, None, false).unwrap_err();
        assert!(
            error.contains("bad.txt") && error.contains("line 2"),
            "{error}"
        );
        std::fs::write(&path, "# nothing\n").unwrap();
        assert!(
            read_file(&path, None, false)
                .unwrap_err()
                .contains("no frames")
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
