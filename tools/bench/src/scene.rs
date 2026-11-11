//! Screens that look like Arko's. A transcript fills the top, then a rule, the status line, the input
//! box between two rules, and the colours are Arko's default theme painted over every cell.

use unicode_segmentation::UnicodeSegmentation;

use crate::model::{Color, Cursor, CursorShape, Frame, Mods, Style, width_of};

/// The theme's colours (macOS Classic Dark in Arko).
pub(crate) mod palette {
    use crate::model::Color;

    pub(crate) const BACKGROUND: Color = Color::Rgb(0x13, 0x13, 0x13);
    pub(crate) const TEXT: Color = Color::Rgb(0xde, 0xde, 0xde);
    pub(crate) const DIM: Color = Color::Rgb(0x9d, 0x9d, 0x9d);
    pub(crate) const ACCENT: Color = Color::Rgb(0x41, 0x9c, 0xff);
    pub(crate) const BORDER: Color = Color::Rgb(0x30, 0x30, 0x30);
    /// The user's messages.
    pub(crate) const USER: Color = Color::Rgb(0x23, 0x23, 0x23);
    pub(crate) const SUCCESS: Color = Color::Rgb(0x30, 0xd1, 0x58);
    pub(crate) const WARNING: Color = Color::Rgb(0xff, 0xc6, 0x00);
    /// A diff's removed and added lines, the error and success colours faint over the background.
    pub(crate) const REMOVED: Color = Color::Rgb(54, 28, 29);
    pub(crate) const ADDED: Color = Color::Rgb(23, 47, 29);
    pub(crate) const KEYWORD: Color = Color::Rgb(0x87, 0xb1, 0xf6);
    pub(crate) const STRING: Color = Color::Rgb(0xa3, 0xe0, 0x9f);
    pub(crate) const FUNCTION: Color = Color::Rgb(0xb3, 0xc5, 0xf3);
    pub(crate) const TYPE: Color = Color::Rgb(0xcb, 0xa6, 0xf7);
    pub(crate) const NUMBER: Color = Color::Rgb(0xcc, 0x9e, 0x00);
    pub(crate) const PROPERTY: Color = Color::Rgb(0xbc, 0xc4, 0xe0);
}

/// `fg` on the theme's background, which is how every cell of Arko's screen is styled.
pub(crate) const fn on_background(fg: Color) -> Style {
    Style {
        fg,
        bg: palette::BACKGROUND,
        ul: Color::Default,
        underline: crate::model::Underline::None,
        mods: Mods::NONE,
        link: 0,
    }
}

pub(crate) const fn text() -> Style {
    on_background(palette::TEXT)
}

pub(crate) const fn dim() -> Style {
    on_background(palette::DIM)
}

pub(crate) const fn accent() -> Style {
    on_background(palette::ACCENT)
}

/// A piece of text in one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) text: String,
    pub(crate) style: Style,
}

impl Span {
    pub(crate) fn new(text: impl Into<String>, style: Style) -> Span {
        Span {
            text: text.into(),
            style,
        }
    }
}

/// One row of the transcript.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) spans: Vec<Span>,
    /// A background across the whole row, as a user's message or a diff line has.
    pub(crate) band: Option<Color>,
}

impl Row {
    pub(crate) fn of(spans: Vec<Span>) -> Row {
        Row { spans, band: None }
    }

    pub(crate) fn blank() -> Row {
        Row::default()
    }
}

