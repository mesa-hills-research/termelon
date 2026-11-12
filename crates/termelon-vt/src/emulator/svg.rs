//! The screen as SVG, a still or an animation that loops.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::Duration;

use super::readback::{HTML_BG as BG, HTML_FG as FG, hex};
use super::{CursorShape, Emulator};
use crate::grid::Cell;
use crate::style::{Style, Underline};

type Rgb = (u8, u8, u8);

/// When an element shows, as intervals in milliseconds from the start of the loop.
type Times = [(u64, u64)];

/// A cell's width and height in the image, in pixels. Monospace fonts are about 0.6 em wide, which
/// makes the text 15 px.
const CELL_W: u32 = 9;
const CELL_H: u32 = 18;
const FONT_SIZE: u32 = 15;
/// From a cell's top to the text's baseline.
const BASELINE: u32 = 13;
/// Around the cells.
const MARGIN: u32 = 10;
const FONTS: &str =
    "ui-monospace,'SF Mono',Menlo,Consolas,'DejaVu Sans Mono','Liberation Mono',monospace";

impl Emulator {
    /// The screen as an SVG image, for docs and contact sheets. Colours follow xterm's default
    /// palette, the default colours are light grey on black, and the cursor shows in its shape.
    ///
    /// Each run of text is stretched to the cells it takes, so columns line up in any monospace
    /// font. Block elements, box drawing and braille are drawn as shapes, so they join up whatever
    /// font the viewer has.
    pub fn to_svg(&self) -> String {
        let (cols, rows) = self.size();
        let mut out = String::new();
        open(&mut out, cols, rows);
        for element in elements(self) {
            out.push_str(&element);
        }
        out.push_str("</g>\n</svg>\n");
        out
    }
}

/// Screens one after another, as an animated SVG that loops, each drawn as
/// [`Emulator::to_svg`] draws it.
///
/// Each part of the image is written once, with the times it shows, so a recording where little
/// changes from one screen to the next stays small. Browsers play it, in an `<img>` too, and a
/// viewer without SMIL animation shows the first screen. The image takes the first screen's size.
///
/// ```
/// use std::time::Duration;
/// use termelon_vt::{Emulator, SvgAnimation};
///
/// let mut term = Emulator::new(12, 1);
/// let mut animation = SvgAnimation::new();
/// for spinner in ["⠋", "⠙", "⠹"] {
///     term.feed(format!("\r{spinner} Working").as_bytes()).unwrap();
///     animation.push(&term, Duration::from_millis(80));
/// }
/// let svg = animation.to_svg();
/// assert_eq!(svg.matches("Working").count(), 1);
/// assert!(svg.contains("dur=\"240ms\""));
/// ```
#[derive(Debug, Clone, Default)]
pub struct SvgAnimation {
    size: Option<(u16, u16)>,
    /// Every element drawn, in the order first drawn.
    elements: Vec<String>,
    ids: HashMap<String, usize>,
    /// Each screen's elements, and how many milliseconds it shows.
    frames: Vec<(Vec<usize>, u64)>,
}

impl SvgAnimation {
    /// An animation of no screens.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds what `screen` shows now, for `duration`. A screen that shows the same as the one before
    /// makes that one last longer, and a duration under a millisecond adds nothing.
    pub fn push(&mut self, screen: &Emulator, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        if millis == 0 {
            return;
        }
        self.size.get_or_insert(screen.size());
        let ids: Vec<usize> = elements(screen)
            .into_iter()
            .map(|element| {
                let next = self.elements.len();
                *self.ids.entry(element.clone()).or_insert_with(|| {
                    self.elements.push(element);
                    next
                })
            })
            .collect();
        match self.frames.last_mut() {
            Some((last, shown)) if *last == ids => *shown = shown.saturating_add(millis),
            _ => self.frames.push((ids, millis)),
        }
    }

