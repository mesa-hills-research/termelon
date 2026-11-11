//! The workloads that concern output (DESIGN.md section 15.2). Each is a sequence of frames, and
//! every stack draws exactly the same ones.

use crate::content;
use crate::model::{Color, Frame};
use crate::scene::{Screen, on_background};

/// A sequence of frames to draw one after another.
#[derive(Debug)]
pub(crate) struct Workload {
    pub(crate) name: String,
    /// Frames drawn first and not timed, to put the screen in its starting state.
    pub(crate) warmup: Vec<Frame>,
    pub(crate) frames: Vec<Frame>,
    /// Whether the terminal is cleared and the previous frame forgotten before every frame, as a
    /// resize does.
    pub(crate) repaint: bool,
}

/// The workloads, in the order they run.
pub(crate) const NAMES: [&str; 5] = ["idle", "streaming", "scrolling", "typing", "full-redraw"];

/// How many frames a workload has unless told otherwise.
pub(crate) fn default_frames(name: &str) -> usize {
    match name {
        "streaming" => 300,
        "full-redraw" => 50,
        _ => 100,
    }
}

/// The workload `name` with `frames` timed frames, or `None` for a name it doesn't know.
pub(crate) fn build(name: &str, frames: usize) -> Option<Workload> {
    let (warmup, timed, repaint) = match name {
        "idle" => {
            let (warmup, timed) = idle(frames);
            (warmup, timed, false)
        }
        "streaming" => {
            let (warmup, timed) = streaming(frames);
            (warmup, timed, false)
        }
        "scrolling" => {
            let (warmup, timed) = scrolling(frames);
            (warmup, timed, false)
        }
        "typing" => {
            let (warmup, timed) = typing(frames);
            (warmup, timed, false)
        }
        "full-redraw" => {
            let (warmup, timed) = full_redraw(frames);
            (warmup, timed, true)
        }
        _ => return None,
    };
    Some(Workload {
        name: name.to_string(),
        warmup,
        frames: timed,
        repaint,
    })
}

/// The workloads replayed from frames recorded from Arko, each with the file in the recordings
/// directory that holds it.
pub(crate) const RECORDED: [(&str, &str); 5] = [
    ("idle-rec", "idle.styled"),
    ("streaming-rec", "streaming.styled"),
    ("scrolling-rec", "scrolling.styled"),
    ("typing-rec", "typing.styled"),
    ("full-redraw-rec", "redraw.styled"),
];

/// The file that holds the recorded workload `name`, or `None` for a name that isn't one.
pub(crate) fn recorded_file(name: &str) -> Option<&'static str> {
    RECORDED
        .iter()
        .find(|(recorded, _)| *recorded == name)
        .map(|(_, file)| *file)
}

/// A workload from recorded frames. The first is drawn untimed, to put the screen in its starting
/// state, unless it is the only one. With `repaint`, every frame is timed, after a resize, and the
/// first is also drawn untimed beforehand.
pub(crate) fn from_frames(name: &str, mut frames: Vec<Frame>, repaint: bool) -> Workload {
    let warmup = if repaint {
        frames.first().cloned().into_iter().collect()
    } else if frames.len() > 1 {
        frames.drain(..1).collect()
    } else {
        Vec::new()
    };
    Workload {
        name: name.to_string(),
        warmup,
        frames,
        repaint,
    }
}

/// Idle animation. A turn waits on its model at 200 by 60, and only the spinner, the highlight over
/// "Working…" and the status line's timer change, a frame every 100 ms.
fn idle(frames: usize) -> (Vec<Frame>, Vec<Frame>) {
    const START_MS: u64 = 12_300;
    let (width, height) = (200, 60);
    let mut screen = Screen::new(width, height, content::history(width, 150));
    let mut at = |ms: u64| {
        screen.working_ms = Some(ms);
        screen.status.working = Some(ms / 1000);
        screen.draw()
    };
    let warmup = vec![at(START_MS)];
    let timed = (1..=frames)
        .map(|k| at(START_MS + 100 * k as u64))
        .collect();
    (warmup, timed)
}

