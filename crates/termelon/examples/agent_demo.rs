//! A coding agent's screen, with Termelon's mascot. It plays a made-up session on a loop, where a
//! request is typed and sent, a reply streams in, and two tools run. q or Ctrl+C quits.
//!
//! ```sh
//! cargo run --example agent_demo
//! ```
//!
//! With the `testing` feature, `--capture DIR` plays the loop in a test terminal instead, and
//! writes a still of it and an animation to `DIR` as SVG.
//!
//! ```sh
//! cargo run --example agent_demo --features testing -- --capture docs/images
//! ```

#![cfg_attr(
    not(any(unix, windows, feature = "testing")),
    expect(dead_code, reason = "the terminal is on Unix and Windows")
)]

use std::error::Error;
use std::path::Path;
use std::time::Duration;

use termelon::caps::{Capabilities, ColorDepth, GlyphSupport};
use termelon::layout::{Constraint, Layout, Margin, Rect};
use termelon::style::{Color, Modifier, Style};
use termelon::text::{self, Line, Span};
use termelon::widgets::{Block, BorderSet, Padding, Paragraph, Wrap};
use termelon::{Buffer, Frame};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => play(),
        [flag, dir] if flag == "--capture" => capture(Path::new(dir)),
        _ => Err("the arguments are none, or --capture and a directory".into()),
    }
}

/// Plays the script in the terminal until q or Ctrl+C.
#[cfg(any(unix, windows))]
fn play() -> Result<(), Box<dyn Error>> {
    use std::time::Instant;

    use termelon::widgets::Clear;
    use termelon::{Drawn, Event, KeyCode, KeyEventKind, KeyModifiers, Options, Terminal};

    // (Often enough for the fastest change, a character typed every 40 ms.)
    const TICK: Duration = Duration::from_millis(20);

    let mut terminal = Terminal::open(Options::default())?;
    let theme = Theme::for_terminal(terminal.capabilities());
    let mut events = terminal.events();
    let start = Instant::now();
    // (The scene on the screen, and where the parts that move are drawn.)
    let mut shown: Option<Scene> = None;
    let mut moving = Vec::new();
    loop {
        let scene = Scene::at(start.elapsed());
        let mut wait = TICK;
        if shown.as_ref() != Some(&scene) {
            // When only the mascot or the spinner changed, only their regions are drawn.
            let same_content = shown.as_ref().map(|shown| &shown.content) == Some(&scene.content);
            let drawn = if same_content {
                terminal.draw_regions(&moving, |frame| {
                    // (The regions start with the last frame's cells.)
                    for region in &moving {
                        frame.render_widget(Clear, *region);
                    }
                    draw(frame, &scene, &theme);
                })?
            } else {
                terminal.draw(|frame| moving = draw(frame, &scene, &theme))?
            };
            match drawn {
                Drawn::Written(_) => shown = Some(scene),
                // (After a resize, the next frame is drawn whole.)
                Drawn::FullFrameDue => {
                    shown = None;
                    continue;
                }
                // (The terminal is still taking the last frame, so try again soon.)
                Drawn::Held => wait = Duration::from_millis(10),
                _ => {}
            }
        }
        match events.next_timeout(wait)? {
            Some(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                let ctrl_c =
                    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                if key.code == KeyCode::Char('q') || ctrl_c {
                    break;
                }
            }
            Some(Event::Resize(_) | Event::Resumed) => shown = None,
            _ => {}
        }
    }
    Ok(terminal.close()?)
}

#[cfg(not(any(unix, windows)))]
fn play() -> Result<(), Box<dyn Error>> {
    Err("the terminal is on Unix and Windows only".into())
}

/// Plays the loop once in a test terminal of 80 by 24, and writes it to `agent-demo.svg` and a still
/// of a tool running to `agent-demo-still.svg`.
#[cfg(feature = "testing")]
fn capture(dir: &Path) -> Result<(), Box<dyn Error>> {
    use termelon::testing::{TestTerminal, vt};

    const STEP: Duration = Duration::from_millis(10);

    let term = TestTerminal::new(80, 24);
    let mut caps = term.capabilities().clone();
    caps.colors = ColorDepth::TrueColor;
    let mut term = term.with_caps(caps);
    let theme = Theme::for_terminal(term.capabilities());
    let still_at = Scene::start_of_tool("Run") + Duration::from_millis(3_300);
    let mut still = String::new();
    let mut animation = vt::SvgAnimation::new();
    let mut at = Duration::ZERO;
    while at < Scene::length() {
        let scene = Scene::at(at);
        term.draw(|frame| {
            draw(frame, &scene, &theme);
        })?;
        animation.push(term.emulator(), STEP);
        if still.is_empty() && at >= still_at {
            still = term.screen().to_svg();
        }
        at += STEP;
    }
    std::fs::write(dir.join("agent-demo.svg"), animation.to_svg())?;
    std::fs::write(dir.join("agent-demo-still.svg"), still)?;
    Ok(())
}