    /// The animation, which loops over every screen added, as an SVG image. With no screens, it's
    /// an empty image.
    pub fn to_svg(&self) -> String {
        let (cols, rows) = self.size.unwrap_or((0, 0));
        let total = self
            .frames
            .iter()
            .fold(0, |total: u64, (_, millis)| total.saturating_add(*millis));
        // (When each element shows, as intervals in milliseconds.)
        let mut shown: Vec<Vec<(u64, u64)>> = vec![Vec::new(); self.elements.len()];
        let mut start: u64 = 0;
        for (ids, millis) in &self.frames {
            let end = start.saturating_add(*millis);
            for &id in ids {
                match shown[id].last_mut() {
                    Some(last) if last.1 == start => last.1 = end,
                    _ => shown[id].push((start, end)),
                }
            }
            start = end;
        }
        // (Elements that show at the same times share one animation.)
        let mut groups: Vec<(&Times, Vec<usize>)> = Vec::new();
        let mut group_of: HashMap<&Times, usize> = HashMap::new();
        for (id, times) in shown.iter().enumerate() {
            let group = *group_of.entry(times).or_insert_with(|| {
                groups.push((times, Vec::new()));
                groups.len() - 1
            });
            groups[group].1.push(id);
        }
        let mut out = String::new();
        open(&mut out, cols, rows);
        for (times, ids) in groups {
            let always = times == [(0, total)];
            if !always {
                animate(&mut out, times, total);
            }
            for id in ids {
                out.push_str(&self.elements[id]);
            }
            if !always {
                out.push_str("</g>\n");
            }
        }
        out.push_str("</g>\n</svg>\n");
        out
    }
}

/// Opens a group that shows at `times` of a loop `total` milliseconds long.
fn animate(out: &mut String, times: &Times, total: u64) {
    let mut values = Vec::new();
    let mut key_times = Vec::new();
    if times.first().is_some_and(|(start, _)| *start > 0) {
        values.push("hidden");
        key_times.push(fraction(0, total));
    }
    for &(start, end) in times {
        values.push("visible");
        key_times.push(fraction(start, total));
        if end < total {
            values.push("hidden");
            key_times.push(fraction(end, total));
        }
    }
    let _ = writeln!(
        out,
        "<g visibility=\"{}\"><animate attributeName=\"visibility\" values=\"{}\" keyTimes=\"{}\" \
         dur=\"{total}ms\" calcMode=\"discrete\" repeatCount=\"indefinite\"/>",
        values.first().unwrap_or(&"hidden"),
        values.join(";"),
        key_times.join(";"),
    );
}

/// `part` of `total` as a decimal fraction with at most five digits.
fn fraction(part: u64, total: u64) -> String {
    let scaled = u128::from(part) * 100_000 / u128::from(total.max(1));
    if scaled == 0 {
        return "0".to_owned();
    }
    if scaled >= 100_000 {
        return "1".to_owned();
    }
    format!("0.{scaled:05}").trim_end_matches('0').to_owned()
}

/// Starts the image of a screen of `cols` by `rows`, with its background, and opens the group the
/// cells go in.
fn open(out: &mut String, cols: u16, rows: u16) {
    let width = u32::from(cols) * CELL_W + 2 * MARGIN;
    let height = u32::from(rows) * CELL_H + 2 * MARGIN;
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\" font-family=\"{FONTS}\" font-size=\"{FONT_SIZE}\">\n\
         <rect width=\"{width}\" height=\"{height}\" rx=\"6\" fill=\"{}\"/>\n\
         <g transform=\"translate({MARGIN} {MARGIN})\" fill=\"{}\">\n",
        hex(BG),
        hex(FG),
    );
}

/// The parts of a screen's image, in row order, from runs of cells in one style and the cell the
/// cursor is on, as [`draw_run`] makes them. They can be drawn in any order.
fn elements(screen: &Emulator) -> Vec<String> {
    let (_, rows) = screen.size();
    let cursor = screen.cursor();
    let cursor_at = cursor
        .visible
        .then(|| (screen.grid().leader(cursor.x, cursor.y), cursor.y));
    let mut elements = Vec::new();
    for y in 0..rows {
        let mut x = 0;
        while let Some(first) = screen.cell(x, y) {
            let start = x;
            if cursor_at == Some((x, y)) {
                x = x.saturating_add(u16::from(first.width().max(1)));
                draw_cursor(&mut elements, cursor.shape, (start, x), y, first);
            } else {
                let mut cells = Vec::new();
                while let Some(cell) = screen.cell(x, y)
                    && cell.style() == first.style()
                    && cursor_at != Some((x, y))
                {
                    if !cell.is_continuation() {
                        cells.push((x, cell));
                    }
                    x += 1;
                }
                draw_run(
                    &mut elements,
                    y,
                    (start, x),
                    &cells,
                    &Paint::of(first.style()),
                );
            }
        }
    }
    elements
}

