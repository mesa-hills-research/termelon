//! A small JSON writer for the harness's reports.

use std::fmt::Write as _;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A string.
    Str(String),
    /// A list.
    List(Vec<Json>),
    /// An object, its fields in the order given.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// An object from fields.
    pub fn object<const N: usize>(fields: [(&str, Json); N]) -> Json {
        Json::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    }

    /// A string value.
    pub fn str(text: impl Into<String>) -> Json {
        Json::Str(text.into())
    }

    /// The value as JSON text, indented by two spaces, ending with a newline.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Int(value) => {
                let _ = write!(out, "{value}");
            }
            Json::Str(text) => string(out, text),
            Json::List(items) if items.is_empty() => out.push_str("[]"),
            Json::Object(fields) if fields.is_empty() => out.push_str("{}"),
            Json::List(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    item.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    string(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_nested_values() {
        let json = Json::object([
            ("name", Json::str("tmux \"3.6\"")),
            ("ok", Json::Bool(false)),
            (
                "rows",
                Json::List(vec![Json::str("a\u{1b}b"), Json::Int(-3)]),
            ),
            ("none", Json::List(Vec::new())),
            ("missing", Json::Null),
        ]);
        assert_eq!(
            json.to_text(),
            "{\n  \"name\": \"tmux \\\"3.6\\\"\",\n  \"ok\": false,\n  \"rows\": [\n    \"a\\u001bb\",\n    -3\n  ],\n  \
             \"none\": [],\n  \"missing\": null\n}\n"
        );
    }
}
