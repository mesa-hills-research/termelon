//! Pressing the scripted keys and mouse events in a terminal, and holding what it sent for each
//! against what `tuiframe-vt` says that terminal sends.
//!
//! The probe runs as `probe --keys MODE --input FILE --record FILE`. It turns the mode on, shows
//! `probe-keys:ready`, and saves every byte it reads. The harness presses each event and then a
//! `|`, which no event in the script sends, so the bytes split into one piece per event however
//! the terminal's writes come. A fixture keeps the pieces, a line per event with its name
//! and the bytes in hex, for the decoder's tests in `crates/tuiframe`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tuiframe_vt::{Emulator, Key, Mods, Profile};

use crate::driver::{Driver, Error, Launch};
use crate::json::Json;
use crate::run::SCENE_TIMEOUT;
use crate::script;

/// What the probe shows once the mode is on.
pub const READY: &str = "probe-keys:ready";

/// The modes the probe knows, the legacy encodings, the kitty keyboard protocol's flag 1 and
/// xterm's modifyOtherKeys at level 2.
pub const MODES: [&str; 3] = ["legacy", "disambiguate", "modify-other-keys"];

/// What the harness sends after each event.
const SEPARATOR: u8 = b'|';

/// How long the terminal may take to send everything.
const INPUT_TIMEOUT: Duration = Duration::from_secs(10);

/// An event pressed, what the terminal sent for it, and what `tuiframe-vt` expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The event's name, such as `ctrl+a` or `press left 3,2`.
    pub name: String,
    /// The bytes the terminal sent.
    pub sent: Vec<u8>,
    /// The bytes `tuiframe-vt`'s profile of the terminal sends.
    pub expected: Vec<u8>,
}

/// What a terminal sent in one mode.
#[derive(Debug)]
pub struct Recording {
    /// The mode.
    pub mode: String,
    /// Each event, in the order pressed.
    pub entries: Vec<Entry>,
}

impl Recording {
    /// The events whose bytes differ from what `tuiframe-vt` expected.
    pub fn mismatches(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.sent != entry.expected)
    }

    /// The recording as a fixture, headed by the terminal and its version.
    pub fn fixture(&self, terminal: &str, version: &str) -> String {
        let mut out = format!(
            "# What {terminal} ({version}) sent in the {} mode, in hex, for each event.\n",
            self.mode
        );
        for entry in &self.entries {
            let _ = writeln!(out, "{}\t{}", entry.name, hex(&entry.sent));
        }
        out
    }

    /// The recording as a report, with every event's bytes and what was expected.
    pub fn report(&self, terminal: &str, version: &str) -> Json {
        let entries = self
            .entries
            .iter()
            .map(|entry| {
                Json::object([
                    ("event", Json::str(entry.name.clone())),
                    ("sent", Json::str(hex(&entry.sent))),
                    ("expected", Json::str(hex(&entry.expected))),
                    ("matches", Json::Bool(entry.sent == entry.expected)),
                ])
            })
            .collect();
        Json::object([
            ("terminal", Json::str(terminal)),
            ("version", Json::str(version)),
            ("mode", Json::str(self.mode.clone())),
            ("matches", Json::Bool(self.mismatches().next().is_none())),
            ("events", Json::List(entries)),
        ])
    }
}

/// Bytes in hex, as fixtures keep them.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// A fixture's events and bytes.
///
/// # Errors
///
/// When a line has no tab or its bytes aren't hex.
pub fn parse_fixture(text: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, bytes) = line
                .split_once('\t')
                .ok_or_else(|| format!("{line:?} has no tab"))?;
            let bytes = (0..bytes.len())
                .step_by(2)
                .map(|i| {
                    bytes
                        .get(i..i + 2)
                        .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                })
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(|| format!("{line:?} has bytes that aren't hex"))?;
            Ok((name.to_owned(), bytes))
        })
        .collect()
}