/// Draws the cell from column `span.0` to `span.1` of row `y` with the cursor on it, in `shape`. A
/// block shows the cell in reverse.
fn draw_cursor(
    pieces: &mut Vec<String>,
    shape: CursorShape,
    span: (u16, u16),
    y: u16,
    cell: &Cell,
) {
    let paint = Paint::of(cell.style());
    let (left, top) = (u32::from(span.0) * CELL_W, u32::from(y) * CELL_H);
    let width = u32::from(span.1 - span.0) * CELL_W;
    match shape {
        CursorShape::Bar | CursorShape::Underline => {
            draw_run(pieces, y, span, &[(span.0, cell)], &paint);
            let mut bar = String::new();
            if shape == CursorShape::Bar {
                rect(&mut bar, (left, top, 2, CELL_H), paint.fg);
            } else {
                rect(&mut bar, (left, top + CELL_H - 2, width, 2), paint.fg);
            }
            pieces.push(bar);
        }
        CursorShape::Block | CursorShape::Default => {
            let reversed = Paint {
                fg: paint.bg.unwrap_or(BG),
                bg: Some(paint.fg),
                ..paint
            };
            draw_run(pieces, y, span, &[(span.0, cell)], &reversed);
        }
    }
}

/// The colours and attributes a cell is drawn with.
#[derive(Clone, Copy)]
struct Paint {
    fg: Rgb,
    /// `None` for the default background, which the image's background already shows.
    bg: Option<Rgb>,
    bold: bool,
    italic: bool,
    hidden: bool,
    underline: Underline,
    underline_color: Rgb,
    strike: bool,
    overline: bool,
}

impl Paint {
    fn of(style: &Style) -> Paint {
        let mut fg = style.fg.rgb().unwrap_or(FG);
        let mut bg = style.bg.rgb();
        if style.reverse {
            (fg, bg) = (bg.unwrap_or(BG), Some(fg));
        }
        if style.dim {
            fg = mix(fg, bg.unwrap_or(BG));
        }
        Paint {
            fg,
            bg,
            bold: style.bold,
            italic: style.italic,
            hidden: style.hidden,
            underline: style.underline,
            underline_color: style.underline_color.rgb().unwrap_or(fg),
            strike: style.strike,
            overline: style.overline,
        }
    }
}

/// Halfway between two colours.
fn mix(a: Rgb, b: Rgb) -> Rgb {
    (
        u8::midpoint(a.0, b.0),
        u8::midpoint(a.1, b.1),
        u8::midpoint(a.2, b.2),
    )
}

