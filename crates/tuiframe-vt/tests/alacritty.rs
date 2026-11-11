//! A cross-check against `alacritty_terminal`, an independent emulator. Random streams of the
//! sequences both know go through both, and their screens must agree in text, colours,
//! attributes, links and the cursor.
//!
//! The streams steer clear of the places where the two disagree on purpose. Each of those is a
//! test below, which pins what each emulator does, and the crate docs list them.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AColor, CursorShape as AShape, NamedColor, Processor,
};
use proptest::prelude::*;
use tuiframe_vt::{Color, CursorShape, Emulator, Profile, Underline};

/// What both emulators keep of a cell.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Attrs {
    fg: Color,
    bg: Color,
    underline_color: Color,
    underline: Underline,
    bold: bool,
    dim: bool,
    italic: bool,
    reverse: bool,
    hidden: bool,
    strike: bool,
    link: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    rows: Vec<String>,
    attrs: Vec<Vec<Attrs>>,
    cursor: (u16, u16),
    visible: bool,
    /// The cursor's shape, with the default as a block, which is Alacritty's default.
    shape: CursorShape,
    /// Modes 1, 7, 1004, 1006, 1049 and 2004.
    modes: [bool; 6],
}

const MODES: [(u16, TermMode); 6] = [
    (1, TermMode::APP_CURSOR),
    (7, TermMode::LINE_WRAP),
    (1004, TermMode::FOCUS_IN_OUT),
    (1006, TermMode::SGR_MOUSE),
    (1049, TermMode::ALT_SCREEN),
    (2004, TermMode::BRACKETED_PASTE),
];

/// The profile closest to Alacritty: xterm's, with hyperlinks and synchronized output.
fn alacritty_profile() -> Profile {
    let mut profile = Profile {
        hyperlinks: true,
        ..Profile::xterm()
    };
    profile.modes.push(2026);
    profile
}

fn ours(term: &Emulator) -> Snapshot {
    let (cols, rows) = term.size();
    let attrs = (0..rows)
        .map(|y| {
            (0..cols)
                .map(|x| {
                    let style = term
                        .cell(x, y)
                        .map(|cell| cell.style().clone())
                        .unwrap_or_default();
                    Attrs {
                        fg: style.fg,
                        bg: style.bg,
                        underline_color: style.underline_color,
                        underline: style.underline,
                        bold: style.bold,
                        dim: style.dim,
                        italic: style.italic,
                        reverse: style.reverse,
                        hidden: style.hidden,
                        strike: style.strike,
                        link: style.link.as_ref().map(|link| link.url.clone()),
                    }
                })
                .collect()
        })
        .collect();
    let cursor = term.cursor();
    Snapshot {
        rows: term.rows(),
        attrs,
        cursor: (cursor.x, cursor.y),
        visible: cursor.visible,
        shape: match cursor.shape {
            CursorShape::Default => CursorShape::Block,
            shape => shape,
        },
        modes: MODES.map(|(mode, _)| term.mode(mode)),
    }
}

fn color(color: AColor) -> Color {
    match color {
        AColor::Named(NamedColor::Foreground | NamedColor::Background) => Color::Default,
        AColor::Named(named) => u8::try_from(named as usize)
            .ok()
            .and_then(Color::named)
            .unwrap_or_default(),
        AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AColor::Indexed(n) => Color::Indexed(n),
    }
}

struct Alacritty {
    term: Term<VoidListener>,
    parser: Processor,
}