/// How a recording differs from the fixture made before, a line per event.
pub fn changes(fixture: &[(String, Vec<u8>)], recording: &Recording) -> Vec<String> {
    let mut out = Vec::new();
    for entry in &recording.entries {
        match fixture.iter().find(|(name, _)| *name == entry.name) {
            Some((_, bytes)) if *bytes == entry.sent => {}
            Some((_, bytes)) => out.push(format!(
                "{}: {} before, {} now",
                entry.name,
                shown(bytes),
                shown(&entry.sent)
            )),
            None => out.push(format!("{}: new, {}", entry.name, shown(&entry.sent))),
        }
    }
    for (name, _) in fixture {
        if !recording.entries.iter().any(|entry| entry.name == *name) {
            out.push(format!("{name}: gone"));
        }
    }
    out
}

/// Bytes as a person reads them, with ESC as `\e`.
pub fn shown(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "nothing".to_owned();
    }
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\x1b' => out.push_str("\\e"),
            c if c.is_control() => {
                let _ = write!(out, "\\x{:02x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// Where the input fixtures are kept, beside the decoder they test.
pub fn default_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/tuiframe/tests/fixtures/input")
}

/// Where a terminal's fixture for a mode goes, such as `tmux-3.6-legacy/input.txt`.
pub fn fixture_path(dir: &Path, terminal: &str, version: &str, mode: &str) -> PathBuf {
    dir.join(format!("{terminal}-{}-{mode}", version_slug(version)))
        .join("input.txt")
}

/// The version number in a terminal's version, such as `3.6` in `tmux 3.6`, for file names.
pub fn version_slug(version: &str) -> String {
    version
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .map_or_else(
            || "unknown".to_owned(),
            |word| {
                word.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '.')
                    .collect()
            },
        )
}

/// Presses the script in a terminal in one mode, and holds each event's bytes against what
/// `tuiframe-vt` sends under `profile` once it's seen the probe's setup. Keys go where the driver
/// can press them, and mouse events, which no mode changes, in the legacy mode where it can make
/// them.
///
/// # Errors
///
/// When the terminal doesn't start, the probe doesn't get ready, the driver can press nothing, or
/// the bytes don't come.
pub fn run_input(
    driver: &mut dyn Driver,
    probe: &Path,
    mode: &str,
    profile: Profile,
    out: &Path,
) -> Result<Recording, Error> {
    let stem = format!("{}-keys-{mode}", driver.name());
    let input = out.join(format!("{stem}.input"));
    let record = out.join(format!("{stem}.bytes"));
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&record);
    let (probe, input_arg, record_arg) = (
        probe.display().to_string(),
        input.display().to_string(),
        record.display().to_string(),
    );
    let args = [
        probe.as_str(),
        "--keys",
        mode,
        "--input",
        &input_arg,
        "--record",
        &record_arg,
        "--hold",
        "120",
    ];
    let (cols, rows) = (80, 24);
    driver.start(&Launch::new(cols, rows, &args))?;
    let pressed = driver
        .wait_for(READY, SCENE_TIMEOUT)
        .and_then(|_| press(driver, mode == "legacy"))
        .and_then(|names| {
            wait_for_separators(&input, names.len())?;
            Ok(names)
        });
    driver.stop()?;
    let names = pressed?;
    let bytes = std::fs::read(&input).map_err(|error| Error::Io(input_arg, error))?;
    let pieces: Vec<&[u8]> = bytes.split(|&b| b == SEPARATOR).collect();
    if pieces.len() != names.len() + 1 {
        return Err(Error::Parse(format!(
            "{} events sent {} pieces, as {}",
            names.len(),
            pieces.len().saturating_sub(1),
            shown(&bytes)
        )));
    }
    let setup = std::fs::read(&record).map_err(|error| Error::Io(record_arg, error))?;
    let mut emulator = Emulator::with_profile(profile, cols, rows);
    emulator
        .feed(&setup)
        .map_err(|error| Error::Parse(format!("the probe's setup holds an {error}")))?;
    let entries = names
        .into_iter()
        .zip(pieces)
        .map(|((name, event), sent)| Entry {
            name,
            sent: sent.to_vec(),
            expected: match event {
                Event::Key(key, mods) => emulator.encode_key(key, mods),
                Event::Mouse(mouse) => emulator.encode_mouse(mouse),
            },
        })
        .collect();
    Ok(Recording {
        mode: mode.to_owned(),
        entries,
    })
}

#[derive(Clone, Copy)]
enum Event {
    Key(Key, Mods),
    Mouse(tuiframe_vt::Mouse),
}