#[cfg(not(feature = "testing"))]
fn capture(_: &Path) -> Result<(), Box<dyn Error>> {
    Err("capturing needs the testing feature, as in --features testing".into())
}

/// The script, played on a loop.
const SCRIPT: &[Step] = &[
    Step::Pause(800),
    Step::Ask("Make the to-do list keep its items between runs."),
    Step::Reply(
        "I'll save the items to a file whenever the list changes, and read them back at start. \
         First, how the app holds them.",
    ),
    Step::Tool {
        name: "Read",
        input: "src/app.rs",
        output: "86 lines",
        millis: 900,
    },
    Step::Tool {
        name: "Run",
        input: "cargo test",
        output: "14 passed",
        millis: 4_200,
    },
    Step::Reply("Done. The list now saves to todo.json and loads it at start."),
    Step::Pause(3_000),
];

/// When the mascot blinks, in milliseconds into the loop, for `BLINK` each.
const BLINKS: [u64; 3] = [2_000, 7_400, 12_400];
const BLINK: u64 = 160;

/// Milliseconds for each character typed.
const TYPE_EVERY: u64 = 40;
/// Milliseconds for each word streamed.
const WORD_EVERY: u64 = 80;
/// Milliseconds for each frame of the spinner.
const SPIN_EVERY: u64 = 80;
/// Milliseconds of the pause after each step.
const AFTER: u64 = 300;

