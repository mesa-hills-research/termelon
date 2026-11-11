//! The probe's report: what the terminal is and supports, as the framework found it, and what came
//! in while the scene showed. It's JSON, written with a few lines of our own.

use std::fmt::Debug;
use std::fmt::Write as _;

use termelon::{Capabilities, Event, Size};

/// A JSON string.
fn string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn debug(value: &impl Debug) -> String {
    string(&format!("{value:?}"))
}

/// What a run found.
#[derive(Debug, Default)]
pub(crate) struct Report {
    pub(crate) scene: String,
    pub(crate) size: Option<Size>,
    pub(crate) asked_size: Option<(u16, u16)>,
    pub(crate) capabilities: Option<Capabilities>,
    pub(crate) events: Vec<Event>,
    pub(crate) error: Option<String>,
}

impl Report {
    /// The report as JSON.
    pub(crate) fn to_json(&self) -> String {
        let mut fields = vec![format!("\"scene\": {}", string(&self.scene))];
        let pair = |(c, r): (u16, u16)| format!("[{c}, {r}]");
        fields.push(format!(
            "\"size\": {}",
            self.size
                .map_or("null".to_owned(), |s| pair((s.width, s.height)))
        ));
        fields.push(format!(
            "\"asked_size\": {}",
            self.asked_size.map_or("null".to_owned(), pair)
        ));
        if let Some(caps) = &self.capabilities {
            let terminal = format!(
                "{{\"name\": {}, \"version\": {}}}",
                caps.terminal
                    .name
                    .as_deref()
                    .map_or("null".to_owned(), string),
                caps.terminal
                    .version
                    .as_deref()
                    .map_or("null".to_owned(), string)
            );
            let supports = [
                ("probe", debug(&caps.probe)),
                ("multiplexer", debug(&caps.multiplexer)),
                ("colors", debug(&caps.colors)),
                ("width_profile", debug(&caps.width_profile)),
                ("alternate_screen", debug(&caps.alternate_screen)),
                ("keyboard", debug(&caps.keyboard)),
                ("synchronized_output", debug(&caps.synchronized_output)),
                ("grapheme_clustering", debug(&caps.grapheme_clustering)),
                ("hyperlinks", debug(&caps.hyperlinks)),
                ("underline_styles", debug(&caps.underline_styles)),
                ("underline_color", debug(&caps.underline_color)),
                ("focus_events", debug(&caps.focus_events)),
                ("bracketed_paste", debug(&caps.bracketed_paste)),
                ("mouse", debug(&caps.mouse)),
                ("clipboard_write", debug(&caps.clipboard_write)),
                ("cursor_shape", debug(&caps.cursor_shape)),
                ("title", debug(&caps.title)),
                ("title_stack", debug(&caps.title_stack)),
                ("background", debug(&caps.background)),
                ("glyphs", debug(&caps.glyphs)),
            ];
            let supports: Vec<String> = supports
                .iter()
                .map(|(key, value)| format!("    \"{key}\": {value}"))
                .collect();
            fields.push(format!("\"terminal\": {terminal}"));
            fields.push(format!(
                "\"capabilities\": {{\n{}\n  }}",
                supports.join(",\n")
            ));
        }
        let events: Vec<String> = self
            .events
            .iter()
            .map(|event| format!("    {}", debug(event)))
            .collect();
        fields.push(if events.is_empty() {
            "\"events\": []".to_owned()
        } else {
            format!("\"events\": [\n{}\n  ]", events.join(",\n"))
        });
        fields.push(format!(
            "\"error\": {}",
            self.error.as_deref().map_or("null".to_owned(), string)
        ));
        format!("{{\n  {}\n}}\n", fields.join(",\n  "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_json() {
        let report = Report {
            scene: "text".to_owned(),
            size: Some(Size::new(80, 24)),
            asked_size: Some((80, 25)),
            error: Some("the terminal is 80x24, not 80x25 \"as asked\"".to_owned()),
            ..Report::default()
        };
        let json = report.to_json();
        assert!(json.starts_with(
            "{\n  \"scene\": \"text\",\n  \"size\": [80, 24],\n  \"asked_size\": [80, 25],"
        ));
        assert!(json.contains("\"events\": [],"));
        assert!(json.contains("\"error\": \"the terminal is 80x24, not 80x25 \\\"as asked\\\"\""));
    }
}
