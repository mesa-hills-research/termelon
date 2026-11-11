//! Mouse reports in SGR, X10 and urxvt form, from xterm's control sequences document.

use super::*;
use crate::event::{MouseButton, MouseEvent, MouseEventKind};

const NONE: KeyModifiers = KeyModifiers::NONE;

fn mouse(kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) -> Decoded {
    Decoded::Event(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }))
}

#[test]
fn sgr() {
    use MouseButton::{Left, Middle, Right};
    use MouseEventKind::{Down, Drag, Moved, ScrollDown, ScrollLeft, ScrollRight, ScrollUp, Up};
    check_table(&[
        (b"\x1b[<0;1;1M", vec![mouse(Down(Left), 0, 0, NONE)]),
        (b"\x1b[<0;20;10M", vec![mouse(Down(Left), 19, 9, NONE)]),
        (b"\x1b[<0;20;10m", vec![mouse(Up(Left), 19, 9, NONE)]),
        (b"\x1b[<1;5;6M", vec![mouse(Down(Middle), 4, 5, NONE)]),
        (b"\x1b[<1;5;6m", vec![mouse(Up(Middle), 4, 5, NONE)]),
        (b"\x1b[<2;5;6M", vec![mouse(Down(Right), 4, 5, NONE)]),
        (b"\x1b[<2;5;6m", vec![mouse(Up(Right), 4, 5, NONE)]),
        (b"\x1b[<32;7;8M", vec![mouse(Drag(Left), 6, 7, NONE)]),
        (b"\x1b[<33;7;8M", vec![mouse(Drag(Middle), 6, 7, NONE)]),
        (b"\x1b[<34;7;8M", vec![mouse(Drag(Right), 6, 7, NONE)]),
        (b"\x1b[<35;7;8M", vec![mouse(Moved, 6, 7, NONE)]),
        (b"\x1b[<64;3;4M", vec![mouse(ScrollUp, 2, 3, NONE)]),
        (b"\x1b[<65;3;4M", vec![mouse(ScrollDown, 2, 3, NONE)]),
        (b"\x1b[<66;3;4M", vec![mouse(ScrollLeft, 2, 3, NONE)]),
        (b"\x1b[<67;3;4M", vec![mouse(ScrollRight, 2, 3, NONE)]),
        (
            b"\x1b[<4;1;1M",
            vec![mouse(Down(Left), 0, 0, KeyModifiers::SHIFT)],
        ),
        (
            b"\x1b[<8;1;1M",
            vec![mouse(Down(Left), 0, 0, KeyModifiers::ALT)],
        ),
        (
            b"\x1b[<16;1;1M",
            vec![mouse(Down(Left), 0, 0, KeyModifiers::CONTROL)],
        ),
        (
            b"\x1b[<28;1;1M",
            vec![mouse(
                Down(Left),
                0,
                0,
                KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
            )],
        ),
        (
            b"\x1b[<80;1;1M",
            vec![mouse(ScrollUp, 0, 0, KeyModifiers::CONTROL)],
        ),
        (
            b"\x1b[<48;1;1M",
            vec![mouse(Drag(Left), 0, 0, KeyModifiers::CONTROL)],
        ),
        // Past the 223 columns X10 can say, and with the trailing `;` some terminals send.
        (b"\x1b[<0;300;200M", vec![mouse(Down(Left), 299, 199, NONE)]),
        (b"\x1b[<0;20;10;M", vec![mouse(Down(Left), 19, 9, NONE)]),
        (
            b"\x1b[<0;99999;1M",
            vec![mouse(Down(Left), u16::MAX, 0, NONE)],
        ),
    ]);
}

#[test]
fn sgr_reports_that_say_nothing_are_dropped() {
    check_table(&[
        // Buttons 8 to 11, and motion with the wheel's bits.
        (b"\x1b[<128;1;1Mx", vec![ch('x')]),
        (b"\x1b[<98;1;1Mx", vec![ch('x')]),
        (b"\x1b[<0;1Mx", vec![ch('x')]),
        (b"\x1b[<;;Mx", vec![ch('x')]),
    ]);
}

#[test]
fn x10() {
    use MouseButton::{Left, Middle, Right};
    use MouseEventKind::{Down, Drag, Moved, ScrollDown, ScrollUp, Up};
    check_table(&[
        (b"\x1b[M\x20\x21\x21", vec![mouse(Down(Left), 0, 0, NONE)]),
        (
            b"\x1b[M\x21\x2a\x2b",
            vec![mouse(Down(Middle), 9, 10, NONE)],
        ),
        (b"\x1b[M\x22\x21\x21", vec![mouse(Down(Right), 0, 0, NONE)]),
        (b"\x1b[M\x23\x21\x21", vec![mouse(Up(Left), 0, 0, NONE)]),
        (b"\x1b[M\x40\x21\x21", vec![mouse(Drag(Left), 0, 0, NONE)]),
        (b"\x1b[M\x43\x21\x21", vec![mouse(Moved, 0, 0, NONE)]),
        (b"\x1b[M\x60\x21\x21", vec![mouse(ScrollUp, 0, 0, NONE)]),
        (b"\x1b[M\x61\x21\x21", vec![mouse(ScrollDown, 0, 0, NONE)]),
        (
            b"\x1b[M0\x60\x70",
            vec![mouse(Down(Left), 63, 79, KeyModifiers::CONTROL)],
        ),
        (
            b"\x1b[M\x20\xff\xff",
            vec![mouse(Down(Left), 222, 222, NONE)],
        ),
    ]);
}

#[test]
fn x10_bytes_that_are_no_report_are_keys() {
    let alt_bracket = key(KeyCode::Char('['), KeyModifiers::ALT);
    let shift_m = key(KeyCode::Char('M'), KeyModifiers::SHIFT);
    check_table(&[
        (b"\x1b[M", vec![alt_bracket.clone(), shift_m.clone()]),
        (
            b"\x1b[Ma",
            vec![alt_bracket.clone(), shift_m.clone(), ch('a')],
        ),
        (
            b"\x1b[Mab\r",
            vec![
                alt_bracket,
                shift_m,
                ch('a'),
                ch('b'),
                key(KeyCode::Enter, NONE),
            ],
        ),
    ]);
}

#[test]
fn urxvt() {
    use MouseButton::Left;
    use MouseEventKind::{Down, ScrollUp, Up};
    check_table(&[
        (b"\x1b[32;30;40M", vec![mouse(Down(Left), 29, 39, NONE)]),
        (b"\x1b[32;30;40;M", vec![mouse(Down(Left), 29, 39, NONE)]),
        (b"\x1b[35;1;1M", vec![mouse(Up(Left), 0, 0, NONE)]),
        (b"\x1b[96;1;1M", vec![mouse(ScrollUp, 0, 0, NONE)]),
        (b"\x1b[31;1;1Mx", vec![ch('x')]),
    ]);
}
