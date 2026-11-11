//! The tmux driver against the tmux on this machine. Each test passes without running when tmux
//! isn't installed.

use std::path::Path;
use std::time::{Duration, Instant};

use harness::driver::{Driver, Launch, installed, work_dir};
use harness::drivers::{self, tmux::Tmux};
use harness::run::smoke;
use harness::{Error, compare};
use termelon_vt::{Emulator, Key, Mods, Profile};

#[expect(
    clippy::expect_used,
    reason = "a helper of tests, which fail on an error"
)]
fn tmux() -> Option<Tmux> {
    installed("tmux").then(|| Tmux::new().expect("a work directory"))
}

/// Runs bytes in a tmux pane with `cat`, waits for the marker at their end, and compares the pane
/// with `termelon-vt`'s screen for them.
#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn shows_the_same(cols: u16, rows: u16, bytes: &str) {
    let Some(mut tmux) = tmux() else { return };
    let dir = work_dir("tmux-test").unwrap();
    let file = dir.join("bytes");
    let bytes = format!("{bytes}\x1b[0m\x1b[{rows};1Hmarker-done");
    std::fs::write(&file, &bytes).unwrap();
    let script = format!("cat '{}'; exec sleep 60", file.display());
    tmux.start(&Launch::new(cols, rows, &["sh", "-c", &script]))
        .unwrap();
    let readback = tmux
        .wait_for("marker-done", Duration::from_secs(10))
        .unwrap();
    let mut expected = Emulator::with_profile(Profile::tmux_3_4(), cols, rows);
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
    tmux.stop().unwrap();
}

#[test]
fn smoke_test() {
    let Some(mut tmux) = tmux() else { return };
    smoke(&mut tmux).unwrap();
    assert!(tmux.version().unwrap().starts_with("tmux "));
}

#[test]
fn text_and_wide_characters() {
    shows_the_same(
        30,
        6,
        "\x1b[1;1Hplain\x1b[2;3H字幕 é e\u{301} ⣷…\x1b[3;29Hab\x1b[4;1Hover\x1b[4;1HOV",
    );
}

#[test]
fn colours_and_attributes() {
    shows_the_same(
        40,
        6,
        "\x1b[1;1H\x1b[1mb\x1b[2md\x1b[0m\x1b[3mi\x1b[4mu\x1b[0m\x1b[7mr\x1b[9ms\x1b[5mk\x1b[53mo\x1b[8mh\x1b[0m\
         \x1b[2;1H\x1b[31ma\x1b[38;5;1mb\x1b[38;2;1;2;3mc\x1b[95md\x1b[44me\x1b[48;5;236mf\x1b[48;2;9;8;7mg\x1b[0m\
         \x1b[3;1H\x1b[4:3mcurly\x1b[4:2mdouble\x1b[58;5;196mcoloured\x1b[0m\
         \x1b[4;1H\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\",
    );
}

#[test]
fn erasing_and_scrolling() {
    shows_the_same(
        20,
        8,
        "\x1b[1;1Hfirst row here\x1b[1;6H\x1b[K\x1b[2;1H\x1b[44m\x1b[K\x1b[0mblue\
         \x1b[3;1Ha\x1b[4;1Hb\x1b[5;1Hc\x1b[3;5r\x1b[5;1H\x1bDd\x1b[r\x1b[7;1H\x1b[S",
    );
}

#[test]
fn the_cursor() {
    shows_the_same(20, 4, "\x1b[2;5Hx\x1b[3 q");
    shows_the_same(20, 4, "\x1b[2;5Hx\x1b[?25l");
}

#[test]
fn waiting_times_out_with_the_screen() {
    let Some(mut tmux) = tmux() else { return };
    tmux.start(&Launch::new(
        20,
        3,
        &["sh", "-c", "echo shown; exec sleep 60"],
    ))
    .unwrap();
    match tmux.wait_for("never", Duration::from_millis(300)) {
        Err(Error::Timeout { marker, screen }) => {
            assert_eq!(marker, "never");
            assert_eq!(screen[0], "shown");
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
}

/// Starts a pane whose program saves every byte it reads, raw, to a file.
#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
/// Returns the file, and its directory, which goes when dropped.
fn recorder(tmux: &mut Tmux) -> (std::path::PathBuf, harness::driver::WorkDir) {
    let dir = work_dir("tmux-keys").unwrap();
    let file = dir.join("typed");
    let script = format!(
        "stty raw -echo; echo ready; exec dd bs=1 of='{}' 2>/dev/null",
        file.display()
    );
    tmux.start(&Launch::new(20, 3, &["sh", "-c", &script]))
        .unwrap();
    tmux.wait_for("ready", Duration::from_secs(10)).unwrap();
    (file, dir)
}

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

#[test]
fn raw_bytes_reach_the_program() {
    let Some(mut tmux) = tmux() else { return };
    let (file, _dir) = recorder(&mut tmux);
    let bytes = b"a\x1b[A\x00\xff;";
    tmux.send_bytes(bytes).unwrap();
    assert_eq!(read_until(&file, bytes.len()), bytes);
}

/// tmux's keys are the ones `termelon-vt`'s tmux profile encodes, key by key.
#[test]
fn keys_match_the_tmux_profile() {
    let Some(mut tmux) = tmux() else { return };
    let (file, _dir) = recorder(&mut tmux);
    let version = tmux.version().unwrap();
    let profile = Emulator::with_profile(drivers::profile("tmux", &version).unwrap(), 20, 3);
    let none = Mods::NONE;
    let keys = [
        (Key::Char('a'), none),
        (Key::Char('a'), Mods::SHIFT),
        (Key::Char('a'), Mods::CTRL),
        (Key::Char('a'), Mods::ALT),
        (Key::Enter, none),
        (Key::Tab, none),
        (Key::Tab, Mods::SHIFT),
        (Key::Backspace, none),
        (Key::Escape, none),
        (Key::Up, none),
        (Key::Left, Mods::CTRL),
        (Key::Right, Mods::SHIFT),
        (Key::Down, Mods::ALT),
        (Key::Home, none),
        (Key::End, none),
        (Key::Home, Mods::CTRL),
        (Key::PageUp, none),
        (Key::PageDown, Mods::SHIFT),
        (Key::Insert, none),
        (Key::Delete, none),
        (Key::F(1), none),
        (Key::F(4), Mods::SHIFT),
        (Key::F(5), none),
        (Key::F(12), Mods::CTRL),
    ];
    let mut expected = Vec::new();
    for (key, mods) in keys {
        tmux.send_keys(&[(key, mods)]).unwrap();
        expected.extend(profile.encode_key(key, mods));
        // A separator after each key keeps the sequences apart in a failure.
        tmux.send_bytes(b"|").unwrap();
        expected.push(b'|');
    }
    let got = read_until(&file, expected.len());
    assert_eq!(
        String::from_utf8_lossy(&got),
        String::from_utf8_lossy(&expected)
    );
}
