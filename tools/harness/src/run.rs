//! Running a scene in a terminal and comparing what it shows with `termelon-vt`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use termelon_vt::{Emulator, Key, Mods, Profile};

use crate::compare::{Mismatch, compare};
use crate::driver::{Driver, Error, Launch, Readback};
use crate::json::Json;

/// How long a scene may take to show its marker.
pub const SCENE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the smoke test prints and waits for.
pub const SMOKE_MARKER: &str = "harness-smoke-ok";

/// The marker a probe writes after a scene, which the harness waits for.
pub fn marker(scene: &str) -> String {
    format!("probe-done:{scene}")
}

/// Starts a terminal running `echo`, reads it back and stops it, so a broken runner image is told
/// apart from a failure of the program under test.
///
/// # Errors
///
/// When the terminal doesn't start or the text doesn't show.
pub fn smoke(driver: &mut dyn Driver) -> Result<(), Error> {
    let script = format!("echo {SMOKE_MARKER}; exec sleep 60");
    driver.start(&Launch::new(40, 10, &["sh", "-c", &script]))?;
    let result = driver.wait_for(SMOKE_MARKER, SCENE_TIMEOUT).map(drop);
    driver.stop()?;
    result
}

/// A scene's result.
#[derive(Debug)]
pub struct Outcome {
    /// What the terminal showed.
    pub readback: Readback,
    /// What `termelon-vt` computed for the bytes the program wrote.
    pub expected: Emulator,
    /// How the two differ.
    pub mismatches: Vec<Mismatch>,
    /// Where the screenshot went, if one was taken.
    pub screenshot: Option<PathBuf>,
}

/// Runs a probe's scene in a terminal and compares the screen with what `termelon-vt` computes
/// under `profile` for the bytes the probe recorded.
///
/// The probe runs as `probe --scene NAME --record FILE --report FILE --size COLSxROWS`. It draws
/// the scene with its marker and holds the screen until `q`, then saves every byte the framework
/// wrote and writes its report. The harness presses `q` after reading the screen back, as bytes
/// where the terminal takes them and as a key otherwise, so the report has the capabilities the
/// probe found.
///
/// # Errors
///
/// When the terminal doesn't start, the marker doesn't show (with the probe's own error, if it
/// wrote one), or the recording can't be read.
pub fn run_scene(
    driver: &mut dyn Driver,
    probe: &Path,
    scene: &str,
    (cols, rows): (u16, u16),
    profile: Profile,
    out: &Path,
) -> Result<Outcome, Error> {
    let stem = format!("{}-{scene}", driver.name());
    let record = out.join(format!("{stem}.bytes"));
    let probe_report = out.join(format!("{stem}.probe.json"));
    let _ = std::fs::remove_file(&record);
    let _ = std::fs::remove_file(&probe_report);
    let (probe, target, report_path, size) = (
        probe.display().to_string(),
        record.display().to_string(),
        probe_report.display().to_string(),
        format!("{cols}x{rows}"),
    );
    let args = [
        probe.as_str(),
        "--scene",
        scene,
        "--record",
        &target,
        "--report",
        &report_path,
        "--size",
        &size,
    ];
    let started = driver.start(&Launch::new(cols, rows, &args));
    let result = started.and_then(|()| driver.wait_for(&marker(scene), SCENE_TIMEOUT));
    let screenshot = out.join(format!("{stem}.png"));
    let screenshot = match driver.screenshot(&screenshot) {
        Ok(true) => Some(screenshot),
        _ => None,
    };
    // The probe writes its report once the frame is drawn, and again as it ends.
    let drawn_report = std::fs::read(&probe_report).ok();
    let quit = driver
        .send_bytes(b"q")
        .or_else(|_| driver.send_keys(&[(Key::Char('q'), Mods::NONE)]));
    if quit.is_ok() {
        wait_for_change(
            &probe_report,
            drawn_report.as_deref(),
            Duration::from_secs(3),
        );
    }
    driver.stop()?;
    let readback = result.map_err(|error| match probe_error(&probe_report) {
        Some(probe) => Error::Parse(format!("{error}, and the probe said {probe}")),
        None => error,
    })?;
    let bytes = std::fs::read(&record).map_err(|error| Error::Io(target.clone(), error))?;
    let mut expected = Emulator::with_profile(profile, cols, rows);
    expected
        .feed(&bytes)
        .map_err(|error| Error::Parse(format!("the probe's bytes hold an {error}")))?;
    let mismatches = compare(&expected, &readback);
    Ok(Outcome {
        readback,
        expected,
        mismatches,
        screenshot,
    })
}