/// Draws the cells of row `y` from column `span.0` to `span.1`, which `cells` lists without their
/// continuation cells, as pieces that may be drawn in any order. A run with a background is one
/// piece. One without is a piece for each stretch of text or of shapes, so a spinner beside a
/// label in its style changes alone.
fn draw_run(
    pieces: &mut Vec<String>,
    y: u16,
    span: (u16, u16),
    cells: &[(u16, &Cell)],
    paint: &Paint,
) {
    let top = u32::from(y) * CELL_H;
    let left = u32::from(span.0) * CELL_W;
    let width = u32::from(span.1 - span.0) * CELL_W;
    let mut piece = String::new();
    let mut end_piece = |piece: &mut String| {
        if paint.bg.is_none() && !piece.is_empty() {
            pieces.push(std::mem::take(piece));
        }
    };
    if let Some(bg) = paint.bg {
        rect(&mut piece, (left, top, width, CELL_H), bg);
    }
    if !paint.hidden {
        let mut shapes = Shapes::default();
        let mut text = TextRun::default();
        let mut i = 0;
        while i < cells.len() {
            let (x, cell) = cells[i];
            let mut chars = cell.text().chars();
            let shape = match (chars.next(), chars.next()) {
                (Some(c), None) => Shape::of(c),
                _ => None,
            };
            if let Some(shape) = shape {
                text.write(&mut piece, top, paint);
                end_piece(&mut piece);
                // (A row of the same shape is drawn as one, so its parts join.)
                let same = cells[i..]
                    .iter()
                    .take_while(|(_, other)| other.text() == cell.text())
                    .count();
                shapes.add(shape, (x, u16::try_from(same).unwrap_or(u16::MAX)), top);
                i += same;
            } else {
                shapes.write(&mut piece, paint.fg);
                end_piece(&mut piece);
                // (Two spaces end the text, so the columns hold in a viewer that collapses spaces.)
                if cell.text() == " " && text.text.ends_with(' ') {
                    text.write(&mut piece, top, paint);
                    end_piece(&mut piece);
                } else {
                    text.push(x, cell);
                }
                i += 1;
            }
        }
        text.write(&mut piece, top, paint);
        shapes.write(&mut piece, paint.fg);
        end_piece(&mut piece);
        let mut line = |offset: u32, color: Rgb| {
            rect(&mut piece, (left, top + offset, width, 1), color);
        };
        match paint.underline {
            Underline::None => {}
            Underline::Double => {
                line(CELL_H - 4, paint.underline_color);
                line(CELL_H - 2, paint.underline_color);
            }
            _ => line(CELL_H - 2, paint.underline_color),
        }
        if paint.strike {
            line(CELL_H / 2, paint.fg);
        }
        if paint.overline {
            line(0, paint.fg);
        }
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
}

fn rect(out: &mut String, (x, y, width, height): (u32, u32, u32, u32), color: Rgb) {
    let _ = writeln!(
        out,
        "<rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" fill=\"{}\"/>",
        hex(color)
    );
}

/// Cells of text that run on, from the first's column to the end of the last.
#[derive(Default)]
struct TextRun {
    start: u16,
    end: u16,
    text: String,
}

impl TextRun {
    /// Adds a cell, unless it's a space the text would start with.
    fn push(&mut self, x: u16, cell: &Cell) {
        if self.text.is_empty() {
            if cell.text() == " " {
                return;
            }
            self.start = x;
        }
        self.text.push_str(cell.text());
        self.end = x.saturating_add(u16::from(cell.width()));
    }

    /// Writes the text, without the space it may end with, stretched to its cells, and empties
    /// it.
    fn write(&mut self, out: &mut String, top: u32, paint: &Paint) {
        let text = std::mem::take(&mut self.text);
        let trimmed = text.trim_end_matches(' ');
        if trimmed.is_empty() {
            return;
        }
        let trailing = text.len() - trimmed.len();
        let start = u32::from(self.start);
        let end = u32::from(self.end).saturating_sub(u32::try_from(trailing).unwrap_or(0));
        let _ = write!(
            out,
            "<text x=\"{}\" y=\"{}\"",
            start * CELL_W,
            top + BASELINE
        );
        if trimmed.chars().nth(1).is_some() {
            let _ = write!(out, " textLength=\"{}\"", (end - start) * CELL_W);
        }
        if paint.fg != FG {
            let _ = write!(out, " fill=\"{}\"", hex(paint.fg));
        }
        if paint.bold {
            out.push_str(" font-weight=\"bold\"");
        }
        if paint.italic {
            out.push_str(" font-style=\"italic\"");
        }
        out.push('>');
        for c in trimmed.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                c => out.push(c),
            }
        }
        out.push_str("</text>\n");
    }
}