/// Presses every key the driver can, and the mouse events if `mouse`, each followed by the
/// separator. Returns the events pressed, by name.
///
/// # Errors
///
/// When the driver fails, or can press none of them.
fn press(driver: &mut dyn Driver, mouse: bool) -> Result<Vec<(String, Event)>, Error> {
    let mut events: Vec<Event> = script::keys()
        .into_iter()
        .map(|(key, mods)| Event::Key(key, mods))
        .collect();
    if mouse {
        events.extend(script::MOUSE.map(Event::Mouse));
    }
    let mut pressed = Vec::new();
    for event in events {
        let (sent, name) = match event {
            Event::Key(key, mods) => (
                driver.send_keys(&[(key, mods)]),
                script::key_name(key, mods),
            ),
            Event::Mouse(mouse) => (driver.send_mouse(mouse), script::mouse_name(mouse)),
        };
        match sent {
            Ok(()) => {}
            // The fixture leaves out what the driver can't press.
            Err(Error::Unsupported(_)) => continue,
            Err(error) => return Err(error),
        }
        // The separator goes the way the event went, since a driver's keys and raw bytes may take
        // paths that don't keep their order, such as xdotool and a terminal's own command line.
        match event {
            Event::Key(..) => driver.send_keys(&[(Key::Char(char::from(SEPARATOR)), Mods::NONE)]),
            Event::Mouse(_) => driver.send_bytes(&[SEPARATOR]),
        }?;
        pressed.push((name, event));
    }
    if pressed.is_empty() {
        return Err(Error::Unsupported("take synthetic keys or mouse events"));
    }
    Ok(pressed)
}

/// Waits until the probe has saved `count` separators.
fn wait_for_separators(path: &Path, count: usize) -> Result<(), Error> {
    let deadline = Instant::now() + INPUT_TIMEOUT;
    loop {
        let bytes = std::fs::read(path).unwrap_or_default();
        let seen = bytes.split(|&b| b == SEPARATOR).count() - 1;
        if seen >= count {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Error::Parse(format!(
                "{seen} of {count} events came in time, as {}",
                shown(&bytes)
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording(entries: &[(&str, &[u8])]) -> Recording {
        Recording {
            mode: "legacy".to_owned(),
            entries: entries
                .iter()
                .map(|&(name, sent)| Entry {
                    name: name.to_owned(),
                    sent: sent.to_vec(),
                    expected: sent.to_vec(),
                })
                .collect(),
        }
    }

    #[test]
    fn fixtures_read_back() {
        let recording = recording(&[("a", b"a"), ("ctrl+space", b"\0"), ("f13", b"")]);
        let text = recording.fixture("tmux", "tmux 3.6");
        assert!(text.starts_with("# What tmux (tmux 3.6) sent in the legacy mode"));
        assert!(text.contains("\nctrl+space\t00\n"));
        let fixture = parse_fixture(&text).unwrap();
        assert_eq!(fixture[0], ("a".to_owned(), b"a".to_vec()));
        assert_eq!(fixture[2], ("f13".to_owned(), Vec::new()));
        assert!(changes(&fixture, &recording).is_empty());
        assert!(parse_fixture("a\t6").is_err());
        assert!(parse_fixture("a 61").is_err());
    }

    #[test]
    fn changes_name_each_event() {
        let before = parse_fixture("a\t61\nb\t62\nescape\t1b\n").unwrap();
        let now = recording(&[("a", b"a"), ("b", b"\x1bb"), ("c", b"c")]);
        assert_eq!(
            changes(&before, &now),
            ["b: b before, \\eb now", "c: new, c", "escape: gone"]
        );
    }

    #[test]
    fn versions_name_files() {
        assert_eq!(version_slug("tmux 3.6a"), "3.6a");
        assert_eq!(version_slug("VTE 0.84.1"), "0.84.1");
        assert_eq!(version_slug("@xterm/headless 6.0.0"), "6.0.0");
        assert_eq!(version_slug("xterm"), "unknown");
        assert_eq!(
            fixture_path(Path::new("f"), "tmux", "tmux 3.6", "legacy"),
            Path::new("f/tmux-3.6-legacy/input.txt")
        );
    }
}