/// Waits until the file at `path` holds something other than `before`.
fn wait_for_change(path: &Path, before: Option<&[u8]>, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match std::fs::read(path) {
            Ok(now) if Some(now.as_slice()) != before => return,
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// The error a probe's report gives, if it has one.
fn probe_error(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let json = Json::parse(&text).ok()?;
    json.get("error").and_then(Json::as_str).map(str::to_owned)
}

/// The report of a scene, as JSON.
pub fn report(terminal: &str, version: &str, scene: &str, outcome: &Outcome) -> Json {
    let rows = |screen: &Emulator| Json::List(screen.rows().into_iter().map(Json::Str).collect());
    let (cols, height) = outcome.expected.size();
    Json::object([
        ("terminal", Json::str(terminal)),
        ("version", Json::str(version)),
        ("scene", Json::str(scene)),
        (
            "size",
            Json::List(vec![
                Json::Int(i64::from(cols)),
                Json::Int(i64::from(height)),
            ]),
        ),
        ("matches", Json::Bool(outcome.mismatches.is_empty())),
        (
            "mismatches",
            Json::List(
                outcome
                    .mismatches
                    .iter()
                    .map(|m| {
                        Json::object([
                            ("x", Json::Int(i64::from(m.x))),
                            ("y", Json::Int(i64::from(m.y))),
                            ("what", Json::str(m.what)),
                            ("expected", Json::str(m.expected.clone())),
                            ("got", Json::str(m.got.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("expected", rows(&outcome.expected)),
        ("shown", rows(&outcome.readback.screen)),
        (
            "screenshot",
            outcome
                .screenshot
                .as_ref()
                .map_or(Json::Null, |path| Json::str(path.display().to_string())),
        ),
    ])
}

/// Writes a scene's report, both screens in the styled-text format, an HTML page with the two side
/// by side, and what the terminal gave as it gave it, into `out`.
///
/// # Errors
///
/// When a file can't be written.
pub fn write_outcome(
    out: &Path,
    terminal: &str,
    version: &str,
    scene: &str,
    outcome: &Outcome,
) -> Result<(), Error> {
    let stem = format!("{terminal}-{scene}");
    let page = format!(
        "<!DOCTYPE html>\n<meta charset=\"utf-8\">\n<title>{stem}</title>\n<h1>{stem}</h1>\n<p>{version}</p>\n\
         <h2>termelon-vt</h2>\n{}\n<h2>{terminal}</h2>\n{}\n",
        outcome.expected.to_html(),
        outcome.readback.screen.to_html()
    );
    let files = [
        (
            format!("{stem}.json"),
            report(terminal, version, scene, outcome).to_text(),
        ),
        (format!("{stem}.expected.styled"), outcome.expected.styled()),
        (
            format!("{stem}.shown.styled"),
            outcome.readback.screen.styled(),
        ),
        (format!("{stem}.html"), page),
    ];
    let raw = outcome
        .readback
        .raw
        .iter()
        .map(|(part, bytes)| (format!("{stem}.readback.{part}"), bytes.clone()));
    for (name, bytes) in files
        .into_iter()
        .map(|(name, text)| (name, text.into_bytes()))
        .chain(raw)
    {
        let path = out.join(name);
        std::fs::write(&path, bytes)
            .map_err(|error| Error::Io(path.display().to_string(), error))?;
    }
    Ok(())
}