/// A character drawn as a shape rather than from the font.
#[derive(Clone, Copy)]
enum Shape {
    /// A block element, as rectangles in eighths of the cell (left, top, right, bottom).
    Block(&'static [[u8; 4]]),
    /// A shade, the whole cell at this many quarters of the colour.
    Shade(u8),
    /// A box-drawing line, with arms of [`UP`], [`RIGHT`], [`DOWN`] and [`LEFT`].
    Line { arms: u8, heavy: bool, round: bool },
    /// A braille pattern, its dots as the bits of U+2800 + n.
    Braille(u8),
}

const UP: u8 = 1;
const RIGHT: u8 = 2;
const DOWN: u8 = 4;
const LEFT: u8 = 8;

impl Shape {
    fn of(c: char) -> Option<Shape> {
        const UPPER_LEFT: [u8; 4] = [0, 0, 4, 4];
        const UPPER_RIGHT: [u8; 4] = [4, 0, 8, 4];
        const LOWER_LEFT: [u8; 4] = [0, 4, 4, 8];
        const LOWER_RIGHT: [u8; 4] = [4, 4, 8, 8];
        let line = |arms, heavy| {
            Some(Shape::Line {
                arms,
                heavy,
                round: false,
            })
        };
        let round = |arms| {
            Some(Shape::Line {
                arms,
                heavy: false,
                round: true,
            })
        };
        let block = |rects| Some(Shape::Block(rects));
        match c {
            '▀' => block(&[[0, 0, 8, 4]]),
            '▁' => block(&[[0, 7, 8, 8]]),
            '▂' => block(&[[0, 6, 8, 8]]),
            '▃' => block(&[[0, 5, 8, 8]]),
            '▄' => block(&[[0, 4, 8, 8]]),
            '▅' => block(&[[0, 3, 8, 8]]),
            '▆' => block(&[[0, 2, 8, 8]]),
            '▇' => block(&[[0, 1, 8, 8]]),
            '█' => block(&[[0, 0, 8, 8]]),
            '▉' => block(&[[0, 0, 7, 8]]),
            '▊' => block(&[[0, 0, 6, 8]]),
            '▋' => block(&[[0, 0, 5, 8]]),
            '▌' => block(&[[0, 0, 4, 8]]),
            '▍' => block(&[[0, 0, 3, 8]]),
            '▎' => block(&[[0, 0, 2, 8]]),
            '▏' => block(&[[0, 0, 1, 8]]),
            '▐' => block(&[[4, 0, 8, 8]]),
            '░' => Some(Shape::Shade(1)),
            '▒' => Some(Shape::Shade(2)),
            '▓' => Some(Shape::Shade(3)),
            '▔' => block(&[[0, 0, 8, 1]]),
            '▕' => block(&[[7, 0, 8, 8]]),
            '▖' => block(&[LOWER_LEFT]),
            '▗' => block(&[LOWER_RIGHT]),
            '▘' => block(&[UPPER_LEFT]),
            '▙' => block(&[UPPER_LEFT, LOWER_LEFT, LOWER_RIGHT]),
            '▚' => block(&[UPPER_LEFT, LOWER_RIGHT]),
            '▛' => block(&[UPPER_LEFT, UPPER_RIGHT, LOWER_LEFT]),
            '▜' => block(&[UPPER_LEFT, UPPER_RIGHT, LOWER_RIGHT]),
            '▝' => block(&[UPPER_RIGHT]),
            '▞' => block(&[UPPER_RIGHT, LOWER_LEFT]),
            '▟' => block(&[UPPER_RIGHT, LOWER_LEFT, LOWER_RIGHT]),
            '─' => line(LEFT | RIGHT, false),
            '━' => line(LEFT | RIGHT, true),
            '│' => line(UP | DOWN, false),
            '┃' => line(UP | DOWN, true),
            '┌' => line(DOWN | RIGHT, false),
            '┏' => line(DOWN | RIGHT, true),
            '┐' => line(DOWN | LEFT, false),
            '┓' => line(DOWN | LEFT, true),
            '└' => line(UP | RIGHT, false),
            '┗' => line(UP | RIGHT, true),
            '┘' => line(UP | LEFT, false),
            '┛' => line(UP | LEFT, true),
            '├' => line(UP | DOWN | RIGHT, false),
            '┣' => line(UP | DOWN | RIGHT, true),
            '┤' => line(UP | DOWN | LEFT, false),
            '┫' => line(UP | DOWN | LEFT, true),
            '┬' => line(DOWN | LEFT | RIGHT, false),
            '┳' => line(DOWN | LEFT | RIGHT, true),
            '┴' => line(UP | LEFT | RIGHT, false),
            '┻' => line(UP | LEFT | RIGHT, true),
            '┼' => line(UP | DOWN | LEFT | RIGHT, false),
            '╋' => line(UP | DOWN | LEFT | RIGHT, true),
            '╴' => line(LEFT, false),
            '╵' => line(UP, false),
            '╶' => line(RIGHT, false),
            '╷' => line(DOWN, false),
            '╸' => line(LEFT, true),
            '╹' => line(UP, true),
            '╺' => line(RIGHT, true),
            '╻' => line(DOWN, true),
            '╭' => round(DOWN | RIGHT),
            '╮' => round(DOWN | LEFT),
            '╯' => round(UP | LEFT),
            '╰' => round(UP | RIGHT),
            '\u{2800}'..='\u{28ff}' => u8::try_from(u32::from(c) - 0x2800).ok().map(Shape::Braille),
            _ => None,
        }
    }
}

/// The shapes of a run, gathered so that each kind is one path.
#[derive(Default)]
struct Shapes {
    blocks: String,
    shades: [String; 3],
    lines: [String; 2],
    dots: String,
}

impl Shapes {
    /// Adds `shape` in `count` cells from column `x`, on the row whose top is `top`.
    fn add(&mut self, shape: Shape, (x, count): (u16, u16), top: u32) {
        let cell = |i: u16| Place {
            left: f64::from((u32::from(x) + u32::from(i)) * CELL_W),
            top: f64::from(top),
        };
        let span = f64::from(u32::from(count) * CELL_W);
        match shape {
            Shape::Block(rects) => {
                for &[x0, y0, x1, y1] in rects {
                    let eighths = |n: u8, size: u32| f64::from(n) * f64::from(size) / 8.0;
                    let (y, height) = (eighths(y0, CELL_H), eighths(y1 - y0, CELL_H));
                    if [x0, x1] == [0, 8] {
                        path_rect(&mut self.blocks, cell(0).at(0.0, y), span, height);
                    } else {
                        for i in 0..count {
                            let corner = cell(i).at(eighths(x0, CELL_W), y);
                            path_rect(&mut self.blocks, corner, eighths(x1 - x0, CELL_W), height);
                        }
                    }
                }
            }
            Shape::Shade(quarters) => {
                let path = &mut self.shades[usize::from(quarters - 1)];
                path_rect(path, cell(0).at(0.0, 0.0), span, f64::from(CELL_H));
            }
            Shape::Line { arms, heavy, round } => {
                let path = &mut self.lines[usize::from(heavy)];
                if arms == LEFT | RIGHT {
                    let (_, y) = cell(0).middle();
                    let _ = write!(path, "M{} {y}h{span}", cell(0).left);
                    return;
                }
                for i in 0..count {
                    cell(i).line(path, arms, round);
                }
            }
            Shape::Braille(bits) => {
                for i in 0..count {
                    cell(i).braille(&mut self.dots, bits);
                }
            }
        }
    }

