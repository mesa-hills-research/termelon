//! The quirks a profile can turn on, each against the profile without it.

use termelon_vt::{Color, Emulator, Event, Profile, Quirk};

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn rows(profile: Profile, cols: u16, rows: u16, bytes: &[u8]) -> Vec<String> {
    let mut term = Emulator::with_profile(profile, cols, rows);
    term.feed(bytes).unwrap();
    term.rows()
}

fn both(quirk: Quirk, cols: u16, height: u16, bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let base = Profile::kitty();
    (
        rows(base.clone(), cols, height, bytes),
        rows(base.with_quirk(quirk), cols, height, bytes),
    )
}

#[test]
fn ignoring_autowrap_off_keeps_wrapping() {
    let (plain, quirk) = both(Quirk::IgnoresAutowrapOff, 3, 2, b"\x1b[?7labcd");
    assert_eq!(plain, ["abd", ""]);
    assert_eq!(quirk, ["abc", "d"]);
    let mut term =
        Emulator::with_profile(Profile::kitty().with_quirk(Quirk::IgnoresAutowrapOff), 3, 2);
    term.feed(b"\x1b[?7l").unwrap();
    assert!(term.mode(7));
    assert_eq!(term.events(), []);
}

#[test]
fn keeping_wide_halves_leaves_the_other_half() {
    let (plain, quirk) = both(Quirk::KeepsWideHalves, 5, 1, "字\x1b[1;2Hx".as_bytes());
    assert_eq!(plain, [" x"]);
    assert_eq!(quirk, ["字x"]);
    let (plain, quirk) = both(Quirk::KeepsWideHalves, 5, 1, "字\x1b[1;1Hx".as_bytes());
    assert_eq!(plain, ["x"]);
    assert_eq!(quirk, ["x字"]);
}

#[test]
fn no_scroll_up_down() {
    let (plain, quirk) = both(Quirk::NoScrollUpDown, 2, 2, b"a\r\nb\x1b[S");
    assert_eq!(plain, ["b", ""]);
    assert_eq!(quirk, ["a", "b"]);
    let (_, quirk) = both(Quirk::NoScrollUpDown, 2, 2, b"a\r\nb\x1b[T");
    assert_eq!(quirk, ["a", "b"]);
}

#[test]
fn no_insert_delete_lines() {
    let (plain, quirk) = both(Quirk::NoInsertDeleteLines, 2, 2, b"a\r\nb\x1b[1;1H\x1b[L");
    assert_eq!(plain, ["", "a"]);
    assert_eq!(quirk, ["a", "b"]);
    let (_, quirk) = both(Quirk::NoInsertDeleteLines, 2, 2, b"a\r\nb\x1b[1;1H\x1b[M");
    assert_eq!(quirk, ["a", "b"]);
}

#[test]
fn no_sync() {
    let mut term = Emulator::with_profile(Profile::kitty().with_quirk(Quirk::NoSync), 3, 1);
    term.feed(b"\x1b[?2026h").unwrap();
    assert!(!term.mode(2026));
    assert!(!term.events().iter().any(|e| matches!(e, Event::SyncBegin)));
}

#[test]
fn ambiguous_wide() {
    let mut term = Emulator::with_profile(Profile::kitty().with_quirk(Quirk::AmbiguousWide), 5, 1);
    term.feed("é".as_bytes()).unwrap();
    assert_eq!(term.cell(0, 0).unwrap().width(), 2);
}

#[test]
fn no_bce() {
    let mut term = Emulator::with_profile(Profile::kitty().with_quirk(Quirk::NoBce), 3, 1);
    term.feed(b"\x1b[41m\x1b[2J").unwrap();
    assert_eq!(term.cell(0, 0).unwrap().style().bg, Color::Default);
}

#[test]
fn every_quirk_changes_kittys_profile() {
    for quirk in Quirk::ALL {
        assert_ne!(
            Profile::kitty().with_quirk(quirk),
            Profile::kitty(),
            "{quirk:?}"
        );
    }
}
