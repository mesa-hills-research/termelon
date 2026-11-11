//! A helper program that runs the terminal and answers JSON requests, one per line, on its
//! standard input and output. The xterm.js and VTE drivers talk to theirs this way.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use termelon_vt::{Mouse, MouseButton, MouseKind};

use crate::driver::{Error, describe};
use crate::json::Json;

/// How long the helper may take to answer.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// A running helper.
#[derive(Debug)]
pub struct Helper {
    name: &'static str,
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<String>,
}

impl Helper {
    /// Starts a helper, its standard error going to `log`, and waits for its first line, which
    /// must be `{"ready":true}`.
    ///
    /// # Errors
    ///
    /// When it can't start, or begins with anything else.
    pub fn start(name: &'static str, mut command: Command, log: &Path) -> Result<Helper, Error> {
        let described = describe(&command);
        let log =
            std::fs::File::create(log).map_err(|error| Error::Io(described.clone(), error))?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log);
        let mut child = command
            .spawn()
            .map_err(|error| Error::Io(described, error))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(Error::Missing(format!("{name}'s pipes")));
        };
        let (sender, replies) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut helper = Helper {
            name,
            child,
            stdin,
            replies,
        };
        let ready = helper.reply()?;
        if ready.get("ready").and_then(Json::as_bool) != Some(true) {
            return Err(Error::Parse(format!(
                "{name} began with {}",
                ready.to_text().trim()
            )));
        }
        Ok(helper)
    }

    /// Sends a request and returns the answer.
    ///
    /// # Errors
    ///
    /// When the helper is gone or doesn't answer in time.
    pub fn request(&mut self, request: &Json) -> Result<Json, Error> {
        let mut line = request.to_text().replace('\n', "");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .map_err(|error| Error::Io(self.name.to_owned(), error))?;
        self.reply()
    }

    fn reply(&mut self) -> Result<Json, Error> {
        let line = self
            .replies
            .recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| Error::Parse(format!("{} didn't answer", self.name)))?;
        Json::parse(&line)
            .map_err(|error| Error::Parse(format!("{} wrote {line:?}: {error}", self.name)))
    }

    /// Asks the helper to quit, then makes sure it has.
    pub fn stop(mut self) {
        let _ = self.stdin.write_all(b"{\"quit\":true}\n");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A mouse event as the helpers take it, such as
/// `{"mouse":"press","button":"left","x":3,"y":2}`, with the cell counted from 0. The wheel is
/// `scroll` with `up` or `down`.
///
/// # Errors
///
/// For a drag, a move or a tilt of the wheel, which the helpers don't make.
pub fn mouse_request(mouse: Mouse) -> Result<Json, Error> {
    let button = |button| match button {
        MouseButton::Left => "left",
        MouseButton::Middle => "middle",
        MouseButton::Right => "right",
    };
    let (what, button) = match mouse.kind {
        MouseKind::Press(b) => ("press", button(b)),
        MouseKind::Release(b) => ("release", button(b)),
        MouseKind::ScrollUp => ("scroll", "up"),
        MouseKind::ScrollDown => ("scroll", "down"),
        _ => return Err(Error::Unsupported("make that mouse event")),
    };
    Ok(Json::object([
        ("mouse", Json::str(what)),
        ("button", Json::str(button)),
        ("x", Json::Int(i64::from(mouse.x))),
        ("y", Json::Int(i64::from(mouse.y))),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_requests() {
        let press = mouse_request(Mouse::new(MouseKind::Press(MouseButton::Right), 3, 2)).unwrap();
        assert_eq!(
            press.to_text().replace(['\n', ' '], ""),
            r#"{"mouse":"press","button":"right","x":3,"y":2}"#
        );
        let wheel = mouse_request(Mouse::new(MouseKind::ScrollDown, 0, 0)).unwrap();
        assert_eq!(wheel.get("button").and_then(Json::as_str), Some("down"));
        assert!(mouse_request(Mouse::new(MouseKind::Move, 0, 0)).is_err());
    }
}
