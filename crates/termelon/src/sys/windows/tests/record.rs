//! Records what a console delivers, for the tests that settle how Windows consoles behave. Each
//! step writes a query, types key records with `WriteConsoleInputW`, or waits for what the
//! terminal on a pseudoconsole's other side sends, and keeps every record that comes back, read by
//! read. The records then go through the library's reader on a console in memory, so the report
//! says what the library makes of them too.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::console::testing::write_input;
use super::super::modes::{InputSettings, Modes};
use super::super::reader::Settings;
use super::super::{Console, Input, InputMode, Ready, WinConsole};
use super::{Fake, key_record, reading};
use crate::event::Event;
use crate::event::decode::{Query, Reply};
use crate::event::records::{InputRecord, KeyRecord};
use crate::sys::Next;

/// How long a console may be quiet once it has sent something before that's taken as all.
const QUIET: Duration = Duration::from_millis(300);

/// How long a step waits for the first record.
const FIRST: Duration = Duration::from_secs(1);

/// How long a step that waits for the terminal waits for the first record.
const SENT_WAIT: Duration = Duration::from_secs(5);

/// One thing the recorder does, and the name the report gives it.
#[derive(Debug, Clone)]
pub(super) enum Step {
    /// Writes a query to the console.
    Query(&'static str, Vec<u8>),
    /// Puts records into the console's input with `WriteConsoleInputW`.
    Type(&'static str, Vec<InputRecord>),
    /// Shows [`marker`] and waits for the terminal on a pseudoconsole's other side to send bytes.
    Send(&'static str),
}

/// The queries, one at a time, then all of the probe's at once.
pub(super) fn queries() -> Vec<Step> {
    let one = |name, query: &str| Step::Query(name, query.as_bytes().to_vec());
    vec![
        one("DA1", "\x1b[c"),
        one("DA2", "\x1b[>c"),
        one("cursor report", "\x1b[6n"),
        one("DECRQM 2026", "\x1b[?2026$p"),
        one("XTVERSION", "\x1b[>q"),
        one("OSC 11", "\x1b]11;?\x1b\\"),
        one("kitty flags", "\x1b[?u"),
        one("size in cells", "\x1b[18t"),
        Step::Query("the probe", crate::caps::probe::queries(false)),
    ]
}

/// A key's records as a keyboard gives them, its press and its release.
fn press(virtual_key: u16, scan_code: u16, unicode: u16, control_state: u32) -> Vec<InputRecord> {
    let with_scan = |record| match record {
        InputRecord::Key(key) => InputRecord::Key(KeyRecord { scan_code, ..key }),
        other => other,
    };
    vec![
        with_scan(key_record(true, virtual_key, unicode, control_state)),
        with_scan(key_record(false, virtual_key, unicode, control_state)),
    ]
}

/// Keys typed as records, as conhost's window makes them from a US keyboard (Alt Gr as a German
/// keyboard makes it).
pub(super) fn typed() -> Vec<Step> {
    let (shift, ctrl, alt, right_alt, enhanced) = (0x10, 0x08, 0x02, 0x01, 0x100);
    vec![
        Step::Type("a", press(0x41, 0x1e, u16::from(b'a'), 0)),
        Step::Type("Shift+Enter", press(0x0d, 0x1c, 0x0d, shift)),
        Step::Type("Up", press(0x26, 0x48, 0, enhanced)),
        Step::Type("Ctrl+C", press(0x43, 0x2e, 0x03, ctrl)),
        Step::Type("Alt+x", press(0x58, 0x2d, u16::from(b'x'), alt)),
        Step::Type("F5", press(0x74, 0x3f, 0, 0)),
        Step::Type("Esc", press(0x1b, 0x01, 0x1b, 0)),
        Step::Type(
            "Alt Gr+q",
            press(0x51, 0x10, u16::from(b'@'), right_alt | ctrl),
        ),
        Step::Type("Shift alone", press(0x10, 0x2a, 0, shift)),
        Step::Type(
            "an emoji as two packets",
            [0xd83d, 0xde00]
                .into_iter()
                .flat_map(|unit| press(0xe7, 0, unit, 0))
                .collect(),
        ),
        Step::Type(
            "a reply as characters with no key",
            "\x1b[?62;22c"
                .encode_utf16()
                .flat_map(|unit| press(0, 0, unit, 0))
                .collect(),
        ),
    ]
}

/// What a step that waits for the terminal shows, for the terminal to send its bytes on.
pub(super) fn marker(mode: InputMode, index: usize) -> String {
    format!("[send {mode:?} {index:02}]")
}

/// Runs `steps` in each input mode on the console the process has, and reports what came.
pub(super) fn record(steps: &[Step]) -> String {
    let console = Arc::new(WinConsole::open().unwrap());
    let conin = OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")
        .unwrap();
    let mut report = String::new();
    for mode in [InputMode::Vt, InputMode::Records] {
        let modes = Modes::enter(console.clone(), InputSettings { mode, mouse: false }).unwrap();
        let (input, _wake) = console.input().unwrap();
        let mut out = console.output().unwrap();
        for (index, step) in steps.iter().enumerate() {
            drain(&input);
            let (kind, name, wait) = match step {
                Step::Query(name, query) => {
                    out.write_all(query).unwrap();
                    ("query", *name, FIRST)
                }
                Step::Type(name, records) => {
                    write_input(&conin, records).unwrap();
                    ("typed", *name, FIRST)
                }
                Step::Send(name) => {
                    let shown = format!("\x1b[H\x1b[2K{}", marker(mode, index));
                    out.write_all(shown.as_bytes()).unwrap();
                    ("sent", *name, SENT_WAIT)
                }
            };
            let reads = collect(&input, wait);
            let _ = writeln!(report, "{mode:?}, {kind} {name}:");
            if reads.is_empty() {
                let _ = writeln!(report, "  nothing came");
            }
            for read in &reads {
                let shown: Vec<String> = read.iter().map(show).collect();
                let _ = writeln!(report, "  read {}: {}", read.len(), shown.join(" "));
            }
            let _ = writeln!(report, "  characters {:?}", characters(&reads));
            let expects =
                matches!(step, Step::Query(_, query) if query.windows(3).any(|w| w == b"[6n"));
            let (events, replies) = replay(&reads, mode, expects);
            let _ = writeln!(report, "  the library's events {events:?}");
            let _ = writeln!(report, "  the library's replies {replies:?}");
        }
        drop(out);
        modes.restore().unwrap();
    }
    report
}

/// Reads and drops what's waiting.
fn drain(input: &impl Input) {
    let mut records = Vec::new();
    while input.more().unwrap_or(false) {
        records.clear();
        if input.read(&mut records).is_err() {
            return;
        }
    }
}

/// The records that come, read by read, waiting `first` for the first and [`QUIET`] after each.
fn collect(input: &impl Input, first: Duration) -> Vec<Vec<InputRecord>> {
    let started = Instant::now();
    let mut reads = Vec::new();
    let mut last: Option<Instant> = None;
    while started.elapsed() < first + Duration::from_secs(5) {
        let left = match last {
            None => first.saturating_sub(started.elapsed()),
            Some(at) => QUIET.saturating_sub(at.elapsed()),
        };
        if left.is_zero() {
            break;
        }
        if input.wait(Some(left)).unwrap() != Ready::Input {
            continue;
        }
        let mut read = Vec::new();
        input.read(&mut read).unwrap();
        if !read.is_empty() {
            reads.push(read);
            last = Some(Instant::now());
        }
    }
    reads
}

/// A record, compactly: `K+` or `K-` for a key's press or release, then its virtual key, scan
/// code, UTF-16 unit and control key state in hex, and its repeat count where it isn't 1.
pub(super) fn show(record: &InputRecord) -> String {
    match record {
        InputRecord::Key(key) => {
            let repeat = if key.repeat == 1 {
                String::new()
            } else {
                format!("x{}", key.repeat)
            };
            format!(
                "K{}{:02x}:{:02x}:{:04x}:{:04x}{repeat}",
                if key.key_down { '+' } else { '-' },
                key.virtual_key,
                key.scan_code,
                key.unicode,
                key.control_state
            )
        }
        InputRecord::Mouse(mouse) => format!(
            "M{},{}:{:x}:{:x}:{:x}",
            mouse.column, mouse.row, mouse.button_state, mouse.control_state, mouse.event_flags
        ),
        InputRecord::Focus(focus) => format!("F{}", if focus.focused { '+' } else { '-' }),
        InputRecord::WindowSize(_) => "S".to_owned(),
    }
}

/// The characters of the key-down records, as the bytes a terminal would have sent.
fn characters(reads: &[Vec<InputRecord>]) -> String {
    let units: Vec<u16> = reads
        .iter()
        .flatten()
        .filter_map(|record| match record {
            InputRecord::Key(key) if key.key_down && key.unicode != 0 => Some(key.unicode),
            _ => None,
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// What the library's reader makes of `reads` in `mode`, read by read as they came, with the
/// escape timeout `Terminal` uses then. With `expects`, the decoder expects a cursor report, as
/// the probe makes it.
fn replay(reads: &[Vec<InputRecord>], mode: InputMode, expects: bool) -> (Vec<Event>, Vec<Reply>) {
    let fake = Fake::new();
    let settings = Arc::new(Settings::default());
    settings.set_mode(mode);
    let escape = match mode {
        InputMode::Vt => Duration::from_millis(100),
        InputMode::Records => Duration::from_millis(20),
    };
    let (reader, shared) = reading(&fake, &settings, escape);
    shared.replies.open();
    if expects {
        reader.expect(Query::CursorPosition);
    }
    for read in reads {
        fake.type_in(read);
        let started = Instant::now();
        while !fake.with(|state| state.records.is_empty()) && started.elapsed() < FIRST {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let mut events = Vec::new();
    while let Next::Event(event) = shared.events.next_timeout(QUIET) {
        events.push(event);
    }
    let replies = shared.replies.close();
    reader.stop();
    while let Next::Event(event) = shared.events.next_timeout(Duration::ZERO) {
        events.push(event);
    }
    (events, replies)
}

/// The system, its console host and the process chain, as PowerShell sees them from this
/// console.
pub(super) fn host() -> String {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
$os = [Environment]::OSVersion.Version
$conhost = (Get-Item "$env:windir\System32\conhost.exe").VersionInfo.FileVersionRaw
Add-Type -Namespace TermelonTest -Name Host -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(System.IntPtr window, System.Text.StringBuilder name, int size);
'@
$window = [TermelonTest.Host]::GetConsoleWindow()
$class = New-Object System.Text.StringBuilder 256
[void][TermelonTest.Host]::GetClassName($window, $class, 256)
"windows $os, conhost.exe $conhost"
"console window $window, class '$class'"
$chain = @()
$ids = @()
$id = $PID
for ($i = 0; $i -lt 8 -and $id; $i++) {
    $process = Get-CimInstance Win32_Process -Filter "ProcessId=$id"
    if (-not $process) { break }
    $chain += "$($process.Name) $id"
    $ids += $id
    $id = $process.ParentProcessId
}
"processes: " + ($chain -join ' < ')
Get-CimInstance Win32_Process -Filter "Name='conhost.exe' or Name='OpenConsole.exe' or Name='WindowsTerminal.exe'" |
    ForEach-Object { "host $($_.Name) $($_.ProcessId), parent $($_.ParentProcessId)$(if ($ids -contains $_.ParentProcessId) { ' (ours)' }), $($_.ExecutablePath)" }
"#;
    let script = std::env::temp_dir().join(format!("termelon-host-{}.ps1", std::process::id()));
    std::fs::write(&script, SCRIPT).unwrap();
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .output();
    let _ = std::fs::remove_file(&script);
    let mut text = match output {
        Ok(output) => format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => format!("powershell didn't run: {error}\n"),
    };
    let _ = writeln!(
        text,
        "WT_SESSION {}, conpty.dll beside the binary {}",
        std::env::var_os("WT_SESSION").is_some(),
        std::env::current_exe().is_ok_and(|exe| exe.with_file_name("conpty.dll").exists())
    );
    text
}
