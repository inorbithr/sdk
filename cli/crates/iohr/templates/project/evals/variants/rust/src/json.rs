//! A small strict JSON reader: the dataset is JSON Lines and the standard library has no JSON
//! reader. It reads the whole of JSON (objects, arrays, strings, numbers, true, false, null) and
//! refuses anything after the value.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = p.value(0)?;
    p.space();
    if p.at != p.bytes.len() {
        return Err(format!("unexpected text at column {}", p.at + 1));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at column {}", self.at + 1))
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            self.fail("unexpected text")
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 64 {
            return self.fail("nested too deep");
        }
        self.space();
        match self.bytes.get(self.at) {
            None => self.fail("unexpected end"),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.fail("unexpected character"),
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        while matches!(
            self.bytes.get(self.at),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).unwrap_or_default();
        match text.parse::<f64>() {
            Ok(n) => Ok(Json::Number(n)),
            Err(_) => self.fail("bad number"),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let slice = self.bytes.get(self.at..self.at + 4);
        let text = slice.and_then(|s| std::str::from_utf8(s).ok());
        match text.and_then(|t| u32::from_str_radix(t, 16).ok()) {
            Some(n) => {
                self.at += 4;
                Ok(n)
            }
            None => self.fail("bad \\u escape"),
        }
    }

    fn unicode(&mut self) -> Result<char, String> {
        let mut code = self.hex4()?;
        if (0xD800..0xDC00).contains(&code) {
            if !self.bytes[self.at..].starts_with(b"\\u") {
                return self.fail("bad surrogate pair");
            }
            self.at += 2;
            let low = self.hex4()?;
            if !(0xDC00..0xE000).contains(&low) {
                return self.fail("bad surrogate pair");
            }
            code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
        }
        match char::from_u32(code) {
            Some(c) => Ok(c),
            None => self.fail("bad \\u escape"),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1; // the opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&b) = self.bytes.get(self.at) else {
                return self.fail("unterminated string");
            };
            self.at += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.bytes.get(self.at) else {
                        return self.fail("unterminated string");
                    };
                    self.at += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode()?,
                        _ => return self.fail("bad escape"),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0..=0x1f => return self.fail("control character in string"),
                _ => out.push(b),
            }
        }
        String::from_utf8(out).map_err(|_| "string is not UTF-8".to_string())
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return self.fail("expected , or ]"),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1;
        let mut map = BTreeMap::new();
        self.space();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return self.fail("expected a key");
            }
            let key = self.string()?;
            self.space();
            if self.bytes.get(self.at) != Some(&b':') {
                return self.fail("expected :");
            }
            self.at += 1;
            map.insert(key, self.value(depth + 1)?);
            self.space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(map));
                }
                _ => return self.fail("expected , or }"),
            }
        }
    }
}

/// A string as JSON text, escaped.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