/// Splits `spans` into rows of at most `width` columns. It breaks after a space and after a wide
/// character, and inside a word only when the word is wider than a row.
pub(crate) fn wrap(spans: &[Span], width: usize) -> Vec<Vec<Span>> {
    let width = width.max(1);
    let mut units: Vec<(String, Style, usize)> = Vec::new();
    for span in spans {
        let mut unit = String::new();
        let mut unit_width = 0;
        for cluster in span.text.graphemes(true) {
            let cells = width_of(cluster);
            unit.push_str(cluster);
            unit_width += cells;
            if cluster == " " || cells >= 2 {
                units.push((std::mem::take(&mut unit), span.style, unit_width));
                unit_width = 0;
            }
        }
        if !unit.is_empty() {
            units.push((unit, span.style, unit_width));
        }
    }
    // A word wider than a row is cut into clusters.
    let units: Vec<(String, Style, usize)> = units
        .into_iter()
        .flat_map(|(unit, style, cells)| {
            if cells > width {
                unit.graphemes(true)
                    .map(|cluster| (cluster.to_string(), style, width_of(cluster)))
                    .collect()
            } else {
                vec![(unit, style, cells)]
            }
        })
        .collect();
    let mut rows: Vec<Vec<Span>> = vec![Vec::new()];
    let mut used = 0;
    for (unit, style, cells) in units {
        let needed = if unit.ends_with(' ') {
            cells - 1
        } else {
            cells
        };
        if used > 0 && used + needed > width {
            trim_end(rows.last_mut());
            rows.push(Vec::new());
            used = 0;
        }
        if let Some(row) = rows.last_mut() {
            match row.last_mut() {
                Some(last) if last.style == style => last.text.push_str(&unit),
                _ => row.push(Span::new(unit, style)),
            }
        }
        used += cells;
    }
    trim_end(rows.last_mut());
    rows
}

/// Drops the spaces at the end of a row.
fn trim_end(row: Option<&mut Vec<Span>>) {
    let Some(row) = row else { return };
    while let Some(last) = row.last_mut() {
        let kept = last.text.trim_end_matches(' ').len();
        last.text.truncate(kept);
        if last.text.is_empty() {
            row.pop();
        } else {
            break;
        }
    }
}

/// The columns `spans` take.
pub(crate) fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|span| width_of(&span.text)).sum()
}

/// A small deterministic generator, so a workload is the same on every run.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The next number, by splitmix64.
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number below `n`.
    pub(crate) fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0)
    }

    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let at = self.below(items.len());
        &items[at]
    }
}

/// The braille spinner's frames, each shown for 100 ms.
const SPINNER: [&str; 8] = ["⣷", "⣯", "⣟", "⡿", "⢿", "⣻", "⣽", "⣾"];
/// One sweep of the highlight, from before the word to past it.
const SWEEP_MS: u64 = 2000;
/// How far the highlight reaches each side of its middle, in columns.
const REACH: f64 = 3.0;
const WORKING: &str = "Working…";

/// The spinner's frame at `ms`.
pub(crate) fn spinner(ms: u64) -> &'static str {
    let at = usize::try_from(ms / 100).unwrap_or(0) % SPINNER.len();
    SPINNER[at]
}

/// How lit (0 to 1) column `col` of a word `width` columns wide is at `ms`.
fn shimmer(col: usize, width: usize, ms: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "columns and milliseconds are small"
    )]
    let (col, travel, phase) = (
        col as f64,
        width as f64 + 2.0 * REACH,
        (ms % SWEEP_MS) as f64 / SWEEP_MS as f64,
    );
    let distance = (col + 0.5 - (phase * travel - REACH)).abs();
    if distance >= REACH {
        0.0
    } else {
        f64::midpoint(1.0, (std::f64::consts::PI * distance / REACH).cos())
    }
}

/// `from` moved `amount` (0 to 1) of the way to `to`.
fn mix(from: Color, to: Color, amount: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (from, to) else {
        return from;
    };
    let channel = |a: u8, b: u8| {
        let value = f64::from(a) + (f64::from(b) - f64::from(a)) * amount;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the value is rounded and clamped to a byte first"
        )]
        let byte = value.round().clamp(0.0, 255.0) as u8;
        byte
    };
    Color::Rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2))
}