/// Streaming. A reply grows a word a frame at the bottom of 120 by 40, and the view follows, so
/// rows scroll up whenever a line fills. Frames come every 33 ms.
fn streaming(frames: usize) -> (Vec<Frame>, Vec<Frame>) {
    const START_MS: u64 = 4_000;
    let (width, height) = (120, 40);
    let history = content::history(width, 200);
    let parts = content::reply(frames + 1);
    let mut at = |words: usize| {
        let mut screen = Screen::new(width, height, history.clone());
        screen
            .rows
            .extend(content::layout(&content::reveal(&parts, words), width));
        let ms = START_MS + 33 * words as u64;
        screen.status.working = Some(ms / 1000);
        screen.draw()
    };
    let warmup = vec![at(1)];
    let timed = (2..frames + 2).map(&mut at).collect();
    (warmup, timed)
}

/// Scrolling. Page Up through a long transcript at 120 by 40, a page a frame, so every row of the
/// transcript changes.
fn scrolling(frames: usize) -> (Vec<Frame>, Vec<Frame>) {
    let (width, height) = (120, 40);
    let mut screen = Screen::new(width, height, Vec::new());
    let page = screen.transcript_height();
    screen.rows = content::history(width, (frames + 2) * page);
    let mut at = |pages: usize| {
        screen.scrolled_back = pages * page;
        screen.draw()
    };
    let warmup = vec![at(0)];
    let timed = (1..=frames).map(at).collect();
    (warmup, timed)
}

/// Typing. A character a frame in the input box at 120 by 40, which moves the cursor with it.
fn typing(frames: usize) -> (Vec<Frame>, Vec<Frame>) {
    let (width, height) = (120, 40);
    let mut screen = Screen::new(width, height, content::history(width, 100));
    let mut at = |characters: usize| {
        screen.input = content::typed(characters);
        screen.draw()
    };
    let warmup = vec![at(1)];
    let timed = (2..frames + 2).map(at).collect();
    (warmup, timed)
}

/// Full redraw. Every cell of a 300 by 100 screen is styled in RGB, each with a colour of its own,
/// as the first frame after a resize is.
fn full_redraw(frames: usize) -> (Vec<Frame>, Vec<Frame>) {
    let warmup = vec![gradient(300, 100, 0)];
    let timed = (1..=frames).map(|t| gradient(300, 100, t)).collect();
    (warmup, timed)
}

const FIELD: &str = "The quick brown fox jumps over the lazy dog 0123456789 ";