    /// Writes the shapes added in `color`, and empties them.
    fn write(&mut self, out: &mut String, color: Rgb) {
        let shapes = std::mem::take(self);
        let fill = if color == FG {
            String::new()
        } else {
            format!(" fill=\"{}\"", hex(color))
        };
        if !shapes.blocks.is_empty() {
            let _ = writeln!(
                out,
                "<path{fill} shape-rendering=\"crispEdges\" d=\"{}\"/>",
                shapes.blocks
            );
        }
        for (quarters, d) in shapes.shades.iter().enumerate() {
            if !d.is_empty() {
                let opacity = ["0.25", "0.5", "0.75"][quarters];
                let _ = writeln!(
                    out,
                    "<path{fill} fill-opacity=\"{opacity}\" shape-rendering=\"crispEdges\" \
                     d=\"{d}\"/>"
                );
            }
        }
        for (heavy, d) in shapes.lines.iter().enumerate() {
            if !d.is_empty() {
                let width = if heavy == 1 { 2 } else { 1 };
                let _ = writeln!(
                    out,
                    "<path fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\" \
                     stroke-linecap=\"square\" d=\"{d}\"/>",
                    hex(color)
                );
            }
        }
        if !shapes.dots.is_empty() {
            let _ = writeln!(out, "<path{fill} d=\"{}\"/>", shapes.dots);
        }
    }
}

/// Adds a rectangle to path data.
fn path_rect(path: &mut String, (x, y): (f64, f64), width: f64, height: f64) {
    let _ = write!(path, "M{x} {y}h{width}v{height}h{}z", -width);
}

/// A cell's place in the image, its top left corner.
#[derive(Clone, Copy)]
struct Place {
    left: f64,
    top: f64,
}

impl Place {
    fn at(self, x: f64, y: f64) -> (f64, f64) {
        (self.left + x, self.top + y)
    }

