//! A small JSON writer for the harness's reports, and a reader for what terminals' tools write.

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
    /// A number with a fraction or an exponent.
    Float(f64),
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
            Json::Float(value) => {
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

impl Json {
    /// Reads JSON text.
    ///
    /// # Errors
    ///
    /// When the text isn't one JSON value, with where it went wrong.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut reader = Reader {
            bytes: text.as_bytes(),
            at: 0,
        };
        let value = reader.value(0)?;
        reader.space();
        if reader.at != reader.bytes.len() {
            return Err(reader.error("text after the value"));
        }
        Ok(value)
    }

    /// A field of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// An item of a list.
    pub fn at(&self, index: usize) -> Option<&Json> {
        match self {
            Json::List(items) => items.get(index),
            _ => None,
        }
    }

    /// The items of a list, or none.
    pub fn items(&self) -> &[Json] {
        match self {
            Json::List(items) => items,
            _ => &[],
        }
    }

    /// A string's text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(text) => Some(text),
            _ => None,
        }
    }

    /// A whole number.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// A boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

/// The deepest nesting the reader takes, so a hostile file can't exhaust the stack.
const MAX_DEPTH: usize = 128;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} at byte {}", self.at)
    }

    fn space(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.space();
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("an unknown word"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("values nested too deep"));
        }
        self.space();
        match self.bytes.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut fields = Vec::new();
                if self.eat(b'}') {
                    return Ok(Json::Object(fields));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    if !self.eat(b':') {
                        return Err(self.error("no `:` after a key"));
                    }
                    fields.push((key, self.value(depth + 1)?));
                    if self.eat(b'}') {
                        return Ok(Json::Object(fields));
                    }
                    if !self.eat(b',') {
                        return Err(self.error("no `,` between fields"));
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Ok(Json::List(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.eat(b']') {
                        return Ok(Json::List(items));
                    }
                    if !self.eat(b',') {
                        return Err(self.error("no `,` between items"));
                    }
                }
            }
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("no value")),
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || b"+-.eE".contains(b))
        {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| self.error("a bad number"))?;
        if let Ok(value) = text.parse::<i64>() {
            return Ok(Json::Int(value));
        }
        text.parse::<f64>()
            .map(Json::Float)
            .map_err(|_| self.error("a bad number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| self.error("a short \\u escape"))?;
        self.at += 4;
        std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .ok_or_else(|| self.error("a bad \\u escape"))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return Err(self.error("no string"));
        }
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return Err(self.error("an unended string"));
            };
            self.at += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let Some(&escape) = self.bytes.get(self.at) else {
                        return Err(self.error("an unended escape"));
                    };
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xd800..0xdc00).contains(&code)
                                && self.bytes[self.at..].starts_with(b"\\u")
                            {
                                self.at += 2;
                                let low = self.hex4()?;
                                code = 0x10000
                                    + ((code - 0xd800) << 10)
                                    + (low.wrapping_sub(0xdc00) & 0x3ff);
                            }
                            char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER)
                        }
                        _ => return Err(self.error("an unknown escape")),
                    };
                    let mut buf = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                byte => out.push(byte),
            }
        }
        String::from_utf8(out).map_err(|_| self.error("a string that isn't UTF-8"))
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
    fn reads_what_it_writes_and_more() {
        let json =
            Json::parse(r#" {"a": [1, -2.5, true, null, "x\"\n\u00e9\ud83d\ude00"], "b": {}} "#)
                .unwrap();
        assert_eq!(
            json.get("a").and_then(|a| a.at(0)).and_then(Json::as_i64),
            Some(1)
        );
        assert_eq!(
            json.get("a").and_then(|a| a.at(1)),
            Some(&Json::Float(-2.5))
        );
        assert_eq!(
            json.get("a").and_then(|a| a.at(4)).and_then(Json::as_str),
            Some("x\"\né😀")
        );
        assert_eq!(json.get("b"), Some(&Json::Object(Vec::new())));
        let written = Json::object([
            ("k", Json::str("v\u{1}")),
            ("n", Json::List(vec![Json::Int(3)])),
        ]);
        assert_eq!(Json::parse(&written.to_text()).unwrap(), written);
        assert!(Json::parse("[1,").is_err());
        assert!(Json::parse("{} x").is_err());
        assert!(Json::parse(&"[".repeat(500)).is_err());
    }

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
