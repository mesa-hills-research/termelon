//! The xterm.js and VTE drivers against the real engines, where this machine has them: xterm.js
//! once `npm ci` has run in `tools/harness/xtermjs`, and VTE with Python's GObject bindings and a
//! display. Each test passes without running otherwise.

use std::path::Path;
use std::time::{Duration, Instant};

use harness::compare;
use harness::driver::{Driver, Launch, installed, work_dir};
use harness::drivers::{vte, xtermjs};
use harness::run::smoke;
use tuiframe_vt::{Emulator, Mouse, MouseButton, MouseKind, Profile};

const SCENE: &str = "\x1b[1;1Hplain \x1b[1mbold\x1b[0m \x1b[38;5;196mred\x1b[0m \x1b[4:3mcurly\x1b[0m\
                     \x1b[2;3H字幕 é e\u{301} ⣷…\x1b[3;1H\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\
                     \x1b[4;1H\x1b[44mblue\x1b[0m";

/// Runs [`SCENE`] with `cat` and compares the readback with `tuiframe-vt`'s screen.
#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn shows_the_same(driver: &mut dyn Driver, profile: Profile) {
    smoke(driver).unwrap();
    let dir = work_dir("live-test").unwrap();
    let file = dir.join("bytes");
    let bytes = format!("{SCENE}\x1b[0m\x1b[6;1Hmarker-done");
    std::fs::write(&file, &bytes).unwrap();
    let script = format!("cat '{}'; exec sleep 60", file.display());
    driver
        .start(&Launch::new(30, 6, &["sh", "-c", &script]))
        .unwrap();
    let readback = driver
        .wait_for("marker-done", Duration::from_secs(15))
        .unwrap();
    driver.stop().unwrap();
    let mut expected = Emulator::with_profile(profile, 30, 6);
    expected.feed(bytes.as_bytes()).unwrap();
    let mismatches = compare(&expected, &readback);
    assert!(
        mismatches.is_empty(),
        "{}",
        mismatches
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The bytes saved to `file` once there are `len` of them, or after 5 s.
fn read_until(file: &Path, len: usize) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let bytes = std::fs::read(file).unwrap_or_default();
        if bytes.len() >= len || Instant::now() >= deadline {
            return bytes;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Clicks and turns the wheel over a program that asked for SGR mouse reports, and checks it got
/// what `tuiframe-vt` sends.
#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn clicks_reach_the_program(driver: &mut dyn Driver, profile: Profile) {
    let dir = work_dir("live-mouse").unwrap();
    let file = dir.join("typed");
    let setup = "\x1b[?1000h\x1b[?1006h";
    let script = format!(
        "printf '{}'; stty raw -echo; echo ready; exec dd bs=1 of='{}' 2>/dev/null",
        setup.replace('\x1b', "\\033"),
        file.display()
    );
    driver
        .start(&Launch::new(20, 5, &["sh", "-c", &script]))
        .unwrap();
    driver.wait_for("ready", Duration::from_secs(15)).unwrap();
    let mut emulator = Emulator::with_profile(profile, 20, 5);
    emulator.feed(setup.as_bytes()).unwrap();
    let mut expected = Vec::new();
    for kind in [
        MouseKind::Press(MouseButton::Left),
        MouseKind::Release(MouseButton::Left),
        MouseKind::ScrollDown,
    ] {
        let mouse = Mouse::new(kind, 3, 2);
        driver.send_mouse(mouse).unwrap();
        expected.extend(emulator.encode_mouse(mouse));
    }
    let got = read_until(&file, expected.len());
    driver.stop().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&got),
        String::from_utf8_lossy(&expected)
    );
}

#[test]
fn xtermjs() {
    if !installed("node") || !xtermjs::script_dir().join("node_modules").is_dir() {
        return;
    }
    let mut driver = xtermjs::XtermJs::new().unwrap();
    shows_the_same(
        &mut driver,
        Profile {
            hyperlinks: true,
            ..Profile::xterm()
        },
    );
    clicks_reach_the_program(&mut driver, Profile::xterm());
}

#[test]
fn vte() {
    let display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|key| std::env::var_os(key).is_some());
    let mut driver = vte::Vte::new().unwrap();
    if !display || driver.version().is_err() {
        return;
    }
    shows_the_same(
        &mut driver,
        Profile {
            hyperlinks: true,
            ..Profile::xterm()
        },
    );
    clicks_reach_the_program(&mut driver, Profile::xterm());
}