/// A step of the script.
enum Step {
    /// Nothing happens for this many milliseconds.
    Pause(u64),
    /// The request is typed in the input box and sent.
    Ask(&'static str),
    /// The reply streams in, word by word.
    Reply(&'static str),
    /// A tool runs for `millis` and gives its output.
    Tool {
        name: &'static str,
        input: &'static str,
        output: &'static str,
        millis: u64,
    },
}

impl Step {
    fn millis(&self) -> u64 {
        let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        match *self {
            Step::Pause(millis) => millis,
            Step::Ask(request) => count(request.chars().count()) * TYPE_EVERY + AFTER,
            Step::Reply(reply) => count(reply.split(' ').count()) * WORD_EVERY + AFTER,
            Step::Tool { millis, .. } => millis + AFTER,
        }
    }
}

/// What the screen shows at a moment of the loop.
#[derive(Debug, Clone, PartialEq)]
struct Scene {
    /// The transcript and the input box, which a full frame draws.
    content: Content,
    /// What changes in place, which a partial frame draws.
    motion: Motion,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Content {
    entries: Vec<Entry>,
    /// What's typed in the input box.
    input: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Entry {
    Request(&'static str),
    /// A reply, as much of it as has streamed in.
    Reply(&'static str),
    Tool {
        name: &'static str,
        input: &'static str,
        output: &'static str,
        running: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Motion {
    /// The spinner's frame and the running tool's seconds, while one runs.
    spinner: usize,
    seconds: u64,
    /// Whether the mascot's winking eye is open.
    eye_open: bool,
}

impl Scene {
    /// How long the loop takes.
    fn length() -> Duration {
        Duration::from_millis(SCRIPT.iter().map(Step::millis).sum())
    }

    /// When the tool `name` starts.
    #[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "for capturing"))]
    fn start_of_tool(name: &str) -> Duration {
        let before = SCRIPT
            .iter()
            .take_while(|step| !matches!(step, Step::Tool { name: n, .. } if *n == name))
            .map(Step::millis)
            .sum();
        Duration::from_millis(before)
    }

    /// The scene `elapsed` after the loop first started.
    fn at(elapsed: Duration) -> Scene {
        let length = Scene::length().as_millis().max(1);
        let now = u64::try_from(elapsed.as_millis() % length).unwrap_or(0);
        let mut content = Content::default();
        let mut motion = Motion {
            eye_open: BLINKS.iter().any(|&at| (at..at + BLINK).contains(&now)),
            ..Motion::default()
        };
        let mut start = 0;
        for step in SCRIPT {
            let Some(into) = now.checked_sub(start) else {
                break;
            };
            match *step {
                Step::Pause(_) => {}
                Step::Ask(request) => {
                    let typed = usize::try_from(into / TYPE_EVERY).unwrap_or(usize::MAX);
                    if into < step.millis() {
                        content.input = prefix(request, typed);
                    } else {
                        content.entries.push(Entry::Request(request));
                    }
                }
                Step::Reply(reply) => {
                    let words = usize::try_from(into / WORD_EVERY + 1).unwrap_or(usize::MAX);
                    content.entries.push(Entry::Reply(words_of(reply, words)));
                }
                Step::Tool {
                    name,
                    input,
                    output,
                    millis,
                } => {
                    let running = into < millis;
                    if running {
                        motion.spinner = usize::try_from(into / SPIN_EVERY).unwrap_or(0);
                        motion.seconds = into / 1_000;
                    }
                    content.entries.push(Entry::Tool {
                        name,
                        input,
                        output,
                        running,
                    });
                }
            }
            start += step.millis();
        }
        Scene { content, motion }
    }
}

/// The first `count` characters of `text`.
fn prefix(text: &str, count: usize) -> &str {
    text.char_indices()
        .nth(count)
        .map_or(text, |(end, _)| &text[..end])
}

/// The first `count` words of `text`.
fn words_of(text: &str, count: usize) -> &str {
    let end: usize = text.split_inclusive(' ').take(count).map(str::len).sum();
    text[..end].trim_end()
}

/// What the screen is drawn with, the characters for a font with box drawing and braille or for
/// one that may lack them, and the mascot's colours.
struct Theme {
    border: BorderSet,
    spinner: &'static [&'static str],
    request: &'static str,
    reply: &'static str,
    done: &'static str,
    separator: &'static str,
    flesh: Color,
    rind: Color,
}

impl Theme {
    fn for_terminal(caps: &Capabilities) -> Theme {
        // (The library brings these down to what the terminal shows, bright red and green where
        // it has 16 colours.)
        let (flesh, rind) = (Color::Rgb(0xef, 0x4a, 0x5c), Color::Rgb(0x1c, 0x91, 0x60));
        match caps.glyphs {
            GlyphSupport::Limited => Theme {
                border: BorderSet::ASCII,
                spinner: &["|", "/", "-", "\\"],
                request: ">",
                reply: "*",
                done: "+",
                separator: "-",
                flesh,
                rind,
            },
            _ => Theme {
                border: BorderSet::ROUNDED,
                spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
                request: "›",
                reply: "●",
                done: "✓",
                separator: "·",
                flesh,
                rind,
            },
        }
    }
}

const DIM: Style = Style::new().fg(Color::DarkGray);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

/// Draws the scene, and gives back the regions of the parts that move, the mascot and the running
/// tool's row.
fn draw(frame: &mut Frame<'_>, scene: &Scene, theme: &Theme) -> Vec<Rect> {
    let area = frame.area();
    let face = Face::to_fit(area);
    let header_height = face.map_or(2, Face::height) + 1;
    let [header, transcript, input, status] = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);
    let mut moving = Vec::new();
    moving.extend(draw_header(
        frame,
        header,
        face,
        scene.motion.eye_open,
        theme,
    ));
    let transcript = transcript.inner(Margin::new(2, 0));
    moving.extend(draw_transcript(frame, transcript, scene, theme));
    draw_input(frame, input, scene.content.input, theme);
    let quit = Line::from(vec![Span::raw(" q"), Span::styled(" quit", DIM)]);
    frame.render_widget(quit, status);
    frame.render_widget(Line::styled("melon-1 ", DIM).right_aligned(), status);
    moving
}

/// Draws the mascot and the program's name, and gives back where the mascot is.
fn draw_header(
    frame: &mut Frame<'_>,
    area: Rect,
    face: Option<&Face>,
    eye_open: bool,
    theme: &Theme,
) -> Option<Rect> {
    let title = Line::from(vec![
        Span::styled("Termelon", BOLD),
        Span::styled(" agent demo", DIM),
    ]);
    let place = Line::styled("~/projects/orchard", DIM);
    let Some(face) = face else {
        let [first, second] =
            Layout::vertical([Constraint::Length(1); 2]).areas(area.inner(Margin::new(2, 0)));
        frame.render_widget(title, first);
        frame.render_widget(place, second);
        return None;
    };
    let face_area = Rect::new(area.x + 2, area.y, face.width(), face.height()).intersection(area);
    face.draw(frame.buffer_mut(), face_area, eye_open, theme);
    let text_x = face_area.right() + 3;
    let text_y = area.y + face.height() / 2 - 1;
    let width = area.right().saturating_sub(text_x);
    frame.render_widget(
        title,
        Rect::new(text_x, text_y, width, 1).intersection(area),
    );
    frame.render_widget(
        place,
        Rect::new(text_x, text_y + 1, width, 1).intersection(area),
    );
    Some(face_area)
}

/// An entry of the transcript as drawn, a marker in the margin with its text beside it, wrapped,
/// after `gap` blank rows.
struct Item<'a> {
    gap: u16,
    marker: Span<'a>,
    text: Line<'a>,
    rows: u16,
    running: bool,
}

/// Draws the transcript, scrolled to its end, and gives back the running tool's row.
fn draw_transcript(
    frame: &mut Frame<'_>,
    area: Rect,
    scene: &Scene,
    theme: &Theme,
) -> Option<Rect> {
    let text_width = area.width.saturating_sub(2);
    let mut items: Vec<Item<'_>> = Vec::new();
    let mut after_tool = false;
    for entry in &scene.content.entries {
        let (marker, text) = match *entry {
            Entry::Request(request) => (Span::styled(theme.request, DIM), Line::raw(request)),
            Entry::Reply(reply) => (
                Span::styled(theme.reply, Style::new().fg(theme.flesh)),
                Line::raw(reply),
            ),
            Entry::Tool {
                name,
                input,
                output,
                running,
            } => {
                let (marker, result) = if running {
                    let spinner = theme.spinner[scene.motion.spinner % theme.spinner.len()];
                    (
                        Span::styled(spinner, Style::new().fg(theme.flesh)),
                        format!("{}s", scene.motion.seconds),
                    )
                } else {
                    (
                        Span::styled(theme.done, Style::new().fg(theme.rind)),
                        output.to_owned(),
                    )
                };
                let text = Line::from(vec![
                    Span::styled(name, BOLD),
                    Span::raw(format!(" {input} ")),
                    Span::styled(format!("{} {result}", theme.separator), DIM),
                ]);
                (marker, text)
            }
        };
        // (A blank row between entries, but none between tools one after another.)
        let is_tool = matches!(entry, Entry::Tool { .. });
        let tools_in_a_row = is_tool && after_tool;
        let gap = u16::from(!items.is_empty() && !tools_in_a_row);
        after_tool = is_tool;
        let rows = text::wrap(&text, text_width, true).len().max(1);
        items.push(Item {
            gap,
            marker,
            rows: u16::try_from(rows).unwrap_or(u16::MAX),
            text,
            running: matches!(entry, Entry::Tool { running: true, .. }),
        });
    }
    // (Rows count from the transcript's top, and a transcript longer than its area loses its first
    // rows.)
    let total: u16 = items.iter().map(|item| item.gap + item.rows).sum();
    let mut y = i32::from(area.height) - i32::from(total.max(area.height));
    let mut running_row = None;
    for item in items {
        y += i32::from(item.gap);
        let hidden = u16::try_from(-y).unwrap_or(0).min(item.rows);
        let top = area.y + u16::try_from(y).unwrap_or(0);
        y += i32::from(item.rows);
        if hidden == item.rows {
            continue;
        }
        let text = Paragraph::new(item.text)
            .wrap(Wrap { trim: true })
            .scroll((hidden, 0));
        let text_area = Rect::new(area.x + 2, top, text_width, item.rows - hidden);
        frame.render_widget(text, text_area);
        if hidden == 0 {
            frame.render_widget(item.marker, Rect::new(area.x, top, 1, 1));
        }
        if item.running {
            running_row = Some(Rect::new(area.x, top, area.width, 1));
        }
    }
    running_row
}

/// Draws the input box with what's typed in it, and puts the cursor after it.
fn draw_input(frame: &mut Frame<'_>, area: Rect, typed: &str, theme: &Theme) {
    let block = Block::bordered()
        .border_set(theme.border)
        .border_style(DIM)
        .padding(Padding::horizontal(1));
    let inside = block.inner(area);
    frame.render_widget(block, area);
    let prompt = Line::from(vec![
        Span::styled(format!("{} ", theme.request), DIM),
        Span::raw(typed),
    ]);
    let width = u16::try_from(prompt.width()).unwrap_or(u16::MAX);
    frame.render_widget(prompt, inside);
    frame.set_cursor_position((inside.x + width.min(inside.width), inside.y));
}

/// Termelon's mascot, a prompt's `>_` for eyes, winking, over a smile that's a slice of melon.
struct Face {
    /// The pixels, `G` for the rind's green and `F` for the flesh, two rows to a row of cells.
    pixels: &'static [&'static str],
    /// The winking eye opened, the `_` raised to the top of the `>`, as the columns and rows it
    /// fills (left, top, right, bottom, the ends excluded).
    open_eye: [usize; 4],
}

impl Face {
    /// 16 pixels square, 16 cells by 8.
    const SMALL: Face = Face {
        pixels: &[
            "................",
            "...GG...........",
            "....GG..........",
            ".....GG.........",
            "....GG...GGGG...",
            "...GG....GGGG...",
            "................",
            "................",
            ".GG.FFFFFFFF.GG.",
            ".GG.FFFFFFFF.GG.",
            "..GG.FFFFFF.GG..",
            "..GG..FFFF..GG..",
            "...GGG....GGG...",
            "....GGGGGGGG....",
            "......GGGG......",
            "................",
        ],
        open_eye: [9, 1, 13, 6],
    };

    /// 32 pixels square, 32 cells by 16.
    const LARGE: Face = Face {
        pixels: &[
            "................................",
            "................................",
            "......GGGG......................",
            ".......GGGG.....................",
            "........GGGG....................",
            ".........GGGG...................",
            "..........GGGG..................",
            "..........GGGG..................",
            ".........GGGG.....GGGGGGGG......",
            "........GGGG......GGGGGGGG......",
            ".......GGGG.......GGGGGGGG......",
            "......GGGG........GGGGGGGG......",
            "................................",
            "................................",
            "................................",
            "................................",
            "...GGG..FFFFFFFFFFFFFFFF..GGG...",
            "...GGG..FFFFFFFFFFFFFFFF..GGG...",
            "...GGG..FFFFFFFFFFFFFFFF..GGG...",
            "...GGGG..FFFFFFFFFFFFFF..GGGG...",
            "....GGG..FFFFFFFFFFFFFF..GGG....",
            "....GGGG..FFFFFFFFFFFF..GGGG....",
            ".....GGG...FFFFFFFFFF...GGG.....",
            ".....GGGG....FFFFFF....GGGG.....",
            "......GGGGG..........GGGGG......",
            ".......GGGGGG......GGGGGG.......",
            "........GGGGGGGGGGGGGGGG........",
            "..........GGGGGGGGGGGG..........",
            "............GGGGGGGG............",
            "................................",
            "................................",
            "................................",
        ],
        open_eye: [18, 2, 26, 12],
    };

    /// The face for a screen of `area`, the large one where there's room, and none where it would
    /// leave the transcript too little.
    fn to_fit(area: Rect) -> Option<&'static Face> {
        if area.height >= 40 && area.width >= 100 {
            Some(&Face::LARGE)
        } else if area.height >= 18 && area.width >= 40 {
            Some(&Face::SMALL)
        } else {
            None
        }
    }

    fn width(&self) -> u16 {
        u16::try_from(self.pixels[0].len()).unwrap_or(u16::MAX)
    }

    fn height(&self) -> u16 {
        u16::try_from(self.pixels.len() / 2).unwrap_or(u16::MAX)
    }

    /// The colour of the pixel at column `x` and row `y`, or `None` where the background shows.
    fn pixel(&self, x: usize, y: usize, eye_open: bool, theme: &Theme) -> Option<Color> {
        let [left, top, right, bottom] = self.open_eye;
        if eye_open && (left..right).contains(&x) && (top..bottom).contains(&y) {
            return Some(theme.rind);
        }
        match self.pixels[y].as_bytes()[x] {
            b'G' => Some(theme.rind),
            b'F' => Some(theme.flesh),
            _ => None,
        }
    }

    /// Draws the face in half blocks from `area`'s top left, as much of it as fits.
    fn draw(&self, buf: &mut Buffer, area: Rect, eye_open: bool, theme: &Theme) {
        for row in 0..area.height {
            for column in 0..area.width {
                let (x, y) = (usize::from(column), 2 * usize::from(row));
                let upper = self.pixel(x, y, eye_open, theme);
                let lower = self.pixel(x, y + 1, eye_open, theme);
                let (symbol, fg, bg) = match (upper, lower) {
                    (None, None) => continue,
                    (Some(upper), None) => ("▀", upper, Color::Reset),
                    (None, Some(lower)) => ("▄", lower, Color::Reset),
                    (Some(upper), Some(lower)) if upper == lower => ("█", upper, Color::Reset),
                    (Some(upper), Some(lower)) => ("▀", upper, lower),
                };
                if let Some(mut cell) = buf.cell_mut((area.x + column, area.y + row)) {
                    cell.set_symbol(symbol).set_fg(fg).set_bg(bg);
                }
            }
        }
    }
}
