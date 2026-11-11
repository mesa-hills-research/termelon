//! The keys and mouse events the harness presses in each terminal, and how a fixture names them.

use tuiframe_vt::{Key, Mods, Mouse, MouseButton, MouseKind};

const NONE: Mods = Mods::NONE;
const SHIFT: Mods = Mods::SHIFT;
const ALT: Mods = Mods::ALT;
const CTRL: Mods = Mods::CTRL;

/// The keys, in order.
pub fn keys() -> Vec<(Key, Mods)> {
    vec![
        (Key::Char('a'), NONE),
        (Key::Char('a'), SHIFT),
        (Key::Char('a'), CTRL),
        (Key::Char('a'), ALT),
        (Key::Char('a'), CTRL | ALT),
        (Key::Char('1'), NONE),
        (Key::Char('1'), CTRL),
        (Key::Char(' '), NONE),
        (Key::Char(' '), CTRL),
        (Key::Char('é'), NONE),
        (Key::Enter, NONE),
        (Key::Enter, SHIFT),
        (Key::Enter, CTRL),
        (Key::Enter, ALT),
        (Key::Tab, NONE),
        (Key::Tab, SHIFT),
        (Key::Tab, CTRL),
        (Key::Backspace, NONE),
        (Key::Backspace, CTRL),
        (Key::Backspace, ALT),
        (Key::Escape, NONE),
        (Key::Up, NONE),
        (Key::Down, NONE),
        (Key::Left, NONE),
        (Key::Right, NONE),
        (Key::Up, SHIFT),
        (Key::Left, CTRL),
        (Key::Right, ALT),
        (Key::Home, NONE),
        (Key::End, NONE),
        (Key::Home, CTRL),
        (Key::PageUp, NONE),
        (Key::PageDown, NONE),
        (Key::Insert, NONE),
        (Key::Delete, NONE),
        (Key::Delete, SHIFT),
        (Key::F(1), NONE),
        (Key::F(5), NONE),
        (Key::F(12), NONE),
        (Key::F(5), CTRL),
    ]
}

/// The mouse events, in order, at cell 3,2.
pub const MOUSE: [Mouse; 6] = [
    Mouse::new(MouseKind::Press(MouseButton::Left), 3, 2),
    Mouse::new(MouseKind::Release(MouseButton::Left), 3, 2),
    Mouse::new(MouseKind::Press(MouseButton::Right), 3, 2),
    Mouse::new(MouseKind::Release(MouseButton::Right), 3, 2),
    Mouse::new(MouseKind::ScrollUp, 3, 2),
    Mouse::new(MouseKind::ScrollDown, 3, 2),
];

/// A key as a fixture names it, kitty's way, such as `ctrl+shift+a` or `page_up`.
pub fn key_name(key: Key, mods: Mods) -> String {
    crate::keys::kitty(key, mods).unwrap_or_else(|| format!("{key:?}"))
}

/// A mouse event as a fixture names it, such as `press left 3,2` or `scroll up 3,2`.
pub fn mouse_name(mouse: Mouse) -> String {
    let button = |b: MouseButton| match b {
        MouseButton::Left => "left",
        MouseButton::Middle => "middle",
        MouseButton::Right => "right",
    };
    let what = match mouse.kind {
        MouseKind::Press(b) => format!("press {}", button(b)),
        MouseKind::Release(b) => format!("release {}", button(b)),
        MouseKind::Drag(b) => format!("drag {}", button(b)),
        MouseKind::Move => "move".to_owned(),
        MouseKind::ScrollUp => "scroll up".to_owned(),
        MouseKind::ScrollDown => "scroll down".to_owned(),
        MouseKind::ScrollLeft => "scroll left".to_owned(),
        MouseKind::ScrollRight => "scroll right".to_owned(),
    };
    format!("{what} {},{}", mouse.x, mouse.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(key_name(Key::Char('a'), CTRL | ALT), "ctrl+alt+a");
        assert_eq!(key_name(Key::PageUp, NONE), "page_up");
        assert_eq!(mouse_name(MOUSE[0]), "press left 3,2");
        assert_eq!(mouse_name(MOUSE[5]), "scroll down 3,2");
    }
}
