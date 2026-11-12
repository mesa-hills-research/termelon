//! One test for each sequence the emulator knows, with the screen it should leave.

use termelon_vt::{Color, CursorShape, Emulator, Event, Pos, Profile, Underline, WidthRules};

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn run(cols: u16, rows: u16, bytes: &[u8]) -> Emulator {
    let mut term = Emulator::new(cols, rows);
    term.feed(bytes).unwrap();
    term
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn run_as(profile: Profile, cols: u16, rows: u16, bytes: &[u8]) -> Emulator {
    let mut term = Emulator::with_profile(profile, cols, rows);
    term.feed(bytes).unwrap();
    term
}

fn at(term: &Emulator) -> (u16, u16) {
    let cursor = term.cursor();
    (cursor.x, cursor.y)
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn unknown(bytes: &[u8]) -> Vec<String> {
    let mut term = Emulator::new(10, 3);
    let error = term.feed(bytes).unwrap_err();
    error.sequences().iter().map(ToString::to_string).collect()
}

// Text and controls.

#[test]
fn text_and_crlf() {
    let term = run(10, 3, b"ab\r\ncd");
    assert_eq!(term.rows(), ["ab", "cd", ""]);
    assert_eq!(at(&term), (2, 1));
}

#[test]
fn backspace_moves_left_and_stops_at_the_edge() {
    assert_eq!(run(10, 1, b"abc\x08\x08X").rows(), ["aXc"]);
    assert_eq!(run(10, 1, b"\x08X").rows(), ["X"]);
}

#[test]
fn tab_goes_to_the_next_stop_and_stops_at_the_edge() {
    assert_eq!(run(20, 1, b"a\tb\tc").rows(), ["a       b       c"]);
    assert_eq!(run(10, 1, b"\t\t\tX").rows(), ["         X"]);
}

#[test]
fn lf_vt_and_ff_move_down_without_a_carriage_return() {
    assert_eq!(
        run(10, 4, b"a\nb\x0bc\x0cd").rows(),
        ["a", " b", "  c", "   d"]
    );
}

#[test]
fn line_feed_scrolls_at_the_bottom() {
    assert_eq!(run(5, 2, b"a\r\nb\r\nc").rows(), ["b", "c"]);
}

#[test]
fn bell_is_an_event() {
    let term = run(5, 1, b"\x07");
    assert_eq!(term.events(), [Event::Bell]);
}

#[test]
fn nul_can_and_sub_do_nothing_in_text() {
    assert_eq!(run(5, 1, b"a\0\x18\x1ab").rows(), ["ab"]);
}

#[test]
fn an_unused_control_is_unknown() {
    assert_eq!(unknown(b"\x0e"), ["`\\x0e`"]);
}

// Wrapping.

#[test]
fn the_last_column_leaves_a_wrap_pending() {
    let mut term = Emulator::new(3, 2);
    term.feed(b"abc").unwrap();
    assert_eq!(at(&term), (2, 0));
    assert!(term.cursor().pending_wrap);
    term.feed(b"d").unwrap();
    assert_eq!(term.rows(), ["abc", "d"]);
    assert_eq!(at(&term), (1, 1));
}

#[test]
fn carriage_return_cancels_a_pending_wrap() {
    assert_eq!(run(3, 2, b"abc\rX").rows(), ["Xbc", ""]);
}

#[test]
fn line_feed_cancels_a_pending_wrap() {
    assert_eq!(run(3, 3, b"abc\nX").rows(), ["abc", "  X", ""]);
}

#[test]
fn a_cursor_move_cancels_a_pending_wrap() {
    assert_eq!(run(3, 2, b"abc\x1b[DX").rows(), ["aXc", ""]);
}

#[test]
fn without_autowrap_the_last_column_is_overwritten() {
    let term = run(3, 2, b"\x1b[?7labcde");
    assert_eq!(term.rows(), ["abe", ""]);
    assert!(term.cursor().pending_wrap);
    assert!(!term.mode(7));
}

#[test]
fn turning_autowrap_on_after_the_last_column_wraps_the_next_character() {
    assert_eq!(run(3, 2, b"\x1b[?7labc\x1b[?7hX").rows(), ["abc", "X"]);
}

#[test]
fn autowrap_set_again_wraps() {
    assert_eq!(run(3, 2, b"\x1b[?7l\x1b[?7habcd").rows(), ["abc", "d"]);
}

#[test]
fn wrapping_at_the_bottom_scrolls() {
    assert_eq!(run(2, 2, b"abcde").rows(), ["cd", "e"]);
}

// Wide characters and clusters.

#[test]
fn a_wide_character_takes_two_cells() {
    let term = run(5, 1, "a字b".as_bytes());
    assert_eq!(term.rows(), ["a字b"]);
    assert_eq!(term.cell(1, 0).unwrap().width(), 2);
    assert!(term.cell(2, 0).unwrap().is_continuation());
    assert_eq!(term.cell(2, 0).unwrap().text(), "");
    assert_eq!(at(&term), (4, 0));
}

#[test]
fn a_wide_character_that_doesnt_fit_wraps() {
    assert_eq!(run(3, 2, "ab字".as_bytes()).rows(), ["ab", "字"]);
}

#[test]
fn a_wide_character_that_wraps_leaves_a_blank_in_the_pen() {
    let term = run(3, 2, "abc\x1b[1;3H\x1b[41m字".as_bytes());
    assert_eq!(term.rows(), ["ab", "字"]);
    assert_eq!(term.cell(2, 0).unwrap().style().bg, Color::Red);
}

#[test]
fn a_wide_character_that_doesnt_fit_without_autowrap_moves_left() {
    assert_eq!(run(3, 1, "\x1b[?7lab字".as_bytes()).rows(), ["a字"]);
}

#[test]
fn writing_over_half_a_wide_character_blanks_the_other_half() {
    assert_eq!(run(5, 1, "字\x1b[1;2HX".as_bytes()).rows(), [" X"]);
    assert_eq!(run(5, 1, "字\x1b[1;1HX".as_bytes()).rows(), ["X"]);
}

#[test]
fn a_combining_mark_joins_the_character_before() {
    let term = run(5, 1, "e\u{301}x".as_bytes());
    assert_eq!(term.cell(0, 0).unwrap().text(), "e\u{301}");
    assert_eq!(term.rows(), ["e\u{301}x"]);
}

#[test]
fn a_combining_mark_after_a_move_joins_the_cell_before_the_cursor() {
    let term = run(5, 1, "ab\x1b[1;2H\u{301}".as_bytes());
    assert_eq!(term.cell(0, 0).unwrap().text(), "a\u{301}");
    assert_eq!(at(&term), (1, 0));
}

#[test]
fn a_combining_mark_at_the_left_edge_is_dropped() {
    assert_eq!(run(5, 1, "\u{301}a".as_bytes()).rows(), ["a"]);
}

const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
const HEART: &str = "\u{2764}\u{fe0f}";

#[test]
fn an_emoji_sequence_adds_up_where_the_terminal_doesnt_join() {
    let term = run(10, 1, FAMILY.as_bytes());
    assert_eq!(term.cell(0, 0).unwrap().text(), FAMILY);
    assert_eq!(term.cell(0, 0).unwrap().width(), 6);
    assert_eq!(at(&term), (6, 0));
}

#[test]
fn an_emoji_sequence_is_two_cells_where_the_terminal_joins() {
    let term = run_as(Profile::kitty(), 10, 1, FAMILY.as_bytes());
    assert_eq!(term.cell(0, 0).unwrap().width(), 2);
    assert_eq!(at(&term), (2, 0));
}

#[test]
fn vs16_widens_by_the_profile() {
    let xterm = run(5, 1, HEART.as_bytes());
    assert_eq!((xterm.cell(0, 0).unwrap().width(), at(&xterm)), (1, (1, 0)));
    let kitty = run_as(Profile::kitty(), 5, 1, HEART.as_bytes());
    assert_eq!((kitty.cell(0, 0).unwrap().width(), at(&kitty)), (2, (2, 0)));
}

#[test]
fn a_flag_is_two_cells() {
    let term = run(5, 1, "\u{1f1fa}\u{1f1f8}x".as_bytes());
    assert_eq!(term.cell(0, 0).unwrap().text(), "\u{1f1fa}\u{1f1f8}");
    assert_eq!(term.rows(), ["\u{1f1fa}\u{1f1f8}x"]);
}

#[test]
fn ambiguous_characters_take_two_cells_where_the_profile_says() {
    let wide = Profile {
        width: WidthRules {
            ambiguous_wide: true,
            ..WidthRules::default()
        },
        ..Profile::xterm()
    };
    let term = run_as(wide, 5, 1, "éa".as_bytes());
    assert_eq!(term.cell(0, 0).unwrap().width(), 2);
    assert_eq!(term.rows(), ["éa"]);
    assert_eq!(at(&term), (3, 0));
    assert_eq!(at(&run(5, 1, "éa".as_bytes())), (2, 0));
}

#[test]
fn halfwidth_sound_marks_take_a_cell() {
    let term = run(5, 1, "\u{ff76}\u{ff9e}a".as_bytes());
    assert_eq!(term.rows(), ["\u{ff76}\u{ff9e}a"]);
    assert_eq!(at(&term), (3, 0));
}

#[test]
fn mode_2027_joins_clusters() {
    let mut term = Emulator::with_profile(Profile::windows_terminal(), 10, 2);
    term.feed(FAMILY.as_bytes()).unwrap();
    assert_eq!(term.cell(0, 0).unwrap().width(), 6);
    term.feed(b"\r\n\x1b[?2027h").unwrap();
    term.feed(FAMILY.as_bytes()).unwrap();
    assert!(term.mode(2027));
    assert_eq!(term.cell(0, 1).unwrap().width(), 2);
}

#[test]
fn a_cluster_that_grows_past_the_edge_moves_to_the_next_row() {
    let term = run(6, 2, format!("ab{FAMILY}").as_bytes());
    assert_eq!(term.rows(), ["ab", FAMILY]);
    assert_eq!(term.cell(0, 1).unwrap().width(), 6);
    assert!(term.cursor().pending_wrap);
}

// Cursor moves.

#[test]
fn cup_and_hvp() {
    let term = run(10, 5, b"\x1b[2;3HA\x1b[4;5fB\x1b[HC");
    assert_eq!(term.rows(), ["C", "  A", "", "    B", ""]);
}

#[test]
fn cup_clamps_to_the_screen() {
    let term = run(5, 3, b"\x1b[99;99HA");
    assert_eq!(term.rows(), ["", "", "    A"]);
}

#[test]
fn cuu_cud_cuf_cub() {
    let start = b"\x1b[3;3H";
    let moved = |seq: &[u8]| at(&run(10, 5, &[start, seq].concat()));
    assert_eq!(moved(b"\x1b[A"), (2, 1));
    assert_eq!(moved(b"\x1b[0A"), (2, 1));
    assert_eq!(moved(b"\x1b[9A"), (2, 0));
    assert_eq!(moved(b"\x1b[2B"), (2, 4));
    assert_eq!(moved(b"\x1b[9B"), (2, 4));
    assert_eq!(moved(b"\x1b[3C"), (5, 2));
    assert_eq!(moved(b"\x1b[99C"), (9, 2));
    assert_eq!(moved(b"\x1b[2D"), (0, 2));
    assert_eq!(moved(b"\x1b[9D"), (0, 2));
}

#[test]
fn cuu_and_cud_stop_at_the_margins() {
    let moved = |seq: &[u8]| at(&run(5, 6, &[b"\x1b[2;4r".as_slice(), seq].concat()));
    assert_eq!(moved(b"\x1b[3;1H\x1b[9A"), (0, 1));
    assert_eq!(moved(b"\x1b[3;1H\x1b[9B"), (0, 3));
    assert_eq!(moved(b"\x1b[1;1H\x1b[9A"), (0, 0));
    assert_eq!(moved(b"\x1b[6;1H\x1b[9B"), (0, 5));
}

#[test]
fn cnl_and_cpl() {
    assert_eq!(at(&run(10, 5, b"\x1b[3;3H\x1b[E")), (0, 3));
    assert_eq!(at(&run(10, 5, b"\x1b[3;3H\x1b[2F")), (0, 0));
}

#[test]
fn cha_and_hpa() {
    assert_eq!(at(&run(10, 2, b"\x1b[2;1H\x1b[5G")), (4, 1));
    assert_eq!(at(&run(10, 2, b"\x1b[3`")), (2, 0));
    assert_eq!(at(&run(10, 2, b"\x1b[99G")), (9, 0));
}

#[test]
fn vpa() {
    assert_eq!(at(&run(10, 5, b"\x1b[1;4H\x1b[3d")), (3, 2));
}

#[test]
fn ind_nel_and_ri() {
    assert_eq!(run(5, 3, b"ab\x1bDc").rows(), ["ab", "  c", ""]);
    assert_eq!(run(5, 3, b"ab\x1bEc").rows(), ["ab", "c", ""]);
    assert_eq!(run(5, 2, b"a\x1bMb").rows(), [" b", "a"]);
    assert_eq!(run(5, 2, b"\r\na\x1bMb").rows(), [" b", "a"]);
}

// Erasing.

fn filled() -> Emulator {
    run(3, 3, b"abcdefghi")
}

#[test]
fn ed_erases_below_above_and_all() {
    let erase = |seq: &[u8]| {
        let mut term = filled();
        term.feed(seq).unwrap();
        term.rows()
    };
    assert_eq!(erase(b"\x1b[2;2H\x1b[J"), ["abc", "d", ""]);
    assert_eq!(erase(b"\x1b[2;2H\x1b[0J"), ["abc", "d", ""]);
    assert_eq!(erase(b"\x1b[2;2H\x1b[1J"), ["", "  f", "ghi"]);
    assert_eq!(erase(b"\x1b[2;2H\x1b[2J"), ["", "", ""]);
    assert_eq!(erase(b"\x1b[3J"), ["abc", "def", "ghi"]);
}

#[test]
fn ed_leaves_the_cursor() {
    let mut term = filled();
    term.feed(b"\x1b[2;2H\x1b[2J").unwrap();
    assert_eq!(at(&term), (1, 1));
}

#[test]
fn el_erases_right_left_and_all() {
    let erase = |seq: &[u8]| run(6, 1, &[b"abcdef\x1b[1;3H".as_slice(), seq].concat()).rows();
    assert_eq!(erase(b"\x1b[K"), ["ab"]);
    assert_eq!(erase(b"\x1b[1K"), ["   def"]);
    assert_eq!(erase(b"\x1b[2K"), [""]);
}

#[test]
fn el_at_a_pending_wrap_erases_the_last_column() {
    let term = run(3, 2, b"abc\x1b[KX");
    assert_eq!(term.rows(), ["abX", ""]);
}

#[test]
fn erasing_uses_the_background_with_bce() {
    let term = run(3, 1, b"\x1b[44m\x1b[2J");
    assert_eq!(term.cell(1, 0).unwrap().style().bg, Color::Blue);
    let without = Profile {
        bce: false,
        ..Profile::xterm()
    };
    let term = run_as(without, 3, 1, b"\x1b[44m\x1b[2J");
    assert_eq!(term.cell(1, 0).unwrap().style().bg, Color::Default);
}

#[test]
fn bce_fills_only_the_background() {
    let term = run(3, 1, b"\x1b[1;4;31;44m\x1b[K");
    let style = term.cell(0, 0).unwrap().style();
    assert_eq!(
        (style.bg, style.fg, style.bold, style.underline),
        (Color::Blue, Color::Default, false, Underline::None)
    );
}

#[test]
fn an_unknown_erase_is_unknown() {
    assert_eq!(unknown(b"\x1b[5J"), ["`\\e[5J`"]);
    assert_eq!(unknown(b"\x1b[3K"), ["`\\e[3K`"]);
}

// Scrolling.

fn lettered(rows: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    for i in 0..rows {
        if i > 0 {
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.push(b'a' + i);
    }
    bytes
}

#[test]
fn su_and_sd() {
    let scroll = |seq: &[u8]| run(1, 3, &[lettered(3).as_slice(), seq].concat()).rows();
    assert_eq!(scroll(b"\x1b[S"), ["b", "c", ""]);
    assert_eq!(scroll(b"\x1b[2S"), ["c", "", ""]);
    assert_eq!(scroll(b"\x1b[T"), ["", "a", "b"]);
    assert_eq!(scroll(b"\x1b[9T"), ["", "", ""]);
}

#[test]
fn scrolling_leaves_the_cursor() {
    assert_eq!(at(&run(1, 3, b"\x1b[2;1H\x1b[S")), (0, 1));
}

#[test]
fn il_and_dl_inside_the_region() {
    let lines = |seq: &[u8]| run(2, 4, &[lettered(4).as_slice(), seq].concat());
    assert_eq!(lines(b"\x1b[2;2H\x1b[L").rows(), ["a", "", "b", "c"]);
    assert_eq!(lines(b"\x1b[2;2H\x1b[2L").rows(), ["a", "", "", "b"]);
    assert_eq!(lines(b"\x1b[2;2H\x1b[M").rows(), ["a", "c", "d", ""]);
    assert_eq!(lines(b"\x1b[2;2H\x1b[9M").rows(), ["a", "", "", ""]);
    assert_eq!(
        lines(b"\x1b[2;3r\x1b[2;1H\x1b[L").rows(),
        ["a", "", "b", "d"]
    );
    assert_eq!(
        lines(b"\x1b[2;3r\x1b[2;1H\x1b[M").rows(),
        ["a", "c", "", "d"]
    );
    // Outside the region they do nothing.
    assert_eq!(
        lines(b"\x1b[2;3r\x1b[4;1H\x1b[L").rows(),
        ["a", "b", "c", "d"]
    );
}

#[test]
fn il_and_dl_move_the_cursor_to_the_first_column() {
    assert_eq!(at(&run(5, 3, b"\x1b[2;4H\x1b[L")), (0, 1));
    assert_eq!(at(&run(5, 3, b"\x1b[2;4H\x1b[M")), (0, 1));
}

#[test]
fn inserted_lines_take_the_background_with_bce() {
    let term = run(2, 2, b"\x1b[42m\x1b[L");
    assert_eq!(term.cell(0, 0).unwrap().style().bg, Color::Green);
}

#[test]
fn decstbm_scrolls_only_the_region() {
    let region =
        |seq: &[u8]| run(1, 4, &[lettered(4).as_slice(), b"\x1b[2;3r", seq].concat()).rows();
    assert_eq!(region(b"\x1b[3;1H\n"), ["a", "c", "", "d"]);
    assert_eq!(region(b"\x1b[2;1H\x1bM"), ["a", "", "b", "d"]);
    assert_eq!(region(b"\x1b[S"), ["a", "c", "", "d"]);
    assert_eq!(region(b"\x1b[T"), ["a", "", "b", "d"]);
    assert_eq!(region(b"\x1b[4;1H\nX"), ["a", "b", "c", "X"]);
}

#[test]
fn decstbm_homes_the_cursor_and_ignores_a_bad_region() {
    let term = run(5, 4, b"\x1b[3;3H\x1b[2;3r");
    assert_eq!(at(&term), (0, 0));
    assert_eq!(term.scroll_region(), (1, 2));
    let term = run(5, 4, b"\x1b[3;2r");
    assert_eq!(term.scroll_region(), (0, 3));
    let term = run(5, 4, b"\x1b[2;3r\x1b[r");
    assert_eq!(term.scroll_region(), (0, 3));
}

#[test]
fn scrolled_in_rows_take_the_background_with_bce() {
    let term = run(2, 2, b"\x1b[41m\x1b[S");
    assert_eq!(term.cell(0, 1).unwrap().style().bg, Color::Red);
}

#[test]
fn origin_mode_positions_inside_the_region() {
    let term = run(5, 5, b"\x1b[2;4r\x1b[?6h");
    assert!(term.mode(6));
    assert_eq!(at(&term), (0, 1));
    assert_eq!(
        run(5, 5, b"\x1b[2;4r\x1b[?6h\x1b[1;1HX").rows(),
        ["", "X", "", "", ""]
    );
    assert_eq!(
        run(5, 5, b"\x1b[2;4r\x1b[?6h\x1b[9;1HX").rows(),
        ["", "", "", "X", ""]
    );
    assert_eq!(
        at(&run(5, 5, b"\x1b[2;4r\x1b[?6h\x1b[3;3H\x1b[?6l")),
        (0, 0)
    );
}

// Saving the cursor.

#[test]
fn decsc_and_decrc_save_the_place_and_the_pen() {
    let term = run(5, 3, b"\x1b[2;3H\x1b[1m\x1b7\x1b[H\x1b[0m\x1b8X");
    assert_eq!(term.rows(), ["", "  X", ""]);
    assert!(term.cell(2, 1).unwrap().style().bold);
}

#[test]
fn scosc_and_scorc() {
    assert_eq!(
        run(5, 3, b"\x1b[2;3H\x1b[s\x1b[H\x1b[uX").rows(),
        ["", "  X", ""]
    );
}

#[test]
fn decrc_with_nothing_saved_goes_home() {
    let term = run(5, 3, b"\x1b[2;2H\x1b[1m\x1b8X");
    assert_eq!(term.rows(), ["X", "", ""]);
    assert!(!term.cell(0, 0).unwrap().style().bold);
}

// Resets and character sets.

#[test]
fn ris_resets_the_screen_and_modes() {
    let term = run(5, 2, b"\x1b]2;kept\x07\x1b[1mab\x1b[?25l\x1b[?1049h\x1bc");
    assert_eq!(term.rows(), ["", ""]);
    assert_eq!(at(&term), (0, 0));
    assert!(term.cursor().visible);
    assert!(term.pen().is_default());
    assert!(!term.alternate_screen());
    assert_eq!(term.title(), "kept");
}

#[test]
fn keypad_modes() {
    assert!(run(5, 1, b"\x1b=").application_keypad());
    assert!(!run(5, 1, b"\x1b=\x1b>").application_keypad());
}

#[test]
fn line_drawing_charset() {
    assert_eq!(run(5, 1, b"\x1b(0lqk\x1b(Bq").rows(), ["┌─┐q"]);
}

#[test]
fn a_lone_string_terminator_does_nothing() {
    assert_eq!(run(5, 1, b"a\x1b\\b").rows(), ["ab"]);
}

#[test]
fn an_unknown_escape_is_unknown() {
    assert_eq!(unknown(b"\x1bZ"), ["`\\eZ`"]);
    assert_eq!(unknown(b"\x1b#8"), ["`\\e#8`"]);
}

// SGR.

fn pen(bytes: &[u8]) -> termelon_vt::Style {
    run(5, 1, bytes).pen().clone()
}

#[test]
fn sgr_attributes_and_their_resets() {
    let style = pen(b"\x1b[1;2;3;4;5;6;7;8;9;53m");
    assert!(style.bold && style.dim && style.italic && style.blink && style.rapid_blink);
    assert!(style.reverse && style.hidden && style.strike && style.overline);
    assert_eq!(style.underline, Underline::Single);
    let cleared = pen(b"\x1b[1;2;3;4;5;6;7;8;9;53m\x1b[22;23;24;25;27;28;29;55m");
    assert!(cleared.is_default());
}

/// kitty takes blink, hidden and overline and draws none of them.
#[test]
fn sgr_a_profile_ignores() {
    let term = run_as(Profile::kitty(), 5, 1, b"\x1b[1;5;8;53mA\x1b[6mB");
    let style = term.pen();
    assert!(style.bold);
    assert!(!style.blink && !style.rapid_blink && !style.hidden && !style.overline);
}

/// WezTerm keeps one intensity, so dim after bold replaces it.
#[test]
fn sgr_bold_and_dim_as_one_setting() {
    let one = Profile {
        one_intensity: true,
        ..Profile::xterm()
    };
    let style = run_as(one.clone(), 5, 1, b"\x1b[1;2mA").pen().clone();
    assert!(style.dim && !style.bold);
    let style = run_as(one, 5, 1, b"\x1b[2m\x1b[1mA").pen().clone();
    assert!(style.bold && !style.dim);
}

#[test]
fn sgr_22_clears_bold_and_dim_together() {
    let style = pen(b"\x1b[1;2m\x1b[22m");
    assert!(!style.bold && !style.dim);
}

#[test]
fn sgr_0_and_empty_reset() {
    assert!(pen(b"\x1b[1;31m\x1b[0m").is_default());
    assert!(pen(b"\x1b[1;31m\x1b[m").is_default());
    assert!(pen(b"\x1b[1;31m\x1b[;m").is_default());
}

#[test]
fn sgr_underline_styles() {
    let styles = [
        Underline::None,
        Underline::Single,
        Underline::Double,
        Underline::Curly,
        Underline::Dotted,
        Underline::Dashed,
    ];
    for (i, expected) in styles.iter().enumerate() {
        assert_eq!(
            pen(format!("\x1b[4m\x1b[4:{i}m").as_bytes()).underline,
            *expected
        );
    }
    assert_eq!(pen(b"\x1b[21m").underline, Underline::Double);
    assert_eq!(pen(b"\x1b[4:3m\x1b[24m").underline, Underline::None);
}

#[test]
fn sgr_named_colours() {
    assert_eq!(pen(b"\x1b[31m").fg, Color::Red);
    assert_eq!(pen(b"\x1b[37m").fg, Color::Gray);
    assert_eq!(pen(b"\x1b[90m").fg, Color::DarkGray);
    assert_eq!(pen(b"\x1b[97m").fg, Color::White);
    assert_eq!(pen(b"\x1b[40m").bg, Color::Black);
    assert_eq!(pen(b"\x1b[104m").bg, Color::LightBlue);
    assert_eq!(
        pen(b"\x1b[31;44m\x1b[39;49m"),
        termelon_vt::Style::default()
    );
}

#[test]
fn sgr_indexed_and_rgb_colours() {
    assert_eq!(pen(b"\x1b[38;5;1m").fg, Color::Indexed(1));
    assert_eq!(pen(b"\x1b[48;5;200m").bg, Color::Indexed(200));
    assert_eq!(pen(b"\x1b[38;2;1;2;3m").fg, Color::Rgb(1, 2, 3));
    assert_eq!(pen(b"\x1b[48;2;4;5;6;1m").bg, Color::Rgb(4, 5, 6));
    assert!(pen(b"\x1b[48;2;4;5;6;1m").bold);
}

#[test]
fn sgr_colon_forms() {
    assert_eq!(pen(b"\x1b[38:5:9m").fg, Color::Indexed(9));
    assert_eq!(pen(b"\x1b[38:2:1:2:3m").fg, Color::Rgb(1, 2, 3));
    assert_eq!(pen(b"\x1b[38:2::1:2:3m").fg, Color::Rgb(1, 2, 3));
    assert_eq!(pen(b"\x1b[48:2:0:7:8:9m").bg, Color::Rgb(7, 8, 9));
}

#[test]
fn sgr_underline_colour() {
    assert_eq!(
        pen(b"\x1b[58;2;1;2;3m").underline_color,
        Color::Rgb(1, 2, 3)
    );
    assert_eq!(pen(b"\x1b[58:5:4m").underline_color, Color::Indexed(4));
    assert_eq!(pen(b"\x1b[58;5;4m\x1b[59m").underline_color, Color::Default);
}

#[test]
fn sgr_unknown_parameters_fail_but_the_rest_apply() {
    let mut term = Emulator::new(5, 1);
    assert!(term.feed(b"\x1b[1;99;3m").is_err());
    assert!(term.pen().bold && term.pen().italic);
    assert!(term.feed(b"\x1b[38;5;300m").is_err());
    assert!(term.feed(b"\x1b[38;2;1m").is_err());
    assert!(term.feed(b"\x1b[1:2m").is_err());
    assert!(term.feed(b"\x1b[4:9m").is_err());
}

#[test]
fn drawn_cells_take_the_pen() {
    let term = run(5, 1, b"\x1b[1;38;5;3mab\x1b[0mc");
    assert!(term.cell(0, 0).unwrap().style().bold);
    assert_eq!(term.cell(1, 0).unwrap().style().fg, Color::Indexed(3));
    assert!(term.cell(2, 0).unwrap().style().is_default());
}

#[test]
fn a_wide_characters_continuation_carries_its_style() {
    let term = run(5, 1, "\x1b[41m字".as_bytes());
    assert_eq!(term.cell(1, 0).unwrap().style().bg, Color::Red);
}

// Modes.

#[test]
fn application_cursor_keys() {
    assert!(run(5, 1, b"\x1b[?1h").mode(1));
    assert!(!run(5, 1, b"\x1b[?1h\x1b[?1l").mode(1));
}

#[test]
fn mouse_modes_replace_each_other() {
    let term = run(5, 1, b"\x1b[?1000h\x1b[?1003h");
    assert!(term.mode(1003) && !term.mode(1000));
    assert!(run(5, 1, b"\x1b[?9h").mode(9));
    assert!(run(5, 1, b"\x1b[?1002h").mode(1002));
    let term = run(5, 1, b"\x1b[?1003h\x1b[?1000l");
    assert!(!term.mode(1003));
}

#[test]
fn several_modes_in_one_sequence() {
    let term = run(5, 1, b"\x1b[?1000;1006;1004;2004h");
    assert!(term.mode(1000) && term.mode(1006) && term.mode(1004) && term.mode(2004));
    assert_eq!(
        term.events(),
        [
            Event::ModeSet(1000),
            Event::ModeSet(1006),
            Event::ModeSet(1004),
            Event::ModeSet(2004)
        ]
    );
    let term = run(
        5,
        1,
        b"\x1b[?1000;1006;1004;2004h\x1b[?2004;1004;1006;1000l",
    );
    assert!(!term.mode(1000) && !term.mode(1006) && !term.mode(1004) && !term.mode(2004));
}

#[test]
fn cursor_blink_mode() {
    assert!(run(5, 1, b"\x1b[?12h").cursor().blinking);
    assert!(!run(5, 1, b"\x1b[?12h\x1b[?12l").cursor().blinking);
}

#[test]
fn cursor_visibility() {
    let term = run(5, 1, b"\x1b[2C\x1b[?25l");
    assert!(!term.cursor().visible);
    assert_eq!(
        term.events().last(),
        Some(&Event::CursorHidden { at: Pos::new(2, 0) })
    );
    let term = run(5, 1, b"\x1b[?25l\x1b[?25h");
    assert!(term.cursor().visible);
    assert_eq!(
        term.events().last(),
        Some(&Event::CursorShown { at: Pos::new(0, 0) })
    );
}

#[test]
fn mode_1049_saves_the_cursor_and_clears_the_alternate_screen() {
    let mut term = Emulator::new(4, 2);
    term.feed(b"\x1b[2;2HM\x1b[?1049h").unwrap();
    assert!(term.alternate_screen());
    assert_eq!(term.rows(), ["", ""]);
    assert_eq!(at(&term), (2, 1));
    term.feed(b"\x1b[HA\x1b[?1049l").unwrap();
    assert!(!term.alternate_screen());
    assert_eq!(term.rows(), ["", " M"]);
    assert_eq!(at(&term), (2, 1));
    term.feed(b"\x1b[?1049h").unwrap();
    assert_eq!(term.rows(), ["", ""]);
}

#[test]
fn mode_1047_clears_the_alternate_screen_when_leaving() {
    let mut term = Emulator::new(4, 1);
    term.feed(b"M\x1b[?1047hA\x1b[?1047l").unwrap();
    assert_eq!(term.rows(), ["M"]);
    term.feed(b"\x1b[?1047h").unwrap();
    assert_eq!(term.rows(), [""]);
}

#[test]
fn mode_47_keeps_the_alternate_screen() {
    let mut term = Emulator::new(4, 1);
    term.feed(b"M\x1b[?47h\rA\x1b[?47l").unwrap();
    assert_eq!(term.rows(), ["M"]);
    term.feed(b"\x1b[?47h").unwrap();
    assert_eq!(term.rows(), ["A"]);
}

#[test]
fn mode_1048_saves_and_restores_the_cursor() {
    assert_eq!(
        at(&run(5, 3, b"\x1b[2;3H\x1b[?1048h\x1b[H\x1b[?1048l")),
        (2, 1)
    );
}

#[test]
fn the_screens_keep_their_own_saved_cursor() {
    let term = run(
        5,
        3,
        b"\x1b[2;2H\x1b7\x1b[?1049h\x1b[3;3H\x1b7\x1b[?1049l\x1b8",
    );
    assert_eq!(at(&term), (1, 1));
}

#[test]
fn synchronized_update() {
    let term = run_as(Profile::kitty(), 5, 1, b"\x1b[?2026ha\x1b[?2026l");
    assert!(!term.mode(2026));
    assert!(matches!(
        term.events(),
        [Event::SyncBegin, Event::Drawn { .. }, Event::SyncEnd]
    ));
    assert!(run_as(Profile::kitty(), 5, 1, b"\x1b[?2026h").mode(2026));
}

#[test]
fn a_mode_the_terminal_lacks_is_ignored_without_error() {
    let term = run(5, 1, b"\x1b[?2026h\x1b[?2027h");
    assert!(!term.mode(2026) && !term.mode(2027));
    assert_eq!(term.events(), []);
}

#[test]
fn an_unknown_mode_is_unknown() {
    assert_eq!(unknown(b"\x1b[?9999h"), ["`\\e[?9999h`"]);
    assert_eq!(unknown(b"\x1b[4h"), ["`\\e[4h`"]);
}

#[test]
fn a_known_mode_beside_an_unknown_one_still_applies() {
    let mut term = Emulator::new(5, 1);
    assert!(term.feed(b"\x1b[?2004;9999h").is_err());
    assert!(term.mode(2004));
}

#[test]
fn decrqm_answers_set_reset_and_unknown() {
    let mut term = Emulator::new(5, 1);
    term.feed(b"\x1b[?1049$p\x1b[?7$p\x1b[?2026$p\x1b[?12345$p\x1b[4$p")
        .unwrap();
    assert_eq!(
        term.take_output(),
        b"\x1b[?1049;2$y\x1b[?7;1$y\x1b[?2026;0$y\x1b[?12345;0$y\x1b[4;0$y"
    );
    term.feed(b"\x1b[?1049h\x1b[?1049$p").unwrap();
    assert_eq!(term.take_output(), b"\x1b[?1049;1$y");
}

#[test]
fn decscusr_sets_shape_and_blinking() {
    let shape = |seq: &[u8]| {
        let cursor = run(5, 1, seq).cursor();
        (cursor.shape, cursor.blinking)
    };
    assert_eq!(shape(b"\x1b[0 q"), (CursorShape::Default, false));
    assert_eq!(shape(b"\x1b[1 q"), (CursorShape::Block, true));
    assert_eq!(shape(b"\x1b[2 q"), (CursorShape::Block, false));
    assert_eq!(shape(b"\x1b[3 q"), (CursorShape::Underline, true));
    assert_eq!(shape(b"\x1b[4 q"), (CursorShape::Underline, false));
    assert_eq!(shape(b"\x1b[5 q"), (CursorShape::Bar, true));
    assert_eq!(shape(b"\x1b[6 q"), (CursorShape::Bar, false));
    assert_eq!(shape(b"\x1b[ q"), (CursorShape::Default, false));
    assert_eq!(unknown(b"\x1b[7 q"), ["`\\e[7 q`"]);
}

// Keyboard modes.

#[test]
fn kitty_flags_push_pop_and_set() {
    let mut term = Emulator::with_profile(Profile::kitty(), 5, 1);
    let mut flags = |seq: &[u8]| {
        term.feed(seq).unwrap();
        term.keyboard_flags()
    };
    assert_eq!(flags(b"\x1b[>1u"), 1);
    assert_eq!(flags(b"\x1b[>5u"), 5);
    assert_eq!(flags(b"\x1b[<u"), 1);
    assert_eq!(flags(b"\x1b[<u"), 0);
    assert_eq!(flags(b"\x1b[=3u"), 3);
    assert_eq!(flags(b"\x1b[=4;2u"), 7);
    assert_eq!(flags(b"\x1b[=1;3u"), 6);
    assert_eq!(flags(b"\x1b[>31u\x1b[>2u\x1b[<2u"), 0);
}

#[test]
fn kitty_flags_are_kept_per_screen() {
    let mut term = Emulator::with_profile(Profile::kitty(), 5, 1);
    term.feed(b"\x1b[>1u\x1b[?1049h").unwrap();
    assert_eq!(term.keyboard_flags(), 0);
    term.feed(b"\x1b[>8u\x1b[?1049l").unwrap();
    assert_eq!(term.keyboard_flags(), 1);
}

#[test]
fn kitty_query() {
    let mut term = Emulator::with_profile(Profile::kitty(), 5, 1);
    term.feed(b"\x1b[>3u\x1b[?u").unwrap();
    assert_eq!(term.take_output(), b"\x1b[?3u");
}

#[test]
fn kitty_sequences_are_ignored_where_the_terminal_lacks_them() {
    let term = run(5, 1, b"\x1b[>1u\x1b[?u\x1b[=1u\x1b[<u");
    assert_eq!(term.keyboard_flags(), 0);
    assert_eq!(term.output(), []);
}

#[test]
fn modify_other_keys() {
    assert_eq!(run(5, 1, b"\x1b[>4;2m").modify_other_keys(), 2);
    assert_eq!(run(5, 1, b"\x1b[>4;2m\x1b[>4m").modify_other_keys(), 0);
    assert_eq!(
        run_as(Profile::kitty(), 5, 1, b"\x1b[>4;2m").modify_other_keys(),
        0
    );
    assert_eq!(unknown(b"\x1b[>4;7m"), ["`\\e[>4;7m`"]);
    assert_eq!(unknown(b"\x1b[>1;2m"), ["`\\e[>1;2m`"]);
}

// Queries.

#[test]
fn device_attributes() {
    let mut term = Emulator::new(5, 1);
    term.feed(b"\x1b[c\x1b[0c\x1b[>c\x1b[>0c").unwrap();
    let da1 = "\x1b[?64;1;2;6;9;15;16;17;18;21;22;28c";
    let da2 = "\x1b[>41;390;0c";
    assert_eq!(term.take_output(), [da1, da1, da2, da2].concat().as_bytes());
}

#[test]
fn xtversion() {
    let mut term = Emulator::new(5, 1);
    term.feed(b"\x1b[>q\x1b[>0q").unwrap();
    assert_eq!(
        term.take_output(),
        b"\x1bP>|XTerm(390)\x1b\\\x1bP>|XTerm(390)\x1b\\"
    );
}

#[test]
fn status_and_cursor_reports() {
    let mut term = Emulator::new(10, 5);
    term.feed(b"\x1b[5n\x1b[3;4H\x1b[6n").unwrap();
    assert_eq!(term.take_output(), b"\x1b[0n\x1b[3;4R");
    term.feed(b"\x1b[2;4r\x1b[?6h\x1b[2;3H\x1b[6n").unwrap();
    assert_eq!(term.take_output(), b"\x1b[2;3R");
}

#[test]
fn a_cursor_report_at_a_pending_wrap_gives_the_last_column() {
    let mut term = Emulator::new(3, 2);
    term.feed(b"abc\x1b[6n").unwrap();
    assert_eq!(term.take_output(), b"\x1b[1;3R");
}

#[test]
fn size_report() {
    let mut term = Emulator::new(80, 24);
    term.feed(b"\x1b[18t").unwrap();
    assert_eq!(term.take_output(), b"\x1b[8;24;80t");
}

#[test]
fn title_stack() {
    let term = run(5, 1, b"\x1b]2;one\x07\x1b[22;0t\x1b]2;two\x07\x1b[23;0t");
    assert_eq!(term.title(), "one");
    assert_eq!(term.title_stack(), []);
    let titles: Vec<&Event> = term
        .events()
        .iter()
        .filter(|e| matches!(e, Event::TitleSet(_)))
        .collect();
    assert_eq!(
        titles,
        [
            &Event::TitleSet("one".into()),
            &Event::TitleSet("two".into()),
            &Event::TitleSet("one".into())
        ]
    );
}

#[test]
fn title_stack_restores_only_what_the_pop_names() {
    let term = run(5, 1, b"\x1b]0;one\x07\x1b[22t\x1b]0;two\x07\x1b[23;1t");
    assert_eq!((term.title(), term.icon_title()), ("two", "one"));
    let term = run(5, 1, b"\x1b]0;one\x07\x1b[22t\x1b]0;two\x07\x1b[23;2t");
    assert_eq!((term.title(), term.icon_title()), ("one", "two"));
}

#[test]
fn a_pop_from_an_empty_title_stack_does_nothing() {
    assert_eq!(run(5, 1, b"\x1b]2;one\x07\x1b[23;0t").title(), "one");
}

#[test]
fn unknown_window_ops_are_unknown() {
    assert_eq!(unknown(b"\x1b[14t"), ["`\\e[14t`"]);
}

// OSC.

#[test]
fn osc_titles() {
    let term = run(5, 1, b"\x1b]0;both\x07");
    assert_eq!((term.title(), term.icon_title()), ("both", "both"));
    let term = run(5, 1, b"\x1b]1;icon\x1b\\");
    assert_eq!((term.title(), term.icon_title()), ("", "icon"));
    let term = run(5, 1, "\x1b]2;tïtle\x1b\\".as_bytes());
    assert_eq!((term.title(), term.icon_title()), ("tïtle", ""));
    assert_eq!(term.events(), [Event::TitleSet("tïtle".into())]);
}

#[test]
fn osc_8_hyperlinks() {
    let term = run_as(
        Profile::kitty(),
        10,
        1,
        b"\x1b]8;id=7;https://example.com/a\x1b\\ab\x1b]8;;\x1b\\c",
    );
    let link = term.cell(0, 0).unwrap().style().link.clone().unwrap();
    assert_eq!(link.url, "https://example.com/a");
    assert_eq!(link.id.as_deref(), Some("7"));
    assert!(term.cell(1, 0).unwrap().style().link.is_some());
    assert!(term.cell(2, 0).unwrap().style().link.is_none());
}

#[test]
fn osc_8_survives_sgr_0() {
    let term = run_as(
        Profile::kitty(),
        10,
        1,
        b"\x1b]8;;https://example.com\x1b\\\x1b[0ma",
    );
    assert!(term.cell(0, 0).unwrap().style().link.is_some());
}

#[test]
fn osc_8_is_ignored_where_the_terminal_lacks_links() {
    let term = run(10, 1, b"\x1b]8;;https://example.com\x1b\\a");
    assert!(term.cell(0, 0).unwrap().style().link.is_none());
}

#[test]
fn osc_colour_queries_answer_with_the_query_terminator() {
    let mut term = Emulator::new(5, 1);
    term.feed(b"\x1b]11;?\x07\x1b]10;?\x1b\\").unwrap();
    assert_eq!(
        term.take_output(),
        b"\x1b]11;rgb:ffff/ffff/ffff\x07\x1b]10;rgb:0000/0000/0000\x1b\\"
    );
}

#[test]
fn osc_52_writes_are_recorded() {
    let term = run(5, 1, b"\x1b]52;c;aGVsbG8=\x07\x1b]52;;\x07\x1b]52;c;?\x07");
    let writes = term.clipboard();
    assert_eq!(writes.len(), 2);
    assert_eq!(
        (writes[0].selection.as_str(), writes[0].data.as_slice()),
        ("c", &b"hello"[..])
    );
    assert_eq!(writes[1].data, b"");
    assert_eq!(unknown(b"\x1b]52;c;@@\x07"), ["`\\e]52;c;@@\\x07`"]);
}

#[test]
fn an_unknown_osc_is_unknown() {
    assert_eq!(
        unknown(b"\x1b]4;1;rgb:00/00/00\x1b\\"),
        ["`\\e]4;1;rgb:00/00/00\\e\\\\`"]
    );
    assert_eq!(unknown(b"\x1b]10;red\x07"), ["`\\e]10;red\\x07`"]);
}

// DCS and other strings.

#[test]
fn xtgettcap() {
    let mut term = Emulator::new(5, 1);
    term.feed(b"\x1bP+q544e;5463\x1b\\").unwrap();
    assert_eq!(
        term.take_output(),
        b"\x1bP1+r544e=787465726D\x1b\\\x1bP0+r5463\x1b\\"
    );
}

#[test]
fn other_strings_are_unknown() {
    assert_eq!(unknown(b"\x1b_Gf=1\x1b\\"), ["`\\e_Gf=1\\e\\\\`"]);
    assert_eq!(unknown(b"\x1bPqdata\x1b\\"), ["`\\ePqdata\\e\\\\`"]);
    assert_eq!(unknown(b"\x1bOA"), ["`\\eOA`"]);
}

// Strict mode.

#[test]
fn strict_mode_names_every_unknown_sequence() {
    let mut term = Emulator::new(10, 1);
    let error = term.feed(b"a\x1b[?9999hb\x1b[99m").unwrap_err();
    assert_eq!(
        error.to_string(),
        "unknown sequences `\\e[?9999h`, `\\e[99m`"
    );
    assert_eq!(term.rows(), ["ab"]);
    assert_eq!(term.unknown_sequences().len(), 2);
}

#[test]
fn lenient_mode_records_without_failing() {
    let mut term = Emulator::new(10, 1);
    term.set_strict(false);
    term.feed(b"\x1b[?9999h").unwrap();
    assert_eq!(term.unknown_sequences()[0].bytes(), b"\x1b[?9999h");
}

#[test]
fn invalid_utf8_is_unknown_and_draws_a_replacement() {
    let mut term = Emulator::new(5, 1);
    assert!(term.feed(b"a\xffb").is_err());
    assert_eq!(term.rows(), ["a\u{fffd}b"]);
}

#[test]
fn a_c1_control_written_as_utf8_is_unknown() {
    assert_eq!(unknown(b"\xc2\x9b"), ["`\\x9b`"]);
}

#[test]
fn a_sequence_split_across_feeds() {
    let mut term = Emulator::new(10, 2);
    term.feed(b"\x1b[2").unwrap();
    assert!(term.in_sequence());
    term.feed(b";3H").unwrap();
    assert!(!term.in_sequence());
    assert_eq!(at(&term), (2, 1));
}

// Events.

#[test]
fn drawn_events_say_whether_the_cursor_showed() {
    let term = run(5, 1, b"a\x1b[?25lb");
    let drawn: Vec<(Pos, bool)> = term
        .events()
        .iter()
        .filter_map(|e| match e {
            Event::Drawn {
                at, cursor_visible, ..
            } => Some((*at, *cursor_visible)),
            _ => None,
        })
        .collect();
    assert_eq!(drawn, [(Pos::new(0, 0), true), (Pos::new(1, 0), false)]);
}

#[test]
fn drawn_events_give_the_cursor_before_a_wrap() {
    let term = run(2, 2, b"abc");
    assert_eq!(
        term.events().last(),
        Some(&Event::Drawn {
            at: Pos::new(0, 1),
            width: 1,
            cursor: Pos::new(1, 0),
            cursor_visible: true
        })
    );
}

#[test]
fn cursor_moves_are_events_but_drawing_is_not() {
    let term = run(5, 3, b"ab\x1b[2;3H\x1b[2;3H\r");
    let moves: Vec<&Event> = term
        .events()
        .iter()
        .filter(|e| matches!(e, Event::CursorMoved { .. }))
        .collect();
    assert_eq!(
        moves,
        [
            &Event::CursorMoved {
                from: Pos::new(2, 0),
                to: Pos::new(2, 1)
            },
            &Event::CursorMoved {
                from: Pos::new(2, 1),
                to: Pos::new(0, 1)
            },
        ]
    );
}

#[test]
fn take_events_clears_them() {
    let mut term = run(5, 1, b"\x07");
    assert_eq!(term.take_events(), [Event::Bell]);
    assert_eq!(term.events(), []);
    term.set_record_events(false);
    term.feed(b"\x07").unwrap();
    assert_eq!(term.events(), []);
}

// The Linux console.

#[test]
fn the_linux_console_prints_an_osc_it_doesnt_know() {
    let mut term = Emulator::with_profile(Profile::linux_console(), 10, 1);
    assert!(term.feed(b"\x1b]0;title\x07").is_err());
    assert_eq!(term.rows(), [";title"]);
}

#[test]
fn the_linux_console_takes_esc_p_as_a_palette_command() {
    let mut term = Emulator::with_profile(Profile::linux_console(), 10, 1);
    assert!(term.feed(b"\x1bP+q5463\x1b\\").is_err());
    assert_eq!(term.rows(), ["q5463"]);
    term.feed(b"\r\x1b]P0ff0000\x1b]R\x1bP1abcdef").unwrap();
    assert_eq!(term.rows(), ["q5463"]);
}

#[test]
fn the_linux_console_has_no_alternate_screen() {
    let term = run_as(Profile::linux_console(), 5, 1, b"a\x1b[?1049hb");
    assert!(!term.alternate_screen());
    assert_eq!(term.rows(), ["ab"]);
}

// Resizing.

#[test]
fn resize_keeps_the_top_left_and_the_cursor_inside() {
    let mut term = run(4, 3, b"abcd\x1b[3;4H");
    term.resize(2, 2);
    assert_eq!(term.rows(), ["ab", ""]);
    assert_eq!(at(&term), (1, 1));
    term.resize(3, 3);
    assert_eq!(term.rows(), ["ab", "", ""]);
    assert_eq!(term.size(), (3, 3));
}
