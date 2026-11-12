//! Starts real terminals with the probe, drives them and reads back what they show.
//!
//! Each terminal has a [`Driver`]: it starts the terminal at a size with a command and a clean
//! environment, sends raw bytes, synthetic keys and, where it can, mouse events, reads the screen
//! back, takes a screenshot where there's a display, and stops it. [`Driver::wait_for`] polls the readback every 100 ms until a
//! marker shows, so nothing waits a fixed time.
//!
//! A readback becomes a `termelon-vt` screen, so [`compare()`] can hold it against the screen
//! `termelon-vt` computes for the bytes the program wrote. Text and colours count, and attributes
//! and the cursor count where the terminal reports them.
//!
//! | Terminal | Started with | Keys | Read back with |
//! |---|---|---|---|
//! | tmux | `new-session -d -x -y` on a server of its own | `send-keys` | `capture-pane -p -e -N`, and the cursor from `display-message` |
//! | GNU screen | screen attached in a tmux pane of the size | the pane's `send-keys`, bytes with `stuff` | `hardcopy`, text only |
//! | kitty | `--listen-on` with remote control, under X | `kitten @ send-key`, bytes with `send-text` | `kitten @ get-text --ansi --add-cursor` |
//! | xterm | `-geometry` and a bitmap font, under X | xdotool | the `CSI 10 i` XHTML dump |
//! | WezTerm | `start --always-new-process`, under X | xdotool, bytes with `cli send-text` | `cli get-text --escapes` and `cli list` |
//! | xterm.js | `@xterm/headless` on `node-pty`, by a node script | bytes, and mouse events through its mouse service | the buffer, as SGR rows |
//! | VTE | a `Vte.Terminal` in an offscreen GTK window, by a Python script | GTK key and mouse events, bytes with `feed_child` | `get_text_format(HTML)` |
//! | Alacritty | `--ref-test`, under X | xdotool | `grid.json`, as the window closes |
//! | Konsole | `--separate --nofork`, under X | xdotool, bytes with D-Bus `sendText` | D-Bus `getAllDisplayedText` |
//! | foot | `--window-size-chars`, under Wayland | wtype | `pipe-visible` on a key binding |
//! | Ghostty | the app's binary with flags, on macOS | System Events | `write_screen_file:copy` on a key binding |
//! | Windows Terminal | `--size`, with settings of the harness's | `SendInput`, bytes with `WriteConsoleInputW` | the `exportBuffer` action on a key binding |
//! | conhost | `CREATE_NEW_CONSOLE`, sized by `harness-console` | `WriteConsoleInputW` | `ReadConsoleOutputW` through `AttachConsole` |

pub mod compare;
pub mod driver;
pub mod drivers;
pub mod expected;
pub mod input;
pub mod json;
pub mod keys;
pub mod readback;
pub mod run;
pub mod script;
#[cfg(windows)]
pub mod windows;

pub use compare::{Mismatch, compare};
pub use driver::{Attributes, Colors, Driver, Error, Launch, Readback, Reports};