/// `seconds` as a clock, like `14s`, `2m14s`, `1h02m`.
pub(crate) fn clock(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

/// What the status line says.
#[derive(Clone, Debug)]
pub(crate) struct Status {
    /// Whether a turn is running, and for how many seconds.
    pub(crate) working: Option<u64>,
    /// Seconds the last turn took, shown while nothing runs.
    pub(crate) last_turn: u64,
    pub(crate) context_k: u32,
    pub(crate) limit_k: u32,
    pub(crate) cost: f64,
    /// Percent used of the plan's two windows, shown at the right.
    pub(crate) plan: (u32, u32),
}

impl Status {
    pub(crate) fn new() -> Status {
        Status {
            working: None,
            last_turn: 14,
            context_k: 84,
            limit_k: 200,
            cost: 0.42,
            plan: (23, 61),
        }
    }

    fn spans(&self) -> (Vec<Span>, Vec<Span>) {
        let sep = || Span::new(" · ", dim());
        let mut left = vec![Span::new(" ", text())];
        if let Some(seconds) = self.working {
            left.push(Span::new("● working", accent()));
            left.push(Span::new(format!(" {}", clock(seconds)), dim()));
        } else {
            left.push(Span::new(
                format!("last turn {}", clock(self.last_turn)),
                dim(),
            ));
        }
        left.push(sep());
        left.push(Span::new(
            format!("{}k/{}k", self.context_k, self.limit_k),
            dim(),
        ));
        left.push(sep());
        left.push(Span::new(format!("main: ${:.2}", self.cost), dim()));
        let used = |percent: u32| {
            if percent >= 80 {
                on_background(palette::WARNING)
            } else {
                dim()
            }
        };
        let right = vec![
            Span::new(format!("5h: {}%", self.plan.0), used(self.plan.0)),
            sep(),
            Span::new(format!("7d: {}%", self.plan.1), used(self.plan.1)),
        ];
        (left, right)
    }
}

/// How many input rows the box grows to before it scrolls.
const INPUT_LINES: usize = 8;

/// A screen's state. [`Screen::draw`] lays it out and paints a frame.
#[derive(Clone, Debug)]
pub(crate) struct Screen {
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// The transcript, oldest row first, laid out for this width.
    pub(crate) rows: Vec<Row>,
    /// How many rows the view is scrolled back from the end.
    pub(crate) scrolled_back: usize,
    /// While a turn waits on its model, the animation clock in milliseconds. The transcript then ends
    /// with a "Working…" line.
    pub(crate) working_ms: Option<u64>,
    pub(crate) status: Status,
    pub(crate) input: String,
}

impl Screen {
    pub(crate) fn new(width: u16, height: u16, rows: Vec<Row>) -> Screen {
        Screen {
            width,
            height,
            rows,
            scrolled_back: 0,
            working_ms: None,
            status: Status::new(),
            input: String::new(),
        }
    }

    /// The rows the transcript has room for.
    pub(crate) fn transcript_height(&self) -> usize {
        usize::from(self.height).saturating_sub(self.input_rows() + 4)
    }

    fn input_width(&self) -> usize {
        usize::from(self.width).saturating_sub(4).max(4)
    }

    /// The input box's text rows, the way the box wraps them.
    fn input_lines(&self) -> Vec<String> {
        let limit = self.input_width();
        let mut lines = vec![String::new()];
        let mut used = 0;
        for cluster in self.input.graphemes(true) {
            let cells = width_of(cluster);
            if used + cells > limit {
                lines.push(String::new());
                used = 0;
            }
            if let Some(line) = lines.last_mut() {
                line.push_str(cluster);
            }
            used += cells;
        }
        // The cursor after a full last row is on a row of its own.
        if used == limit {
            lines.push(String::new());
        }
        lines
    }

    fn input_rows(&self) -> usize {
        let most = INPUT_LINES.min(usize::from(self.height) / 4).max(1);
        self.input_lines().len().min(most)
    }

    pub(crate) fn draw(&self) -> Frame {
        let (width, height) = (self.width, self.height);
        let mut frame = Frame::filled(width, height, text());
        if height < 6 || width < 16 {
            return frame;
        }
        let rule = Span::new(
            "─".repeat(usize::from(width)),
            on_background(palette::BORDER),
        );
        // From the bottom, the input box between two rules, the status line, and a rule above it.
        let lines = self.input_lines();
        let most = INPUT_LINES.min(usize::from(height) / 4).max(1);
        let first = lines.len().saturating_sub(most);
        let shown = &lines[first..];
        let input_height = u16::try_from(shown.len()).unwrap_or(1);
        let input_top = height - input_height - 2;
        let status_y = input_top - 1;
        let rule_y = status_y - 1;
        draw_span(&mut frame, 0, input_top, &rule);
        draw_span(&mut frame, 0, height - 1, &rule);
        draw_span(&mut frame, 0, rule_y, &rule);
        frame.put_str(1, input_top + 1, "› ", accent(), width);
        let text_width = self.input_width();
        if self.input.is_empty() {
            let working = self.status.working.is_some();
            let placeholder = if working {
                "Add to the task"
            } else {
                "Message"
            };
            let at = frame.put_str(3, input_top + 1, placeholder, dim(), width);
            if !working {
                frame.put_str(at, input_top + 1, "  / for commands", dim(), width);
            }
        } else {
            for (n, line) in shown.iter().enumerate() {
                let y = input_top + 1 + u16::try_from(n).unwrap_or(0);
                frame.put_str(3, y, line, text(), width);
            }
        }
        let cursor_row = shown.len().saturating_sub(1);
        let cursor_column = shown.last().map_or(0, |line| width_of(line));
        frame.cursor = Cursor {
            x: 3 + u16::try_from(cursor_column.min(text_width)).unwrap_or(0),
            y: input_top + 1 + u16::try_from(cursor_row).unwrap_or(0),
            visible: true,
            shape: CursorShape::Bar,
        };
        self.draw_status(&mut frame, status_y);
        self.draw_transcript(&mut frame, usize::from(rule_y));
        frame
    }

    fn draw_status(&self, frame: &mut Frame, y: u16) {
        let (left, right) = self.status.spans();
        let mut x = 0;
        for span in &left {
            x = draw_span(frame, x, y, span);
        }
        let right_width = u16::try_from(spans_width(&right)).unwrap_or(0);
        let start = self.width.saturating_sub(right_width + 1);
        if start > x {
            let mut x = start;
            for span in &right {
                x = draw_span(frame, x, y, span);
            }
        }
    }

    fn draw_transcript(&self, frame: &mut Frame, height: usize) {
        // The rows of the transcript, then the "Working…" line and a blank one while a turn waits.
        let extra = if self.working_ms.is_some() { 2 } else { 0 };
        let total = self.rows.len() + extra;
        let end = total.saturating_sub(self.scrolled_back);
        let start = end.saturating_sub(height);
        for (offset, at) in (start..end).enumerate() {
            let y = u16::try_from(offset).unwrap_or(0);
            match self.rows.get(at) {
                Some(row) => draw_row(frame, y, row),
                None if at == self.rows.len() => {
                    if let Some(ms) = self.working_ms {
                        draw_working(frame, y, ms);
                    }
                }
                None => {}
            }
        }
    }
}

/// Draws `span` at `x`, `y` and returns the column after it.
fn draw_span(frame: &mut Frame, x: u16, y: u16, span: &Span) -> u16 {
    let limit = frame.width();
    frame.put_str(x, y, &span.text, span.style, limit)
}

fn draw_row(frame: &mut Frame, y: u16, row: &Row) {
    let width = frame.width();
    if let Some(band) = row.band {
        frame.set_style(y, 0, width, text().bg(band));
    }
    let mut x = 0;
    for span in &row.spans {
        let style = row.band.map_or(span.style, |band| span.style.bg(band));
        x = frame.put_str(x, y, &span.text, style, width);
    }
}

/// The "Working…" line, a spinner in the margin and a highlight sweeping over the label.
fn draw_working(frame: &mut Frame, y: u16, ms: u64) {
    let width = frame.width();
    frame.put_str(0, y, spinner(ms), dim(), width);
    let label_width = width_of(WORKING);
    for (col, cluster) in WORKING.graphemes(true).enumerate() {
        let lit = mix(palette::DIM, palette::TEXT, shimmer(col, label_width, ms));
        let x = 2 + u16::try_from(col).unwrap_or(0);
        frame.put_str(x, y, cluster, on_background(lit), width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;

    fn row_texts(rows: &[Vec<Span>]) -> Vec<String> {
        rows.iter()
            .map(|row| row.iter().map(|span| span.text.as_str()).collect())
            .collect()
    }

    fn plain(text: &str) -> Vec<Span> {
        vec![Span::new(text, dim())]
    }

    #[test]
    fn wraps_at_spaces() {
        let rows = wrap(&plain("one two three four five"), 9);
        assert_eq!(row_texts(&rows), ["one two", "three", "four five"]);
    }

    #[test]
    fn a_word_wider_than_the_row_is_cut() {
        let rows = wrap(&plain("abcdefghij"), 4);
        assert_eq!(row_texts(&rows), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wide_characters_break_between_each_other() {
        let rows = wrap(&plain("日本語のテスト"), 6);
        assert_eq!(row_texts(&rows), ["日本語", "のテス", "ト"]);
        assert!(rows.iter().all(|row| spans_width(row) <= 6));
    }

    #[test]
    fn spans_keep_their_styles_across_a_wrap() {
        let bold = text().bold();
        let rows = wrap(&[Span::new("aa ", dim()), Span::new("bb cc", bold)], 5);
        assert_eq!(row_texts(&rows), ["aa bb", "cc"]);
        assert_eq!(rows[0][1].style, bold);
        assert_eq!(rows[1][0].style, bold);
    }

    #[test]
    fn an_empty_text_is_one_empty_row() {
        assert_eq!(wrap(&[], 10), vec![Vec::<Span>::new()]);
    }

    #[test]
    fn the_generator_is_the_same_every_time() {
        let draw = |seed| {
            let mut rng = Rng::new(seed);
            (0..5).map(|_| rng.below(1000)).collect::<Vec<_>>()
        };
        assert_eq!(draw(3), draw(3));
        assert_ne!(draw(3), draw(4));
        let mut rng = Rng::new(1);
        assert!((0..100).all(|_| rng.below(7) < 7));
        assert_ne!(Rng::new(1).below(1 << 20), Rng::new(2).below(1 << 20));
    }

    #[test]
    fn clocks_read_like_arkos() {
        assert_eq!(clock(14), "14s");
        assert_eq!(clock(134), "2m14s");
        assert_eq!(clock(3720), "1h02m");
    }

    #[test]
    fn the_highlight_sweeps_across_the_word() {
        let lit = |ms: u64| -> Vec<bool> { (0..8).map(|col| shimmer(col, 8, ms) > 0.5).collect() };
        assert_eq!(lit(0), [false; 8]);
        assert_eq!(lit(SWEEP_MS - 1), [false; 8]);
        let first = |ms: u64| lit(ms).iter().position(|on| *on);
        assert!(first(SWEEP_MS / 4) < first(SWEEP_MS * 3 / 4));
        assert_eq!(lit(SWEEP_MS / 4), lit(SWEEP_MS + SWEEP_MS / 4));
    }

    fn sample(rows: usize) -> Screen {
        let rows = (0..rows)
            .map(|n| Row::of(vec![Span::new(format!("  line {n}"), text())]))
            .collect();
        Screen::new(80, 12, rows)
    }

    #[test]
    fn the_screen_stacks_transcript_rule_status_and_input() {
        let screen = sample(30);
        let frame = screen.draw();
        // 12 rows: seven of transcript, a rule, the status, and the input box between two rules.
        let texts: Vec<String> = (0..12).map(|y| frame.row_text(y)).collect();
        assert_eq!(texts[0], "  line 23");
        assert_eq!(texts[6], "  line 29");
        assert_eq!(texts[7], "─".repeat(80));
        assert!(
            texts[8].starts_with(" last turn 14s · 84k/200k · main: $0.42"),
            "{}",
            texts[8]
        );
        assert!(texts[8].ends_with("5h: 23% · 7d: 61%"), "{}", texts[8]);
        assert_eq!(texts[9], "─".repeat(80));
        assert_eq!(texts[10], " › Message  / for commands");
        assert_eq!(texts[11], "─".repeat(80));
        assert_eq!(
            frame.cursor,
            Cursor {
                x: 3,
                y: 10,
                visible: true,
                shape: CursorShape::Bar
            }
        );
    }

    #[test]
    fn every_cell_has_the_themes_colours() {
        let frame = sample(30).draw();
        assert!(frame.cells().iter().all(|cell| {
            matches!(cell.style.fg, Color::Rgb(..)) && matches!(cell.style.bg, Color::Rgb(..))
        }));
    }

    #[test]
    fn a_short_transcript_starts_at_the_top() {
        let frame = sample(2).draw();
        assert_eq!(frame.row_text(0), "  line 0");
        assert_eq!(frame.row_text(1), "  line 1");
        assert_eq!(frame.row_text(2), "");
    }

    #[test]
    fn scrolling_back_moves_the_view_up() {
        let mut screen = sample(30);
        screen.scrolled_back = 7;
        let frame = screen.draw();
        assert_eq!(frame.row_text(0), "  line 16");
        assert_eq!(frame.row_text(6), "  line 22");
        screen.scrolled_back = 1000;
        assert_eq!(screen.draw().row_text(0), "");
    }

    #[test]
    fn working_adds_a_line_with_a_spinner_and_a_lit_label() {
        let mut screen = sample(30);
        screen.working_ms = Some(2500);
        screen.status.working = Some(2);
        let frame = screen.draw();
        // The blank row after Working… is the last of the transcript.
        assert_eq!(frame.row_text(5), format!("{} Working…", spinner(2500)));
        assert_eq!(frame.row_text(6), "");
        assert!(frame.row_text(8).starts_with(" ● working 2s · 84k/200k"));
        assert_eq!(frame.cell(0, 5).unwrap().style, dim());
        let label: Vec<Color> = (2..10)
            .map(|x| frame.cell(x, 5).unwrap().style.fg)
            .collect();
        assert!(label.iter().all(|colour| matches!(colour, Color::Rgb(..))));
        assert!(
            label.iter().any(|colour| *colour != palette::DIM),
            "some of the label is lit"
        );
        assert_ne!(frame.cell(0, 5).unwrap().symbol, Symbol::Char(' '));
    }

    #[test]
    fn a_band_covers_the_whole_row() {
        let mut screen = sample(30);
        screen.rows[29].band = Some(palette::USER);
        let frame = screen.draw();
        assert!(
            frame
                .row(6)
                .iter()
                .all(|cell| cell.style.bg == palette::USER)
        );
        assert_eq!(frame.row_text(6), "  line 29");
        assert!(
            frame
                .row(5)
                .iter()
                .all(|cell| cell.style.bg == palette::BACKGROUND)
        );
    }

    #[test]
    fn typed_text_shows_after_the_prompt_and_moves_the_cursor() {
        let mut screen = sample(30);
        screen.input = "hi 日本".into();
        let frame = screen.draw();
        assert_eq!(frame.row_text(10), " › hi 日本");
        assert_eq!(frame.cursor.x, 3 + 7);
        assert_eq!(frame.cursor.y, 10);
    }

    #[test]
    fn a_long_input_grows_the_box_and_shrinks_the_transcript() {
        let mut screen = sample(30);
        screen.input = "x".repeat(76 + 5);
        let frame = screen.draw();
        // 76 columns of text per row, so two rows, and one fewer row of transcript.
        assert_eq!(screen.transcript_height(), 6);
        assert_eq!(frame.row_text(9), format!(" › {}", "x".repeat(76)));
        assert_eq!(frame.row_text(10), format!("   {}", "x".repeat(5)));
        assert_eq!((frame.cursor.x, frame.cursor.y), (8, 10));
        assert_eq!(frame.row_text(5), "  line 29");
    }

    #[test]
    fn the_input_box_stops_growing_and_keeps_the_cursor_in_sight() {
        let mut screen = Screen::new(40, 40, Vec::new());
        screen.input = "y".repeat(36 * 20);
        let frame = screen.draw();
        // 20 full rows of text show as eight, and the cursor is on the row after the last full one.
        assert_eq!(screen.input_rows(), 8);
        assert_eq!(frame.cursor.y, 38);
        assert_eq!(frame.cursor.x, 3);
    }

    #[test]
    fn a_tiny_screen_draws_blank() {
        let frame = Screen::new(10, 4, Vec::new()).draw();
        assert!(
            frame
                .cells()
                .iter()
                .all(|cell| cell.symbol == Symbol::Char(' '))
        );
    }
}
