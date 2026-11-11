//! Lowering colours to what the terminal shows, by a perceptual distance in integer arithmetic.

use crate::buffer::named_color;
use crate::caps::ColorDepth;
use crate::style::Color;

/// xterm's default palette for the sixteen named colours, which the nearest of 16 is measured
/// against.
const PALETTE16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xcd, 0x00, 0x00),
    (0x00, 0xcd, 0x00),
    (0xcd, 0xcd, 0x00),
    (0x00, 0x00, 0xee),
    (0xcd, 0x00, 0xcd),
    (0x00, 0xcd, 0xcd),
    (0xe5, 0xe5, 0xe5),
    (0x7f, 0x7f, 0x7f),
    (0xff, 0x00, 0x00),
    (0x00, 0xff, 0x00),
    (0xff, 0xff, 0x00),
    (0x5c, 0x5c, 0xff),
    (0xff, 0x00, 0xff),
    (0x00, 0xff, 0xff),
    (0xff, 0xff, 0xff),
];

/// The levels of each channel in xterm's 6 by 6 by 6 colour cube, entries 16 to 231.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// `color` as a terminal of `depth` can show it.
///
/// True colour keeps everything. 256 colours turn RGB into the nearest entry of xterm's cube and
/// grey ramp, leaving out the first 16, which follow the user's palette. 16 colours turn every
/// colour into the nearest named one. No colour turns everything into the default.
pub(crate) fn lower(color: Color, depth: ColorDepth) -> Color {
    match (depth, color) {
        (ColorDepth::None, _) => Color::Reset,
        (ColorDepth::TrueColor, c) => c,
        (ColorDepth::Ansi256, Color::Rgb(r, g, b)) => Color::Indexed(nearest_256(r, g, b)),
        (ColorDepth::Ansi256, c) => c,
        (ColorDepth::Ansi16, Color::Rgb(r, g, b)) => named_color(nearest_16(r, g, b)),
        (ColorDepth::Ansi16, Color::Indexed(n)) if n < 16 => named_color(n),
        (ColorDepth::Ansi16, Color::Indexed(n)) => {
            let (r, g, b) = indexed_rgb(n);
            named_color(nearest_16(r, g, b))
        }
        (ColorDepth::Ansi16, c) => c,
    }
}

/// The ANSI number, 0 to 15, of a named colour.
pub(crate) fn ansi_index(color: Color) -> Option<u8> {
    let n = match color {
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Reset | Color::Indexed(_) | Color::Rgb(..) => return None,
    };
    Some(n)
}

/// The RGB of palette entry `n` in xterm's defaults.
pub(crate) fn indexed_rgb(n: u8) -> (u8, u8, u8) {
    match n {
        0..=15 => PALETTE16[usize::from(n)],
        16..=231 => {
            let i = n - 16;
            (
                CUBE[usize::from(i / 36)],
                CUBE[usize::from(i / 6 % 6)],
                CUBE[usize::from(i % 6)],
            )
        }
        _ => {
            let level = 8 + (n - 232) * 10;
            (level, level, level)
        }
    }
}

/// The distance between two colours, weighted for how the eye sees red, green and blue ("redmean").
pub(crate) fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let (r1, g1, b1) = (u32::from(a.0), u32::from(a.1), u32::from(a.2));
    let (r2, g2, b2) = (u32::from(b.0), u32::from(b.1), u32::from(b.2));
    let mean = u32::midpoint(r1, r2);
    let dr = r1.abs_diff(r2);
    let dg = g1.abs_diff(g2);
    let db = b1.abs_diff(b2);
    (((512 + mean) * dr * dr) >> 8) + 4 * dg * dg + (((767 - mean) * db * db) >> 8)
}

/// The entry of xterm's palette from 16 to 255 nearest the colour, by searching them all. The
/// encoder keeps what it found in a [`Lowering`].
pub(crate) fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let mut best = 16;
    let mut best_distance = u32::MAX;
    for n in 16..=255 {
        let d = distance((r, g, b), indexed_rgb(n));
        if d < best_distance {
            best = n;
            best_distance = d;
        }
    }
    best
}

/// The ANSI number, 0 to 15, of the named colour nearest the colour.
pub(crate) fn nearest_16(r: u8, g: u8, b: u8) -> u8 {
    let mut best = 0;
    let mut best_distance = u32::MAX;
    for (n, &rgb) in (0..).zip(&PALETTE16) {
        let d = distance((r, g, b), rgb);
        if d < best_distance {
            best = n;
            best_distance = d;
        }
    }
    best
}

/// The colours lowered lately, since a frame uses few and lowering may search the palette.
#[derive(Debug, Clone)]
pub(crate) struct Lowering {
    depth: ColorDepth,
    slots: Vec<Option<(Color, Color)>>,
}

