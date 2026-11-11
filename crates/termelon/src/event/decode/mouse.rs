//! Mouse reports, in SGR (`CSI < b ; x ; y M`), X10 (`CSI M` and three bytes) and urxvt
//! (`CSI b ; x ; y M`) form.

use super::tokenizer::Csi;
use crate::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

/// An SGR or urxvt report.
pub(super) fn csi(csi: &Csi) -> Option<MouseEvent> {
    let params = &csi.params;
    if params.count() < 3 {
        return None;
    }
    let sgr = csi.private == Some(b'<');
    let mut button = params.value(0)?;
    if !sgr {
        // urxvt adds 32, as X10 does.
        button = button.checked_sub(32)?;
    }
    let (mut kind, modifiers) = button_kind(button)?;
    if sgr && csi.final_byte == b'm' {
        // SGR says which button came up.
        if let MouseEventKind::Down(button) = kind {
            kind = MouseEventKind::Up(button);
        }
    }
    Some(MouseEvent {
        kind,
        column: coordinate(params.value(1)?),
        row: coordinate(params.value(2)?),
        modifiers,
    })
}

/// An X10 report's three bytes, each 32 more than its value, with 1-based coordinates.
pub(super) fn x10(bytes: [u8; 3]) -> Option<MouseEvent> {
    let [button, column, row] = bytes.map(|byte| u32::from(byte.saturating_sub(32)));
    let (kind, modifiers) = button_kind(button)?;
    Some(MouseEvent {
        kind,
        column: coordinate(column),
        row: coordinate(row),
        modifiers,
    })
}

/// A 1-based coordinate as the library's 0-based one.
fn coordinate(value: u32) -> u16 {
    u16::try_from(value.saturating_sub(1)).unwrap_or(u16::MAX)
}

/// The event and modifiers in a report's button value. The low two bits and bits 64 and 128 are
/// the button, 32 is motion, and 4, 8 and 16 are Shift, Alt and Ctrl. Buttons 8 to 11 (128 and
/// up) aren't reported.
fn button_kind(value: u32) -> Option<(MouseEventKind, KeyModifiers)> {
    if value > 0xff {
        return None;
    }
    let number = (value & 0b11) | ((value & 0b1100_0000) >> 4);
    let motion = value & 0b10_0000 != 0;
    let kind = match (number, motion) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        // X10 and urxvt say a button came up, not which.
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3..=5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        _ => return None,
    };
    let mut modifiers = KeyModifiers::NONE;
    for (bit, modifier) in [
        (4, KeyModifiers::SHIFT),
        (8, KeyModifiers::ALT),
        (16, KeyModifiers::CONTROL),
    ] {
        if value & bit != 0 {
            modifiers |= modifier;
        }
    }
    Some((kind, modifiers))
}
