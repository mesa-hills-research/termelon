//! Each driver's parser on saved readbacks, so they're tested where the terminals can't run.
//!
//! The tmux and GNU screen samples were recorded from tmux 3.6 and screen 4.9.1 running the probe's
//! scenes. The kitty, xterm and WezTerm samples are written by hand in the formats their docs
//! describe, for the small screen in `small.bytes`, until a run on CI records real ones.

use std::path::PathBuf;

use harness::drivers::{kitty, screen, tmux, wezterm, xterm};
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

#[test]
fn kitty_get_text() {
    let readback = kitty::parse(
        &sample("kitty-constructed-small.ansi"),
        20,
        4,
        Profile::kitty().width,
    )
    .unwrap();
    assert_same(&expected(Profile::kitty(), 20, 4, "small.bytes"), &readback);
    assert_eq!(
        (readback.screen.cursor().x, readback.screen.cursor().y),
        (2, 3)
    );
}

#[test]
fn wezterm_get_text_and_list() {
    let list = String::from_utf8(sample("wezterm-constructed-small.json")).unwrap();
    let readback = wezterm::parse(&sample("wezterm-constructed-small.ansi"), &list, 20, 4).unwrap();
    assert_same(&expected(Profile::xterm(), 20, 4, "small.bytes"), &readback);
}

#[test]
fn xterm_screen_dump() {
    let html = String::from_utf8(sample("xterm-constructed-small.xhtml")).unwrap();
    let readback = xterm::parse(&html, 20, 4).unwrap();
    assert_same(&expected(Profile::xterm(), 20, 4, "small.bytes"), &readback);
    assert_eq!(readback.rows()[1], " bg  字x");
}

#[test]
fn an_xterm_dump_shows_a_wrong_colour() {
    let html = String::from_utf8(sample("xterm-constructed-small.xhtml"))
        .unwrap()
        .replace("#cd0000", "#00cd00");
    let readback = xterm::parse(&html, 20, 4).unwrap();
    let mismatches = compare(&expected(Profile::xterm(), 20, 4, "small.bytes"), &readback);
    assert_eq!(mismatches.len(), 3);
    assert!(
        mismatches
            .iter()
            .all(|m| m.what == "fg" && m.expected == "#cd0000" && m.got == "#00cd00")
    );
}

fn args(command: &std::process::Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn launch() -> harness::Launch {
    let mut launch = harness::Launch::new(100, 30, &["probe", "--scene", "text"]);
    launch.cwd = PathBuf::from("/home/ada/proj");
    launch
}

#[test]
fn kitty_starts_at_the_size_with_remote_control() {
    let command = kitty::start_command(&launch(), "unix:/tmp/k.sock");
    let args = args(&command);
    for wanted in [
        "--listen-on",
        "unix:/tmp/k.sock",
        "initial_window_width=100c",
        "initial_window_height=30c",
    ] {
        assert!(args.iter().any(|arg| arg == wanted), "{wanted} in {args:?}");
    }
    assert!(args.iter().any(|arg| arg == "allow_remote_control=yes"));
    assert_eq!(args[args.len() - 3..], ["probe", "--scene", "text"]);
}

#[test]
fn xterm_starts_at_the_size_with_a_bitmap_font_and_records_its_tty() {
    let command = xterm::start_command(
        &launch(),
        std::path::Path::new("/tmp/x"),
        std::path::Path::new("/tmp/x/tty"),
    );
    let args = args(&command);
    assert_eq!(args[..4], ["-geometry", "100x30", "-fn", "9x15"]);
    let e = args.iter().position(|arg| arg == "-e").unwrap();
    assert_eq!(args[e + 1..e + 3], ["sh", "-c"]);
    assert_eq!(
        args[args.len() - 5..],
        ["/tmp/x/tty", "/home/ada/proj", "probe", "--scene", "text"]
    );
}

#[test]
fn wezterm_starts_at_the_size_in_a_new_process() {
    let command = wezterm::start_command(&launch(), std::path::Path::new("/tmp/w"));
    let args = args(&command);
    for wanted in [
        "initial_cols=100",
        "initial_rows=30",
        "check_for_updates=false",
        "front_end=\"Software\"",
    ] {
        assert!(args.iter().any(|arg| arg == wanted), "{wanted} in {args:?}");
    }
    let start = args.iter().position(|arg| arg == "start").unwrap();
    assert_eq!(args[start + 1], "--always-new-process");
    assert_eq!(args[args.len() - 4..], ["--", "probe", "--scene", "text"]);
}
