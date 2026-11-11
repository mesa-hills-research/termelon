//! The GNU screen driver against the screen and tmux on this machine. Each test passes without
//! running when either isn't installed.

use std::time::{Duration, Instant};

use harness::compare;
use harness::driver::{Driver, Launch, installed, work_dir};
use harness::drivers::screen::Screen;
use harness::run::smoke;
use tuiframe_vt::{Emulator, Key, Mods, Profile};

#[expect(
    clippy::expect_used,
    reason = "a helper of tests, which fail on an error"
)]
fn screen() -> Option<Screen> {
    (installed("screen") && installed("tmux")).then(|| Screen::new().expect("a work directory"))
}

#[test]
fn smoke_test() {
    let Some(mut screen) = screen() else { return };
    smoke(&mut screen).unwrap();
    assert!(screen.version().unwrap().contains("Screen version"));
}

#[test]
fn text_reads_back_at_the_size_asked() {
    let Some(mut screen) = screen() else { return };
    let dir = work_dir("screen-test").unwrap();
    let file = dir.join("bytes");
    let bytes = "\x1b[1;1Hcafé ñ\x1b[2;3H字 ⣷ ok\x1b[3;28Hedge\x1b[5;1Hmarker-done";
    std::fs::write(&file, bytes).unwrap();
    let script = format!("stty size; cat '{}'; exec sleep 60", file.display());
    screen
        .start(&Launch::new(31, 5, &["sh", "-c", &script]))
        .unwrap();
    let readback = screen
        .wait_for("marker-done", Duration::from_secs(10))
        .unwrap();
    let mut expected = Emulator::with_profile(Profile::xterm(), 31, 5);
    expected.feed(b"5 31\r\n").unwrap();
    expected.feed(bytes.as_bytes()).unwrap();
    let mismatches = compare(&expected, &readback);
    assert!(
        mismatches.is_empty(),
        "{mismatches:?}\n{:?}",
        readback.rows()
    );
}

#[test]
fn bytes_and_keys_reach_the_program() {
    let Some(mut screen) = screen() else { return };
    let dir = work_dir("screen-keys").unwrap();
    let file = dir.join("typed");
    let script = format!(
        "stty raw -echo; echo ready; exec dd bs=1 of='{}' 2>/dev/null",
        file.display()
    );
    screen
        .start(&Launch::new(20, 3, &["sh", "-c", &script]))
        .unwrap();
    screen.wait_for("ready", Duration::from_secs(10)).unwrap();
    screen.send_bytes(b"a^M\\b\x1b[A").unwrap();
    screen
        .send_keys(&[(Key::Up, Mods::NONE), (Key::Char('x'), Mods::NONE)])
        .unwrap();
    let expected = b"a^M\\b\x1b[A\x1b[Ax";
    let deadline = Instant::now() + Duration::from_secs(5);
    let got = loop {
        let got = std::fs::read(&file).unwrap_or_default();
        if got.len() >= expected.len() || Instant::now() >= deadline {
            break got;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        String::from_utf8_lossy(&got),
        String::from_utf8_lossy(expected)
    );
}
