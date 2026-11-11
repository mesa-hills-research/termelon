//! Expected reports. Each keeps what a terminal of one version answered the probe, so a run that
//! gets another answer shows the change.
//!
//! A report holds the terminal's name and version as it gave them and the capabilities the
//! framework found, its widths among them. The keys it sends are kept apart, in the input
//! fixtures [`crate::input`] writes.

use std::path::{Path, PathBuf};

use crate::input::version_slug;
use crate::json::Json;

/// Where the harness keeps its expected reports.
pub fn default_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected")
}

/// The expected report of a terminal's version, such as `tmux-3.6.json`.
pub fn path(dir: &Path, terminal: &str, version: &str) -> PathBuf {
    dir.join(format!("{terminal}-{}.json", version_slug(version)))
}

/// The answers in a probe's report, or `None` where the probe couldn't open the terminal.
pub fn answers(report: &Json) -> Option<Json> {
    Some(Json::object([
        ("terminal", report.get("terminal")?.clone()),
        ("capabilities", report.get("capabilities")?.clone()),
    ]))
}

/// How `got` differs from `expected`, a line per field, such as
/// `capabilities.keyboard: "Kitty" before, "Legacy" now`.
pub fn changes(expected: &Json, got: &Json) -> Vec<String> {
    let mut out = Vec::new();
    diff("", expected, got, &mut out);
    out
}

fn diff(at: &str, expected: &Json, got: &Json, out: &mut Vec<String>) {
    let field = |key: &str| {
        if at.is_empty() {
            key.to_owned()
        } else {
            format!("{at}.{key}")
        }
    };
    match (expected, got) {
        (Json::Object(before), Json::Object(now)) => {
            for (key, value) in before {
                match got.get(key) {
                    Some(other) => diff(&field(key), value, other, out),
                    None => out.push(format!("{}: gone", field(key))),
                }
            }
            for (key, value) in now {
                if expected.get(key).is_none() {
                    out.push(format!("{}: new, {}", field(key), text(value)));
                }
            }
        }
        _ if expected == got => {}
        _ => out.push(format!(
            "{}: {} before, {} now",
            if at.is_empty() { "the report" } else { at },
            text(expected),
            text(got)
        )),
    }
}

fn text(value: &Json) -> String {
    value.to_text().trim().replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(keyboard: &str) -> Json {
        Json::parse(&format!(
            r#"{{"scene": "text", "terminal": {{"name": "tmux", "version": "3.6"}},
                "capabilities": {{"keyboard": "{keyboard}", "mouse": "Sgr"}}, "events": []}}"#
        ))
        .unwrap()
    }

    #[test]
    fn answers_leave_out_the_scene_and_events() {
        let kept = answers(&report("Legacy")).unwrap();
        assert!(kept.get("scene").is_none());
        assert!(kept.get("events").is_none());
        assert_eq!(
            kept.get("capabilities")
                .and_then(|caps| caps.get("mouse"))
                .and_then(Json::as_str),
            Some("Sgr")
        );
        assert_eq!(
            answers(&Json::parse(r#"{"error": "no tty"}"#).unwrap()),
            None
        );
    }

    #[test]
    fn changes_name_the_field() {
        let before = answers(&report("Legacy")).unwrap();
        let now = answers(&report("Kitty")).unwrap();
        assert_eq!(changes(&before, &before), Vec::<String>::new());
        assert_eq!(
            changes(&before, &now),
            [r#"capabilities.keyboard: "Legacy" before, "Kitty" now"#]
        );
    }

    #[test]
    fn paths_name_the_version() {
        assert_eq!(
            path(Path::new("e"), "vte", "VTE 0.84.0"),
            Path::new("e/vte-0.84.0.json")
        );
    }
}