impl Lowering {
    const SLOTS: usize = 64;

    pub(crate) fn new() -> Self {
        Self {
            depth: ColorDepth::TrueColor,
            slots: vec![None; Self::SLOTS],
        }
    }

    /// [`lower`], remembered.
    pub(crate) fn lower(&mut self, color: Color, depth: ColorDepth) -> Color {
        let slot = match color {
            Color::Rgb(r, g, b) if depth != ColorDepth::TrueColor && depth != ColorDepth::None => {
                usize::from(r.wrapping_mul(7) ^ g.wrapping_mul(3) ^ b)
            }
            Color::Indexed(n) if n >= 16 && depth == ColorDepth::Ansi16 => usize::from(n),
            _ => return lower(color, depth),
        } % Self::SLOTS;
        if depth != self.depth {
            self.slots.fill(None);
            self.depth = depth;
        }
        if let Some(Some((from, to))) = self.slots.get(slot).copied()
            && from == color
        {
            return to;
        }
        let to = lower(color, depth);
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = Some((color, to));
        }
        to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The nearest entry by searching all of them, to check the shortcut against.
    fn nearest_256_by_search(r: u8, g: u8, b: u8) -> u32 {
        (16..=255u8)
            .map(|n| distance((r, g, b), indexed_rgb(n)))
            .min()
            .unwrap_or(0)
    }

    #[test]
    fn the_cube_and_ramp_have_xterms_values() {
        assert_eq!(indexed_rgb(16), (0, 0, 0));
        assert_eq!(indexed_rgb(196), (255, 0, 0));
        assert_eq!(indexed_rgb(231), (255, 255, 255));
        assert_eq!(indexed_rgb(232), (8, 8, 8));
        assert_eq!(indexed_rgb(255), (238, 238, 238));
        assert_eq!(indexed_rgb(9), (0xff, 0, 0));
    }

    #[test]
    fn nearest_256_is_as_near_as_any_entry() {
        for r in (0..=255u8).step_by(5) {
            for g in (0..=255u8).step_by(15) {
                for b in (0..=255u8).step_by(17) {
                    let got = distance((r, g, b), indexed_rgb(nearest_256(r, g, b)));
                    assert_eq!(got, nearest_256_by_search(r, g, b), "{r} {g} {b}");
                }
            }
        }
    }

    #[test]
    fn exact_entries_map_to_themselves() {
        for n in 16..=255u8 {
            let (r, g, b) = indexed_rgb(n);
            assert_eq!(indexed_rgb(nearest_256(r, g, b)), (r, g, b), "{n}");
        }
        assert_eq!(nearest_16(0xcd, 0, 0), 1);
        assert_eq!(nearest_16(250, 250, 250), 15);
        assert_eq!(nearest_16(0, 0, 0), 0);
    }

    #[test]
    fn lowering_follows_the_depth() {
        let rgb = Color::Rgb(250, 10, 10);
        assert_eq!(lower(rgb, ColorDepth::TrueColor), rgb);
        assert_eq!(lower(rgb, ColorDepth::Ansi256), Color::Indexed(196));
        assert_eq!(lower(rgb, ColorDepth::Ansi16), Color::LightRed);
        assert_eq!(lower(rgb, ColorDepth::None), Color::Reset);
        assert_eq!(lower(Color::Indexed(4), ColorDepth::Ansi16), Color::Blue);
        assert_eq!(
            lower(Color::Indexed(4), ColorDepth::Ansi256),
            Color::Indexed(4)
        );
        assert_eq!(
            lower(Color::Indexed(196), ColorDepth::Ansi16),
            Color::LightRed
        );
        assert_eq!(lower(Color::Cyan, ColorDepth::Ansi16), Color::Cyan);
        assert_eq!(lower(Color::Reset, ColorDepth::Ansi256), Color::Reset);
        assert_eq!(lower(Color::Cyan, ColorDepth::None), Color::Reset);
    }

    #[test]
    fn the_cache_gives_what_lowering_gives() {
        let mut cache = Lowering::new();
        for depth in [
            ColorDepth::Ansi16,
            ColorDepth::Ansi256,
            ColorDepth::TrueColor,
            ColorDepth::None,
        ] {
            for round in 0..2 {
                for n in 0..=255u8 {
                    for c in [
                        Color::Rgb(n, 255 - n, n / 2),
                        Color::Indexed(n),
                        named_color(n % 16),
                    ] {
                        assert_eq!(
                            cache.lower(c, depth),
                            lower(c, depth),
                            "{c:?} {depth:?} {round}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ansi_indexes_match_the_named_colours() {
        for n in 0..16 {
            assert_eq!(ansi_index(named_color(n)), Some(n));
        }
        assert_eq!(ansi_index(Color::Indexed(1)), None);
        assert_eq!(ansi_index(Color::Reset), None);
    }
}