    /// The middle, on a pixel's centre so a line one pixel wide is sharp.
    fn middle(self) -> (f64, f64) {
        self.at(
            (width() / 2.0).floor() + 0.5,
            (height() / 2.0).floor() + 0.5,
        )
    }

    /// Adds a box-drawing line with `arms` to path data, out from the middle to the edges, or
    /// rounding the corner where `round`.
    fn line(self, path: &mut String, arms: u8, round: bool) {
        let (x, y) = self.middle();
        let edge_y = |arm| self.top + if arm == UP { 0.0 } else { height() };
        let edge_x = |arm| self.left + if arm == LEFT { 0.0 } else { width() };
        if round {
            let (vertical, horizontal) = (arms & (UP | DOWN), arms & (LEFT | RIGHT));
            let radius = width() / 2.0;
            let dy = if vertical == UP { -radius } else { radius };
            let dx = if horizontal == LEFT { -radius } else { radius };
            let _ = write!(
                path,
                "M{x} {}V{}Q{x} {y} {} {y}H{}",
                edge_y(vertical),
                y + dy,
                x + dx,
                edge_x(horizontal)
            );
            return;
        }
        for arm in [UP, DOWN] {
            if arms & arm != 0 {
                let _ = write!(path, "M{x} {y}V{}", edge_y(arm));
            }
        }
        for arm in [LEFT, RIGHT] {
            if arms & arm != 0 {
                let _ = write!(path, "M{x} {y}H{}", edge_x(arm));
            }
        }
    }

    /// Adds the dots of the braille pattern U+2800 + `bits` to path data, as circles.
    fn braille(self, path: &mut String, bits: u8) {
        // (Dots 1 to 8 are bits 0 to 7, down the left column, down the right, then the bottom
        // row.)
        const DOTS: [(u8, u8); 8] = [
            (0, 0),
            (0, 1),
            (0, 2),
            (1, 0),
            (1, 1),
            (1, 2),
            (0, 3),
            (1, 3),
        ];
        let radius = width() / 6.0;
        for (bit, &(column, row)) in DOTS.iter().enumerate() {
            if bits & (1 << bit) != 0 {
                let x = width() * f64::from(1 + 2 * column) / 4.0 - radius;
                let y = height() * f64::from(1 + 2 * row) / 8.0;
                let (x, y) = self.at(x, y);
                let diameter = 2.0 * radius;
                let _ = write!(
                    path,
                    "M{x} {y}a{radius} {radius} 0 1 0 {diameter} 0a{radius} {radius} 0 1 0 {} 0",
                    -diameter
                );
            }
        }
    }
}

fn width() -> f64 {
    f64::from(CELL_W)
}

fn height() -> f64 {
    f64::from(CELL_H)
}

#[cfg(test)]
mod tests {
    use super::fraction;

    #[test]
    fn fractions_have_at_most_five_digits() {
        assert_eq!(fraction(0, 320), "0");
        assert_eq!(fraction(80, 320), "0.25");
        assert_eq!(fraction(1, 3), "0.33333");
        assert_eq!(fraction(320, 320), "1");
        assert_eq!(fraction(5, 0), "1");
    }
}