/// A screen of text whose every cell has its own foreground, over a background that changes by row
/// and in stripes, all shifted by `step`.
fn gradient(width: u16, height: u16, step: usize) -> Frame {
    let byte = |value: usize| u8::try_from(value % 256).unwrap_or(0);
    let mut frame = Frame::new(width, height);
    let letters: Vec<char> = FIELD.chars().collect();
    let (columns, rows) = (usize::from(width), usize::from(height));
    for y in 0..height {
        let row = usize::from(y);
        for x in 0..width {
            let col = usize::from(x);
            let fg = Color::Rgb(
                byte(col * 255 / columns),
                byte(row * 255 / rows),
                byte(col + row + step * 5),
            );
            let stripe = if col / 16 % 2 == 0 { 0 } else { 6 };
            let bg = Color::Rgb(
                byte(16 + row % 8 * 2),
                byte(16 + row * 40 / rows + stripe),
                byte(24 + step % 8),
            );
            let letter = letters[(col + 7 * row + step) % letters.len()];
            frame.set_char(x, y, letter, on_background(fg).bg(bg));
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;

    /// The cells where `a` and `b` differ, as (column, row).
    fn changed(a: &Frame, b: &Frame) -> Vec<(u16, u16)> {
        assert_eq!((a.width(), a.height()), (b.width(), b.height()));
        let mut cells = Vec::new();
        for y in 0..a.height() {
            for x in 0..a.width() {
                let (left, right) = (a.cell(x, y).unwrap(), b.cell(x, y).unwrap());
                let (mut one, mut two) = ([0; 4], [0; 4]);
                if a.symbol(left.symbol, &mut one) != b.symbol(right.symbol, &mut two)
                    || left.style != right.style
                {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    fn every(workload: &Workload) -> Vec<&Frame> {
        workload.warmup.iter().chain(&workload.frames).collect()
    }

    #[test]
    fn every_workload_builds_with_the_frames_asked_for() {
        for name in NAMES {
            let workload = build(name, 4).unwrap();
            assert_eq!(workload.name, name);
            assert_eq!(workload.frames.len(), 4, "{name}");
            assert!(!workload.warmup.is_empty(), "{name}");
        }
        assert!(build("nothing", 4).is_none());
    }

    #[test]
    fn the_recordings_have_files() {
        assert_eq!(recorded_file("idle-rec"), Some("idle.styled"));
        assert_eq!(recorded_file("full-redraw-rec"), Some("redraw.styled"));
        assert_eq!(recorded_file("idle"), None);
        assert_eq!(RECORDED.len(), NAMES.len());
        for ((recorded, _), name) in RECORDED.iter().zip(NAMES) {
            assert_eq!(*recorded, format!("{name}-rec"));
        }
    }

    #[test]
    fn the_defaults_are_the_designs() {
        assert_eq!(default_frames("idle"), 100);
        assert_eq!(default_frames("streaming"), 300);
        assert_eq!(default_frames("scrolling"), 100);
        assert_eq!(default_frames("typing"), 100);
    }

    #[test]
    fn a_workload_is_the_same_every_time() {
        for name in NAMES {
            let (a, b) = (build(name, 3).unwrap(), build(name, 3).unwrap());
            assert!(every(&a) == every(&b), "{name}");
        }
    }

    #[test]
    fn idle_changes_only_the_spinner_the_shimmer_and_the_timer() {
        let workload = build("idle", 100).unwrap();
        assert!(
            every(&workload)
                .iter()
                .all(|f| (f.width(), f.height()) == (200, 60))
        );
        let frames = every(&workload);
        let working_row = (0..60)
            .find(|y| frames[0].row_text(*y).ends_with("Working…"))
            .expect("a Working… row");
        let status_row = (0..60)
            .find(|y| frames[0].row_text(*y).contains("● working"))
            .expect("a status row");
        let mut timer_changes = 0;
        for pair in frames.windows(2) {
            let cells = changed(pair[0], pair[1]);
            assert!(!cells.is_empty());
            assert!(cells.len() <= 16, "{} cells", cells.len());
            assert!(
                cells.iter().all(|(x, y)| {
                    (*y == working_row && *x < 10) || (*y == status_row && *x < 20)
                }),
                "{cells:?}"
            );
            timer_changes += usize::from(cells.iter().any(|(_, y)| *y == status_row));
        }
        // 100 frames of 100 ms cover ten seconds.
        assert_eq!(timer_changes, 10);
        // Both the spinner and the label's colours move.
        let spinner = |f: &Frame| f.cell(0, working_row).unwrap().symbol;
        assert_ne!(spinner(frames[0]), spinner(frames[1]));
        assert_eq!(spinner(frames[0]), spinner(frames[8]));
    }

    #[test]
    fn streaming_grows_the_last_row_and_scrolls_when_a_line_fills() {
        let workload = build("streaming", 300).unwrap();
        let frames = every(&workload);
        assert!(frames.iter().all(|f| (f.width(), f.height()) == (120, 40)));
        // The transcript's last text row sits above the rule, the status line, and the input box.
        let last = |f: &Frame| {
            (0..35)
                .rev()
                .map(|y| f.row_text(y))
                .find(|text| !text.is_empty())
                .unwrap()
        };
        let mut scrolls = 0;
        for (at, pair) in frames.windows(2).enumerate() {
            let (before, after) = (last(pair[0]), last(pair[1]));
            if after.starts_with(&before) && after.len() > before.len() {
                let cells = changed(pair[0], pair[1]);
                // The row being written, and the status line when its timer ticks over.
                assert!(cells.iter().all(|(_, y)| *y == 33 || *y == 36), "{cells:?}");
            } else {
                // The line filled, so the rows moved up and the new line began.
                scrolls += 1;
                let cells = changed(pair[0], pair[1]).len();
                assert!(
                    cells > 500,
                    "frame {at}: {cells} cells, {before:?} then {after:?}"
                );
            }
        }
        assert!(
            (5..=60).contains(&scrolls),
            "{scrolls} scrolls in 300 frames"
        );
        // The reply is longer by a word each frame, up to a code block's lines and back.
        assert_ne!(last(frames[0]), last(frames[300]));
    }

    #[test]
    fn scrolling_changes_every_row_of_the_transcript() {
        let workload = build("scrolling", 100).unwrap();
        let frames = every(&workload);
        assert!(frames.iter().all(|f| (f.width(), f.height()) == (120, 40)));
        for pair in frames.windows(2) {
            let cells = changed(pair[0], pair[1]);
            let rows: std::collections::HashSet<u16> = cells.iter().map(|(_, y)| *y).collect();
            // Of the 35 transcript rows, nearly all differ, and the status and input stay.
            assert!(rows.len() >= 25, "{} of 35 rows", rows.len());
            assert!(cells.iter().all(|(_, y)| *y < 35));
        }
        // The first frame is the end of the transcript, and the last is far back from it.
        assert_ne!(frames[0].row_text(0), frames[100].row_text(0));
    }

    #[test]
    fn typing_adds_a_character_and_moves_the_cursor() {
        let workload = build("typing", 100).unwrap();
        let frames = every(&workload);
        assert!(frames.iter().all(|f| (f.width(), f.height()) == (120, 40)));
        for pair in frames.windows(2) {
            let cells = changed(pair[0], pair[1]);
            let rows: std::collections::HashSet<u16> = cells.iter().map(|(_, y)| *y).collect();
            // Typing a space over a blank changes no cell, only the cursor.
            assert!(rows.len() <= 1 && cells.len() <= 2, "{cells:?}");
            let moved = pair[1].cursor.x - pair[0].cursor.x;
            assert_eq!(
                usize::from(moved),
                cells.len().max(1),
                "the cursor moves by the width typed"
            );
            assert_eq!(pair[1].cursor.y, pair[0].cursor.y);
            assert!(pair[1].cursor.visible);
        }
        let last = frames.last().unwrap();
        assert!(
            last.row_text(last.cursor.y)
                .starts_with(" › Please fix the blank line")
        );
        assert!(last.row_text(last.cursor.y).contains("日本語"));
    }

    #[test]
    fn typing_past_the_row_wraps_and_grows_the_box() {
        let workload = build("typing", 200).unwrap();
        let last = workload.frames.last().unwrap();
        // Two rows of input between two rules, so the top rule is a row higher than with one.
        assert_eq!(last.row_text(36), "─".repeat(120));
        assert!(last.row_text(37).starts_with(" › Please fix"));
        assert!(last.row_text(38).starts_with("   "));
        assert_eq!(last.cursor.y, 38);
    }

    #[test]
    fn full_redraw_styles_every_cell_in_rgb_and_repaints() {
        let workload = build("full-redraw", 3).unwrap();
        assert!(workload.repaint);
        for frame in every(&workload) {
            assert_eq!((frame.width(), frame.height()), (300, 100));
            assert!(frame.cells().iter().all(|cell| {
                matches!(cell.style.fg, Color::Rgb(..))
                    && matches!(cell.style.bg, Color::Rgb(..))
                    && matches!(cell.symbol, Symbol::Char(_))
            }));
        }
        let frames = every(&workload);
        // The colours change from cell to cell along a row.
        let row = frames[1].row(50);
        assert!(row.windows(2).filter(|w| w[0].style != w[1].style).count() > 250);
        assert!(changed(frames[0], frames[1]).len() > 20_000);
    }

    #[test]
    fn recorded_frames_become_a_workload() {
        let frames: Vec<Frame> = (0..4).map(|n| Frame::new(3 + n, 2)).collect();
        let workload = from_frames("session", frames.clone(), false);
        assert_eq!(workload.name, "session");
        assert_eq!(workload.warmup.len(), 1);
        assert_eq!(workload.frames.len(), 3);
        assert!(!workload.repaint);
        let single = from_frames("one", vec![Frame::new(2, 2)], false);
        assert!(single.warmup.is_empty());
        assert_eq!(single.frames.len(), 1);
        // A repainting workload times every frame, and draws the first once before.
        let repainting = from_frames("resize", frames, true);
        assert!(repainting.repaint);
        assert_eq!(repainting.warmup.len(), 1);
        assert_eq!(repainting.frames.len(), 4);
        assert_eq!(repainting.warmup[0], repainting.frames[0]);
    }
}