impl Alacritty {
    fn new(cols: u16, rows: u16) -> Alacritty {
        let size = TermSize::new(usize::from(cols), usize::from(rows));
        Alacritty {
            term: Term::new(Config::default(), &size, VoidListener),
            parser: Processor::new(),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        // Alacritty holds back what comes inside a synchronized update until it ends.
        self.parser.stop_sync(&mut self.term);
    }

    fn snapshot(&self, cols: u16, rows: u16) -> Snapshot {
        let grid = self.term.grid();
        let mut texts = Vec::new();
        let mut attrs = Vec::new();
        for y in 0..rows {
            let row = &grid[Line(i32::from(y))];
            let mut text = String::new();
            let mut row_attrs = Vec::new();
            for x in 0..cols {
                let cell = &row[Column(usize::from(x))];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    // Alacritty keeps a tab in a blank cell it passes over, for copying.
                    text.push(if cell.c == '\t' { ' ' } else { cell.c });
                    text.extend(cell.zerowidth().unwrap_or_default());
                }
                let flags = cell.flags;
                let underline = if flags.contains(Flags::DOUBLE_UNDERLINE) {
                    Underline::Double
                } else if flags.contains(Flags::UNDERCURL) {
                    Underline::Curly
                } else if flags.contains(Flags::DOTTED_UNDERLINE) {
                    Underline::Dotted
                } else if flags.contains(Flags::DASHED_UNDERLINE) {
                    Underline::Dashed
                } else if flags.contains(Flags::UNDERLINE) {
                    Underline::Single
                } else {
                    Underline::None
                };
                row_attrs.push(Attrs {
                    fg: color(cell.fg),
                    bg: color(cell.bg),
                    underline_color: cell.underline_color().map_or(Color::Default, color),
                    underline,
                    bold: flags.contains(Flags::BOLD),
                    dim: flags.contains(Flags::DIM),
                    italic: flags.contains(Flags::ITALIC),
                    reverse: flags.contains(Flags::INVERSE),
                    hidden: flags.contains(Flags::HIDDEN),
                    strike: flags.contains(Flags::STRIKEOUT),
                    link: cell.hyperlink().map(|link| link.uri().to_owned()),
                });
            }
            texts.push(text.trim_end_matches(' ').to_owned());
            attrs.push(row_attrs);
        }
        let point = grid.cursor.point;
        let mode = self.term.mode();
        Snapshot {
            rows: texts,
            attrs,
            cursor: (
                u16::try_from(point.column.0).unwrap_or(u16::MAX),
                u16::try_from(point.line.0).unwrap_or(u16::MAX),
            ),
            visible: mode.contains(TermMode::SHOW_CURSOR),
            shape: match self.term.cursor_style().shape {
                AShape::Underline => CursorShape::Underline,
                AShape::Beam => CursorShape::Bar,
                _ => CursorShape::Block,
            },
            modes: MODES.map(|(_, flag)| mode.contains(flag)),
        }
    }
}

/// Feeds the same bytes to both and returns their snapshots, ours first.
#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn both(cols: u16, rows: u16, bytes: &[u8]) -> (Snapshot, Snapshot) {
    let mut term = Emulator::with_profile(alacritty_profile(), cols, rows);
    term.feed(bytes).unwrap();
    let mut alacritty = Alacritty::new(cols, rows);
    alacritty.feed(bytes);
    (ours(&term), alacritty.snapshot(cols, rows))
}

// The stream generator.

