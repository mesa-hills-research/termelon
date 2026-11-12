//! The recorder on a pseudoconsole (`ConPTY`), with `termelon-vt` playing the terminal on its other
//! side. The test runs this binary again on a new pseudoconsole as the program, which records what
//! comes and writes its report to a file, while the terminal answers its queries and sends bytes at
//! each step that waits for them. A `conpty.dll` and `OpenConsole.exe` beside the binary make it
//! the pseudoconsole Windows Terminal ships rather than the system's.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use termelon_vt::{Emulator, Profile};

use super::record::{self, Step};
use crate::sys::windows::InputMode;

/// The variable that makes this binary the program, and names the file its report goes to.
const CHILD: &str = "TERMELON_RECORD_TO";

/// What the terminal sends at each step that waits for it.
const SENT: [(&str, &[u8]); 14] = [
    ("a", b"a"),
    ("Up", b"\x1b[A"),
    ("Shift+Enter as the kitty protocol sends it", b"\x1b[13;2u"),
    ("Ctrl+C", b"\x03"),
    ("Alt+x", b"\x1bx"),
    ("F5", b"\x1b[15~"),
    ("Esc", b"\x1b"),
    ("a paste", b"\x1b[200~pasted\x1b[201~"),
    (
        "Shift+Enter in win32-input-mode",
        b"\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_",
    ),
    ("an emoji", "\u{1f600}".as_bytes()),
    ("a late DA1 reply", b"\x1b[?62;22c"),
    ("a late OSC 11 reply", b"\x1b]11;rgb:0000/0000/0000\x1b\\"),
    ("an SGR mouse click", b"\x1b[<0;5;3M\x1b[<0;5;3m"),
    ("focus in", b"\x1b[I"),
];

/// What the program writes first, to see what the pseudoconsole passes on to its terminal.
const OUTPUT: &[u8] = b"\x1b[?2026h\x1b[38;2;1;2;3mrgb\x1b[0m \x1b[4:3mcurly\x1b[0m \
    \x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ \x1b[>1u\x1b[<u\x1b[?2026l[output end]";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The program's steps. Its output first, then the queries, the typed records, and what the
/// terminal sends.
fn steps() -> Vec<Step> {
    let mut steps = vec![Step::Query(
        "output, which answers nothing",
        OUTPUT.to_vec(),
    )];
    steps.extend(record::queries());
    steps.extend(record::typed());
    steps.extend(SENT.iter().map(|&(name, _)| Step::Send(name)));
    steps
}

/// What a pseudoconsole gives its program for queries, typed records and what its terminal
/// sends, in each input mode, and what it passes on to the terminal. It prints the report,
/// which says which pseudoconsole it was.
#[test]
#[ignore = "records how a pseudoconsole delivers input"]
fn how_a_pseudoconsole_delivers_replies_and_keys() {
    let steps = steps();
    if let Some(to) = std::env::var_os(CHILD) {
        let mut report = record::host();
        report.push_str(&record::record(&steps));
        std::fs::write(to, report).unwrap();
        return;
    }
    let to = std::env::temp_dir().join(format!("termelon-record-{}.txt", std::process::id()));
    let size = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pty = native_pty_system().openpty(size).unwrap();
    let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
    command.args([
        "sys::windows::tests::pseudo::how_a_pseudoconsole_delivers_replies_and_keys",
        "--exact",
        "--ignored",
        "--nocapture",
        "--test-threads",
        "1",
    ]);
    command.env(CHILD, &to);
    let mut child = pty.slave.spawn_command(command).unwrap();
    drop(pty.slave);
    // (Every answer of the terminal's says it came from the terminal, not the pseudoconsole.)
    let profile = Profile {
        da1: Some("\x1b[?61;4;9999c".to_owned()),
        xtversion: Some("termelon-vt".to_owned()),
        ..Profile::windows_terminal()
    };
    let mut emulator = Emulator::with_profile(profile, 80, 24);
    emulator.set_strict(false);
    let emulator = Arc::new(Mutex::new(emulator));
    let writer = Arc::new(Mutex::new(pty.master.take_writer().unwrap()));
    let written = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let mut reader = pty.master.try_clone_reader().unwrap();
    {
        let (emulator, writer, written) = (emulator.clone(), writer.clone(), written.clone());
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    return;
                }
                lock(&written).push(buffer[..read].to_vec());
                let answer = {
                    let mut emulator = lock(&emulator);
                    let _ = emulator.feed(&buffer[..read]);
                    emulator.take_output()
                };
                if !answer.is_empty() && lock(&writer).write_all(&answer).is_err() {
                    return;
                }
            }
        });
    }
    let mut notes = Vec::new();
    for mode in [InputMode::Vt, InputMode::Records] {
        for (index, step) in steps.iter().enumerate() {
            let Step::Send(name) = step else { continue };
            let Some(&(_, bytes)) = SENT.iter().find(|(sent, _)| sent == name) else {
                continue;
            };
            let marker = record::marker(mode, index);
            let started = Instant::now();
            while !lock(&emulator).text().contains(&marker) {
                if started.elapsed() > Duration::from_secs(60) {
                    notes.push(format!("{marker} never showed"));
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut writer = lock(&writer);
            writer.write_all(bytes).unwrap();
            writer.flush().unwrap();
        }
    }
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let report = std::fs::read_to_string(&to).unwrap_or_else(|error| format!("no report: {error}"));
    let _ = std::fs::remove_file(&to);
    println!("{report}");
    for note in &notes {
        println!("{note}");
    }
    println!("what the pseudoconsole wrote to its terminal, read by read:");
    for chunk in lock(&written).iter() {
        println!("  {}", String::from_utf8_lossy(chunk).escape_debug());
    }
    println!("the program ended with {status:?}");
    assert!(status.is_some_and(|status| status.success()));
    assert_eq!(notes, [] as [String; 0]);
}
