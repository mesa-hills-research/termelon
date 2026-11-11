//! Each driver's parser on saved readbacks, so they're tested where the terminals can't run.
//!
//! The samples were recorded from tmux 3.6 and screen 4.9.1 running the probe's scenes.

use std::path::PathBuf;

use harness::drivers::{screen, tmux};
use harness::{Readback, compare};
use tuiframe_vt::{Color, Emulator, Profile, WidthRules};

#[expect(clippy::panic, reason = "a helper of tests, which fail on an error")]
fn sample(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/samples")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn expected(profile: Profile, cols: u16, rows: u16, bytes: &str) -> Emulator {
    let mut term = Emulator::with_profile(profile, cols, rows);
    term.feed(&sample(bytes)).unwrap();
    term
}

fn assert_same(expected: &Emulator, readback: &Readback) {
    let mismatches = compare(expected, readback);
    assert!(
        mismatches.is_empty(),
        "{}\nshown: {:#?}",
        mismatches
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        readback.rows()
    );
}

#[test]
fn tmux_capture_of_the_styles_scene() {
    let cursor = String::from_utf8(sample("tmux-3.6-styles.cursor")).unwrap();
    let readback = tmux::parse(
        &sample("tmux-3.6-styles.capture"),
        &cursor,
        80,
        24,
        WidthRules::default(),
    )
    .unwrap();
    assert_same(
        &expected(Profile::tmux_3_4(), 80, 24, "styles.bytes"),
        &readback,
    );
    // Overline came as tmux's 5:3, and the link and underline colour came through.
    let cell = |x: u16, y: u16| readback.screen.cell(x, y).unwrap().style().clone();
    assert!(cell(47, 0).overline);
    assert!(cell(0, 7).link.is_some());
    assert_eq!(cell(27, 1).underline_color, Color::Rgb(255, 0, 0));
}

#[test]
fn tmux_capture_of_the_text_scene() {
    let cursor = String::from_utf8(sample("tmux-3.6-text.cursor")).unwrap();
    let readback = tmux::parse(
        &sample("tmux-3.6-text.capture"),
        &cursor,
        80,
        24,
        WidthRules::default(),
    )
    .unwrap();
    assert_same(
        &expected(Profile::tmux_3_4(), 80, 24, "text.bytes"),
        &readback,
    );
}

#[test]
fn a_tmux_capture_with_a_bad_cursor_line_fails() {
    assert!(tmux::parse(b"", "x y", 80, 24, WidthRules::default()).is_err());
}

#[test]
fn screen_hardcopy_of_the_text_scene() {
    let readback = screen::parse(&sample("screen-4.9-text.hardcopy"), 80, 24).unwrap();
    assert_same(&expected(Profile::xterm(), 80, 24, "text.bytes"), &readback);
    // Latin-1 comes through, and a wide character leaves its low byte and 0xFF.
    assert!(readback.rows()[3].starts_with("Latin-1: café ñ ß"));
    assert!(readback.rows()[4].starts_with("CJK: Wÿ"));
}