/// One step of a stream. Steps that would land on a known difference come with a cursor move
/// first, which takes the cursor out of a pending wrap.
#[derive(Clone, Debug)]
enum Op {
    /// Narrow text, which may wrap.
    Flow(String),
    /// A move, then text that fits on the row from there.
    Placed(u16, u16, String),
    Control(&'static str),
    Cup(u16, u16),
    /// A relative move: CUF, CUB, CHA or VPA, and CUU, CUD, CNL or CPL in streams without a scroll
    /// region.
    Move(char, u16),
    /// A move, then HT, IND, NEL, RI or LF.
    PlacedControl(u16, u16, &'static str),
    /// A move, then an erase. In streams with wide characters, only erases of whole rows.
    Erase(u16, u16, char, u16),
    Sgr(String),
    Region(u16, u16),
    Scroll(char, u16),
    Raw(&'static str),
    Link(Option<String>),
    Title(String),
}

impl Op {
    fn bytes(&self) -> String {
        match self {
            Op::Flow(text) => text.clone(),
            Op::Placed(y, x, text) => format!("\x1b[{};{}H{text}", y + 1, x + 1),
            Op::Control(control) | Op::Raw(control) => (*control).to_owned(),
            Op::Cup(y, x) => format!("\x1b[{y};{x}H"),
            Op::Move(letter, n) => format!("\x1b[{n}{letter}"),
            Op::PlacedControl(y, x, control) => format!("\x1b[{};{}H{control}", y + 1, x + 1),
            Op::Erase(y, x, letter, how) => format!("\x1b[{};{}H\x1b[{how}{letter}", y + 1, x + 1),
            Op::Sgr(params) => format!("\x1b[{params}m"),
            Op::Region(top, bottom) => format!("\x1b[{top};{bottom}r"),
            Op::Scroll(letter, n) => format!("\x1b[{n}{letter}"),
            Op::Link(Some(url)) => format!("\x1b]8;;{url}\x1b\\"),
            Op::Link(None) => "\x1b]8;;\x1b\\".to_owned(),
            Op::Title(title) => format!("\x1b]2;{title}\x07"),
        }
    }
}

const NARROW: &[&str] = &[
    "a", "b", "Z", "0", " ", "-", "é", "ß", "─", "│", "⣷", "…", "→", "~",
];
const WIDE: &[&str] = &["字", "한", "\u{1f600}", "\u{ff21}"];

fn narrow_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(prop::sample::select(NARROW), 1..12).prop_map(|parts| parts.concat())
}

/// Text for a placed op, with its width in cells: narrow and wide characters, and combining marks
/// after a base character.
fn placed_text(wide: bool) -> impl Strategy<Value = Vec<(&'static str, u16)>> {
    let mut choices: Vec<(&'static str, u16)> = NARROW.iter().map(|&s| (s, 1)).collect();
    choices.push(("e\u{301}", 1));
    choices.push(("a\u{308}", 1));
    if wide {
        choices.extend(WIDE.iter().map(|&s| (s, 2)));
    }
    proptest::collection::vec(prop::sample::select(choices), 1..10)
}

fn sgr() -> impl Strategy<Value = String> {
    let simple = prop::sample::select(vec![
        "0", "1", "2", "3", "4", "4:0", "4:1", "4:2", "4:3", "4:4", "4:5", "7", "8", "9", "22",
        "23", "24", "27", "28", "29", "30", "31", "37", "39", "40", "44", "47", "49", "90", "95",
        "97", "100", "107", "59", "",
    ])
    .prop_map(str::to_owned);
    let extended = (
        prop::sample::select(vec![38u16, 48, 58]),
        any::<u8>(),
        any::<(u8, u8, u8)>(),
        0..4u8,
    )
        .prop_map(|(p, n, (r, g, b), form)| match form {
            0 => format!("{p};5;{n}"),
            1 => format!("{p};2;{r};{g};{b}"),
            2 => format!("{p}:5:{n}"),
            _ => format!("{p}:2::{r}:{g}:{b}"),
        });
    proptest::collection::vec(prop_oneof![3 => simple, 1 => extended], 1..4)
        .prop_map(|params| params.join(";"))
}

fn op(cols: u16, rows: u16, wide: bool, regions: bool) -> impl Strategy<Value = Op> {
    let at = (0..rows, 0..cols);
    let placed = (0..rows, 0..cols, placed_text(wide)).prop_map(move |(y, x, parts)| {
        let mut room = cols - x;
        let mut text = String::new();
        for (part, width) in parts {
            if width > room {
                break;
            }
            room -= width;
            text.push_str(part);
        }
        Op::Placed(y, x, text)
    });
    let relative = if regions { "CDG`d" } else { "ABCDEFG`d" };
    let moves = (
        prop::sample::select(relative.chars().collect::<Vec<_>>()),
        0..(cols.max(rows) + 3),
    )
        .prop_map(|(letter, n)| Op::Move(letter, n));
    let erase = (0..rows, 0..cols, prop::bool::ANY, 0..3u16).prop_map(move |(y, x, line, how)| {
        let letter = if line { 'K' } else { 'J' };
        if !wide {
            // Alacritty's ED 1 from the second row leaves the first (see the quirks).
            let how = if letter == 'J' && how == 1 && y == 1 {
                2
            } else {
                how
            };
            return Op::Erase(y, x, letter, how);
        }
        // Whole rows only: from the left edge, or up to the right edge.
        match how {
            0 => Op::Erase(y, 0, letter, 0),
            1 if !(letter == 'J' && y == 1) => Op::Erase(y, cols - 1, letter, 1),
            _ => Op::Erase(y, x, letter, 2),
        }
    });
    let region = (1..=rows, 1..=rows).prop_map(move |(a, b)| {
        if regions && a != b {
            Op::Region(a.min(b), a.max(b))
        } else {
            Op::Control("\r")
        }
    });
    let link = prop::option::of(prop::sample::select(vec![
        "https://example.com/a",
        "https://example.com/b",
    ]))
    .prop_map(|url| Op::Link(url.map(str::to_owned)));
    prop_oneof![
        6 => narrow_text().prop_map(Op::Flow),
        6 => placed,
        2 => prop::sample::select(vec!["\r\n", "\r", "\x08", "\x07"]).prop_map(Op::Control),
        3 => (0..rows + 3, 0..cols + 3).prop_map(|(y, x)| Op::Cup(y, x)),
        3 => moves,
        2 => (at.clone(), prop::sample::select(vec!["\t", "\x1bD", "\x1bE", "\x1bM", "\n"]))
            .prop_map(|((y, x), control)| Op::PlacedControl(y, x, control)),
        3 => erase,
        4 => sgr().prop_map(Op::Sgr),
        1 => region,
        1 => (prop::sample::select(vec!['S', 'T']), 0..rows + 2).prop_map(|(letter, n)| Op::Scroll(letter, n)),
        2 => prop::sample::select(vec![
            "\x1b7", "\x1b8", "\x1b[s", "\x1b[u", "\x1b[?1049h", "\x1b[?1049l", "\x1b[?7l", "\x1b[?7h\r",
            "\x1b[?25l", "\x1b[?25h", "\x1b[?1h", "\x1b[?1l", "\x1b[?2004h", "\x1b[?2004l", "\x1b[?1004h",
            "\x1b[?1006h", "\x1b[?2026h", "\x1b[?2026l", "\x1b[2 q", "\x1b[4 q", "\x1b[5 q", "\x1b[0 q",
            "\x1b(0", "\x1b(B", "\x1b[3J",
        ])
        .prop_map(Op::Raw),
        1 => link,
        1 => "[a-z]{0,6}".prop_map(Op::Title),
    ]
}

fn stream(wide: bool, regions: bool) -> impl Strategy<Value = (u16, u16, Vec<Op>)> {
    (2u16..30, 1u16..8).prop_flat_map(move |(cols, rows)| {
        (
            Just(cols),
            Just(rows),
            proptest::collection::vec(op(cols, rows, wide, regions), 1..60),
        )
    })
}

fn check(cols: u16, rows: u16, ops: &[Op]) -> Result<(), TestCaseError> {
    let bytes: String = ops.iter().map(Op::bytes).collect();
    let (ours, theirs) = both(cols, rows, bytes.as_bytes());
    prop_assert_eq!(&ours.rows, &theirs.rows, "text");
    for (y, (a, b)) in ours.attrs.iter().zip(&theirs.attrs).enumerate() {
        for (x, (a, b)) in a.iter().zip(b).enumerate() {
            prop_assert_eq!(a, b, "attributes at {},{}", x, y);
        }
    }
    prop_assert_eq!(ours, theirs);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn narrow_text_agrees((cols, rows, ops) in stream(false, false)) {
        check(cols, rows, &ops)?;
    }

    #[test]
    fn narrow_text_with_scroll_regions_agrees((cols, rows, ops) in stream(false, true)) {
        check(cols, rows, &ops)?;
    }

    #[test]
    fn wide_text_agrees((cols, rows, ops) in stream(true, false)) {
        check(cols, rows, &ops)?;
    }

    #[test]
    fn wide_text_with_scroll_regions_agrees((cols, rows, ops) in stream(true, true)) {
        check(cols, rows, &ops)?;
    }
}

#[test]
fn a_frame_as_the_library_writes_it_agrees() {
    let frame = "\x1b[?1049h\x1b[?7l\x1b[?2026h\x1b[?25l\x1b[1;1H\x1b[38;2;216;222;233;1mtitle\x1b[0m\
                 \x1b[2;3H\x1b[38;5;244m⣷\x1b[0m \x1b[3mWorking…\x1b[K\x1b]8;id=1;https://example.com\x1b\\link\
                 \x1b]8;;\x1b\\\x1b[3;1H\x1b[48;2;30;30;30m    \x1b[0m\x1b[4:3;58;5;196mcurly\x1b[0m\
                 \x1b[4;20Hedge!\x1b[0m\x1b[3;8H\x1b[6 q\x1b[?25h\x1b[?2026l";
    let (ours, theirs) = both(24, 5, frame.as_bytes());
    assert_eq!(ours, theirs);
}

// Where the two disagree. Each test pins both, ours first.

fn rows_of_both(cols: u16, rows: u16, bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let (ours, theirs) = both(cols, rows, bytes);
    (ours.rows, theirs.rows)
}

#[test]
fn quirk_sgr_21_is_a_double_underline() {
    let (ours, theirs) = both(5, 1, b"\x1b[1;21mx");
    assert_eq!(
        (ours.attrs[0][0].underline, ours.attrs[0][0].bold),
        (Underline::Double, true)
    );
    assert_eq!(
        (theirs.attrs[0][0].underline, theirs.attrs[0][0].bold),
        (Underline::None, false)
    );
}

#[test]
fn quirk_line_feed_ends_a_pending_wrap() {
    let (ours, theirs) = rows_of_both(3, 3, b"abc\nX");
    assert_eq!(ours, ["abc", "  X", ""]);
    assert_eq!(theirs, ["abc", "", "X"]);
    let (ours, theirs) = rows_of_both(3, 3, b"abc\x1bMX");
    assert_eq!(ours, ["  X", "abc", ""]);
    assert_eq!(theirs, ["", "Xbc", ""]);
}

#[test]
fn quirk_erasing_at_a_pending_wrap() {
    let (ours, theirs) = rows_of_both(3, 2, b"abc\x1b[KX");
    assert_eq!(ours, ["abX", ""]);
    assert_eq!(theirs, ["abc", "X"]);
    let (ours, theirs) = rows_of_both(3, 2, b"abc\x1b[JX");
    assert_eq!(ours, ["abX", ""]);
    assert_eq!(theirs, ["ab", "X"]);
}

#[test]
fn quirk_tab_at_a_pending_wrap_stays_in_the_last_column() {
    let (ours, theirs) = rows_of_both(3, 2, b"abc\tX");
    assert_eq!(ours, ["abX", ""]);
    assert_eq!(theirs, ["abc", "X"]);
}

#[test]
fn quirk_vertical_moves_stop_at_the_margins() {
    let (ours, theirs) = both(3, 5, b"\x1b[2;4r\x1b[3;1H\x1b[9AX\x1b[3;2H\x1b[9BY");
    assert_eq!(ours.rows, ["", "X", "", " Y", ""]);
    assert_eq!(theirs.rows, ["X", "", "", "", " Y"]);
}

#[test]
fn quirk_origin_mode_moves() {
    // Resetting origin mode homes the cursor.
    let (ours, theirs) = both(3, 4, b"\x1b[2;3r\x1b[?6h\x1b[2;2H\x1b[?6l");
    assert_eq!((ours.cursor, theirs.cursor), ((0, 0), (1, 2)));
    // In origin mode, Alacritty adds the region's top to CHA again.
    let (ours, theirs) = both(3, 4, b"\x1b[2;4r\x1b[?6h\x1b[1;1H\x1b[2GX");
    assert_eq!(ours.rows, ["", " X", "", ""]);
    assert_eq!(theirs.rows, ["", "", " X", ""]);
}

#[test]
fn quirk_ed_1_from_the_second_row_erases_the_first() {
    let (ours, theirs) = rows_of_both(2, 2, b"a\x1b[2;1H\x1b[1J");
    assert_eq!(ours, ["", ""]);
    assert_eq!(theirs, ["a", ""]);
}

#[test]
fn quirk_erasing_half_a_wide_character_erases_all_of_it() {
    let (ours, theirs) = rows_of_both(4, 1, "字b\x1b[1;2H\x1b[K".as_bytes());
    assert_eq!(ours, [""]);
    assert_eq!(theirs, ["字"]);
}

#[test]
fn quirk_grapheme_clusters_take_one_cell() {
    let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    let mut term = Emulator::with_profile(alacritty_profile(), 10, 1);
    term.feed(family.as_bytes()).unwrap();
    assert_eq!(term.cell(0, 0).unwrap().text(), family);
    let mut alacritty = Alacritty::new(10, 1);
    alacritty.feed(family.as_bytes());
    let snapshot = alacritty.snapshot(10, 1);
    // The text and the cursor agree, but Alacritty keeps each emoji in a cell of its own.
    assert_eq!(snapshot.rows, term.rows());
    assert_eq!(snapshot.cursor, (6, 0));
    assert_eq!(alacritty.term.grid()[Line(0)][Column(2)].c, '\u{1f469}');
}

#[test]
fn quirk_halfwidth_sound_marks_take_a_cell() {
    let (ours, theirs) = both(5, 1, "\u{ff76}\u{ff9e}a".as_bytes());
    assert_eq!(ours.rows, theirs.rows);
    assert_eq!((ours.cursor, theirs.cursor), ((3, 0), (2, 0)));
}

#[test]
fn quirk_a_combining_mark_at_the_left_edge_is_dropped() {
    let (ours, theirs) = rows_of_both(3, 1, "ab\x1b[1;1H\u{301}".as_bytes());
    assert_eq!(ours, ["ab"]);
    assert_eq!(theirs, ["a\u{301}b"]);
}

#[test]
fn quirk_a_wide_character_without_autowrap_moves_left_to_fit() {
    let (ours, theirs) = rows_of_both(3, 1, "\x1b[?7lab字".as_bytes());
    assert_eq!(ours, ["a字"]);
    assert_eq!(theirs, ["ab"]);
}

#[test]
fn quirk_a_scroll_region_starting_past_the_screen_is_ignored() {
    let (ours, theirs) = both(3, 3, b"\x1b[5;9r\x1b[3;1Ha\nb");
    assert_eq!(ours.rows, ["", "a", " b"]);
    assert_eq!(theirs.rows, ["", "", "ab"]);
}

#[test]
fn quirk_resetting_any_mouse_mode_turns_reports_off() {
    let mut term = Emulator::with_profile(alacritty_profile(), 3, 1);
    term.feed(b"\x1b[?1000h\x1b[?1003h\x1b[?1003l").unwrap();
    assert!(!term.mode(1000));
    let mut alacritty = Alacritty::new(3, 1);
    alacritty.feed(b"\x1b[?1000h\x1b[?1003h\x1b[?1003l");
    // Setting 1003 replaced 1000 in both, so neither reports clicks now.
    assert!(!alacritty.term.mode().contains(TermMode::MOUSE_REPORT_CLICK));
    alacritty.feed(b"\x1b[?1000h\x1b[?1002l");
    term.feed(b"\x1b[?1000h\x1b[?1002l").unwrap();
    assert!(!term.mode(1000));
    assert!(alacritty.term.mode().contains(TermMode::MOUSE_REPORT_CLICK));
}
