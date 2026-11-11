//! Each driver's parser on saved readbacks, so they're tested where the terminals can't run.
//!
//! The tmux, GNU screen, xterm.js and VTE samples were recorded from tmux 3.6, screen 4.9.1,
//! `@xterm/headless` 6.0.0 and VTE 0.84 running the probe's scenes, and the Alacritty samples are
//! `alacritty_terminal` 0.26's serialization of its grid for those scenes, which is what
//! `--ref-test` writes. The kitty, xterm, WezTerm, Konsole, foot and Ghostty samples are written by
//! hand in the formats their docs describe, for the small screen in `small.bytes`, until a run on
//! CI records real ones.

use std::path::{Path, PathBuf};

use harness::drivers::{
    alacritty, foot, ghostty, kitty, konsole, screen, tmux, vte, wezterm, xterm, xtermjs,
};
use harness::json::Json;
use harness::{Readback, compare};
use termelon_vt::{Color, Emulator, Profile, WidthRules};
use termelon_vt::{Key, Mods};

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
    let command = xterm::start_command(&launch(), Path::new("/tmp/x"), Path::new("/tmp/x/tty"));
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
    let command = wezterm::start_command(&launch(), Path::new("/tmp/w"));
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

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn json(name: &str) -> Json {
    Json::parse(&String::from_utf8(sample(name)).unwrap()).unwrap()
}

#[test]
fn xtermjs_screens() {
    for scene in ["styles", "text"] {
        let readback =
            xtermjs::parse(&json(&format!("xtermjs-6.0.0-{scene}.json")), 80, 24).unwrap();
        let profile = Profile {
            hyperlinks: true,
            ..Profile::xterm()
        };
        assert_same(
            &expected(profile, 80, 24, &format!("{scene}.bytes")),
            &readback,
        );
    }
}

#[test]
fn vte_screens() {
    for scene in ["styles", "text"] {
        let readback = vte::parse(&json(&format!("vte-0.84-{scene}.json")), 80, 24).unwrap();
        assert_same(
            &expected(Profile::xterm(), 80, 24, &format!("{scene}.bytes")),
            &readback,
        );
    }
}

#[test]
fn alacritty_grids() {
    for scene in ["styles", "text"] {
        let text = String::from_utf8(sample(&format!(
            "alacritty-terminal-0.26-{scene}.grid.json"
        )))
        .unwrap();
        let readback = alacritty::parse(&text).unwrap();
        let profile = Profile {
            hyperlinks: true,
            ..Profile::xterm()
        };
        assert_same(
            &expected(profile, 80, 24, &format!("{scene}.bytes")),
            &readback,
        );
    }
}

#[test]
fn konsole_displayed_text() {
    let result = String::from_utf8(sample("konsole-constructed-small.gdbus")).unwrap();
    let text = konsole::gvariant_string(&result).unwrap();
    let readback = konsole::parse(&text, 20, 4).unwrap();
    assert_same(
        &expected(Profile::konsole(), 20, 4, "small.bytes"),
        &readback,
    );
}

#[test]
fn gvariant_strings() {
    assert_eq!(
        konsole::gvariant_string("('a\\'b\\\\c\\u00e9\\U0001f600\\n',)").unwrap(),
        "a'b\\cé😀\n"
    );
    assert_eq!(konsole::gvariant_string("(\"it's\",)").unwrap(), "it's");
    assert!(konsole::gvariant_string("()").is_err());
    assert_eq!(konsole::gvariant_quote("a'b\\\u{1b}"), "'a\\'b\\\\\\u001b'");
}

#[test]
fn foot_piped_text() {
    let text = String::from_utf8(sample("foot-constructed-small.txt")).unwrap();
    let readback = foot::parse(&text, 20, 4).unwrap();
    assert_same(&expected(Profile::xterm(), 20, 4, "small.bytes"), &readback);
}

#[test]
fn foot_keys_and_binding() {
    assert_eq!(
        foot::wtype_args(Key::Char('a'), Mods::CTRL | Mods::SUPER).unwrap(),
        [
            "-M", "ctrl", "-M", "logo", "-k", "a", "-m", "logo", "-m", "ctrl"
        ]
    );
    assert_eq!(foot::wtype_args(Key::Up, Mods::NONE).unwrap(), ["-k", "Up"]);
    assert_eq!(
        foot::wtype_args(Key::Char('a'), Mods::SHIFT).unwrap(),
        ["-M", "shift", "-k", "A", "-m", "shift"]
    );
    assert_eq!(
        foot::wtype_args(Key::Tab, Mods::SHIFT).unwrap(),
        ["-M", "shift", "-k", "ISO_Left_Tab", "-m", "shift"]
    );
    let config = foot::config(Path::new("/tmp/f/screen.txt"));
    assert!(
        config.contains("pipe-visible=[sh -c \"cat > '/tmp/f/screen.txt'\"] Control+Shift+F12")
    );
}

#[test]
fn ghostty_screen_file() {
    let text = String::from_utf8(sample("ghostty-constructed-small.txt")).unwrap();
    let readback = ghostty::parse(&text, 20, 4).unwrap();
    assert_same(&expected(Profile::kitty(), 20, 4, "small.bytes"), &readback);
}

#[test]
fn ghostty_keystrokes() {
    assert_eq!(
        ghostty::keystroke_script(Key::Char('a'), Mods::CTRL | Mods::SHIFT).unwrap(),
        "tell application \"System Events\" to keystroke \"a\" using {control down, shift down}"
    );
    assert_eq!(
        ghostty::keystroke_script(Key::F(12), Mods::NONE).unwrap(),
        "tell application \"System Events\" to key code 111"
    );
    assert_eq!(ghostty::keystroke_script(Key::Insert, Mods::NONE), None);
}

#[test]
fn alacritty_starts_with_its_ref_test() {
    assert_eq!(
        alacritty::config(100, 30),
        "[window]\ndimensions = { columns = 100, lines = 30 }\n\n[scrolling]\nhistory = 0\n"
    );
    let command = alacritty::start_command(&launch(), Path::new("/tmp/a"), Path::new("/tmp/a/pid"));
    let args = args(&command);
    let config = Path::new("/tmp/a")
        .join("alacritty.toml")
        .display()
        .to_string();
    assert_eq!(args[..3], ["--ref-test", "--config-file", config.as_str()]);
    assert_eq!(
        args[args.len() - 5..],
        ["/tmp/a/pid", "/home/ada/proj", "probe", "--scene", "text"]
    );
}

#[test]
fn konsole_starts_at_the_size() {
    let command = konsole::start_command(&launch(), Path::new("/tmp/k"));
    let args = args(&command);
    for wanted in [
        "--separate",
        "--nofork",
        "TerminalColumns=100",
        "TerminalRows=30",
    ] {
        assert!(args.iter().any(|arg| arg == wanted), "{wanted} in {args:?}");
    }
    assert_eq!(args[args.len() - 4..], ["-e", "probe", "--scene", "text"]);
}

#[test]
fn ghostty_starts_at_the_size_with_its_binding() {
    let command = ghostty::start_command(Path::new(ghostty::APP_BINARY), &launch());
    let args = args(&command);
    for wanted in [
        "--window-width=100",
        "--window-height=30",
        "--keybind=ctrl+shift+f12=write_screen_file:copy",
    ] {
        assert!(args.iter().any(|arg| arg == wanted), "{wanted} in {args:?}");
    }
    assert_eq!(args[args.len() - 4..], ["-e", "probe", "--scene", "text"]);
}
