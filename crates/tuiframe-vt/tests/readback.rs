//! Reading a screen back as text.

use tuiframe_vt::{Emulator, Profile};

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn run(profile: Profile, cols: u16, rows: u16, bytes: &[u8]) -> Emulator {
    let mut term = Emulator::with_profile(profile, cols, rows);
    term.feed(bytes).unwrap();
    term
}

#[test]
fn text_leaves_out_continuation_cells_and_trailing_spaces() {
    let term = run(Profile::xterm(), 8, 2, "a字b  \r\n  x".as_bytes());
    assert_eq!(term.rows(), ["a字b", "  x"]);
    assert_eq!(term.text(), "a字b\n  x");
    assert_eq!(term.row_text(2), None);
}
