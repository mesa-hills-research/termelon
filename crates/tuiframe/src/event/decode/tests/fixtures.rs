//! The bytes real terminals sent for the harness's scripted keys and mouse events, decoded.
//!
//! Each directory in `tests/fixtures/input` is a terminal, its version and a mode, and its
//! `input.txt` has a line for each event the harness pressed, its name, a tab and the bytes in
//! hex, as `tools/harness` writes them. Every event decodes to one event, or to none where the
//! terminal sent nothing. Where the event decoded isn't the one pressed, since the encoding can't
//! carry a modifier, `decoded.txt` beside it lists it with what it decoded to.
//! `UPDATE_GOLDENS=1` writes `decoded.txt` as the decoder has it now.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::*;
use crate::event::{MouseButton, MouseEvent, MouseEventKind};

const HEADER: &str = "# The events that decode as another, and what they decode as.\n";

/// The fixtures' directories, in order.
fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/input");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|dir| dir.join("input.txt").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// A fixture's events and their bytes.
fn parse(text: &str) -> Vec<(&str, Vec<u8>)> {
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, hex) = line.split_once('\t').unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            (name, bytes)
        })
        .collect()
}

/// The modifiers as the harness names them, in its order.
fn modifiers(mods: KeyModifiers) -> String {
    [
        (KeyModifiers::CONTROL, "ctrl+"),
        (KeyModifiers::ALT, "alt+"),
        (KeyModifiers::SHIFT, "shift+"),
        (KeyModifiers::SUPER, "super+"),
        (KeyModifiers::HYPER, "hyper+"),
        (KeyModifiers::META, "meta+"),
    ]
    .into_iter()
    .filter(|&(held, _)| mods.contains(held))
    .map(|(_, name)| name)
    .collect()
}

/// A key as the harness names it, such as `ctrl+a`, `shift+tab` or `page_up`. A capital comes
/// with shift, so `A` with shift is `shift+a`.
fn key_name(key: &KeyEvent) -> String {
    let mut mods = key.modifiers;
    let code = match key.code {
        KeyCode::Char(' ') => "space".to_owned(),
        KeyCode::Char(c) if mods.contains(KeyModifiers::SHIFT) => c.to_lowercase().collect(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::BackTab => {
            mods |= KeyModifiers::SHIFT;
            "tab".to_owned()
        }
        KeyCode::Enter => "enter".to_owned(),
        KeyCode::Tab => "tab".to_owned(),
        KeyCode::Backspace => "backspace".to_owned(),
        KeyCode::Esc => "escape".to_owned(),
        KeyCode::Up => "up".to_owned(),
        KeyCode::Down => "down".to_owned(),
        KeyCode::Left => "left".to_owned(),
        KeyCode::Right => "right".to_owned(),
        KeyCode::Home => "home".to_owned(),
        KeyCode::End => "end".to_owned(),
        KeyCode::PageUp => "page_up".to_owned(),
        KeyCode::PageDown => "page_down".to_owned(),
        KeyCode::Insert => "insert".to_owned(),
        KeyCode::Delete => "delete".to_owned(),
        KeyCode::F(n) => format!("f{n}"),
        other => format!("{other:?}"),
    };
    let mut name = format!("{}{code}", modifiers(mods));
    if key.kind != KeyEventKind::Press {
        let _ = write!(name, " {:?}", key.kind);
    }
    name
}

/// A mouse event as the harness names it, such as `press left 3,2` or `scroll up 3,2`.
fn mouse_name(mouse: MouseEvent) -> String {
    let button = |button: MouseButton| match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    };
    let what = match mouse.kind {
        MouseEventKind::Down(b) => format!("press {}", button(b)),
        MouseEventKind::Up(b) => format!("release {}", button(b)),
        MouseEventKind::Drag(b) => format!("drag {}", button(b)),
        MouseEventKind::Moved => "move".to_owned(),
        MouseEventKind::ScrollUp => "scroll up".to_owned(),
        MouseEventKind::ScrollDown => "scroll down".to_owned(),
        MouseEventKind::ScrollLeft => "scroll left".to_owned(),
        MouseEventKind::ScrollRight => "scroll right".to_owned(),
    };
    format!(
        "{}{what} {},{}",
        modifiers(mouse.modifiers),
        mouse.column,
        mouse.row
    )
}

/// What an event's bytes decoded to, by name.
fn decoded_name(decoded: &[Decoded]) -> String {
    match decoded {
        [] => "nothing".to_owned(),
        [Decoded::Event(Event::Key(key))] => key_name(key),
        [Decoded::Event(Event::Mouse(mouse))] => mouse_name(*mouse),
        other => format!("{other:?}"),
    }
}

#[test]
fn real_terminals_bytes_decode() {
    let update = std::env::var_os("UPDATE_GOLDENS").is_some();
    let dirs = fixtures();
    assert!(!dirs.is_empty(), "no fixtures in tests/fixtures/input");
    for dir in dirs {
        let text = std::fs::read_to_string(dir.join("input.txt")).unwrap();
        let mut differences = HEADER.to_owned();
        for (name, bytes) in parse(&text) {
            let decoded = decode(&bytes);
            assert!(
                matches!(decoded.as_slice(), [] | [Decoded::Event(_)]),
                "{}: {name} decoded to {decoded:?}",
                dir.display()
            );
            if bytes.is_empty() {
                continue;
            }
            let got = decoded_name(&decoded);
            if got != name {
                let _ = writeln!(differences, "{name}\t{got}");
            }
        }
        let path = dir.join("decoded.txt");
        if update {
            std::fs::write(&path, &differences).unwrap();
        } else {
            let expected = std::fs::read_to_string(&path).unwrap_or_default();
            assert_eq!(
                expected,
                differences,
                "{} differs, which UPDATE_GOLDENS=1 writes again",
                path.display()
            );
        }
    }
}

#[test]
fn names_follow_the_harness() {
    let key = |code, mods| KeyEvent::new(code, mods);
    assert_eq!(
        key_name(&key(KeyCode::Char('A'), KeyModifiers::SHIFT)),
        "shift+a"
    );
    assert_eq!(
        key_name(&key(KeyCode::BackTab, KeyModifiers::NONE)),
        "shift+tab"
    );
    assert_eq!(
        key_name(&key(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )),
        "ctrl+alt+a"
    );
    assert_eq!(
        key_name(&key(KeyCode::PageUp, KeyModifiers::NONE)),
        "page_up"
    );
    let mouse = MouseEvent::new(MouseEventKind::ScrollDown, 3, 2, KeyModifiers::NONE);
    assert_eq!(mouse_name(mouse), "scroll down 3,2");
}
