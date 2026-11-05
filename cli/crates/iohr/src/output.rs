use std::fmt::Write as _;
use std::io::{self, Write as _};

use serde::Serialize;

/// Where results go: tables or JSON on stdout, notes on stderr.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Out {
    pub(crate) json: bool,
}

impl Out {
    /// Prints `value` as pretty JSON.
    pub(crate) fn print_json<T: Serialize>(value: &T) {
        let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into());
        Self::raw(format!("{text}\n").as_bytes());
    }

    /// Prints rows under a header, columns aligned.
    pub(crate) fn table(header: &[&str], rows: &[Vec<String>]) {
        Self::raw(table_text(header, rows).as_bytes());
    }

    /// Prints `key  value` pairs.
    pub(crate) fn pairs(pairs: &[(&str, String)]) {
        let w = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        let mut text = String::new();
        for (k, v) in pairs {
            let _ = writeln!(text, "{k:w$}  {v}");
        }
        Self::raw(text.as_bytes());
    }

    /// Writes bytes to stdout as they are. A closed pipe (`| head`) is not an error.
    pub(crate) fn raw(bytes: &[u8]) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(bytes).and_then(|()| out.flush());
    }

    /// A note for the person, on stderr, so stdout stays clean for pipes.
    pub(crate) fn note(line: &str) {
        let _ = writeln!(io::stderr().lock(), "{line}");
    }
}

/// The text of a table: columns aligned by what is shown, so a cell holding colour codes
/// lines up with a plain one. No trailing spaces.
pub(crate) fn table_text(header: &[&str], rows: &[Vec<String>]) -> String {
    let shown = |s: &str| console::measure_text_width(s);
    let mut width: Vec<usize> = header.iter().map(|h| shown(h)).collect();
    for row in rows {
        for (w, cell) in width.iter_mut().zip(row) {
            *w = (*w).max(shown(cell));
        }
    }
    let line = |cells: &mut dyn Iterator<Item = &str>| {
        let mut s = String::new();
        for (i, (cell, w)) in cells.zip(&width).enumerate() {
            if i > 0 {
                s.push_str("  ");
            }
            s.push_str(cell);
            if i + 1 < width.len() {
                s.extend(std::iter::repeat_n(' ', w.saturating_sub(shown(cell))));
            }
        }
        // Only blanks are trimmed: a colour code at the end of a line stays.
        s.truncate(s.trim_end_matches(' ').len());
        s.push('\n');
        s
    };
    let mut text = line(&mut header.iter().copied());
    for row in rows {
        text.push_str(&line(&mut row.iter().map(String::as_str)));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::table_text;

    #[test]
    fn colour_codes_do_not_move_the_columns() {
        let rows = vec![
            vec![
                "a".to_owned(),
                "\u{1b}[32m\u{25cf} live\u{1b}[0m".to_owned(),
                "x".to_owned(),
            ],
            vec!["bb".to_owned(), "coming".to_owned(), "y".to_owned()],
        ];
        let plain = |s: &str| console::strip_ansi_codes(s).into_owned();
        let text = table_text(&["N", "STATUS", "Z"], &rows);
        let lines: Vec<String> = text.lines().map(plain).collect();
        assert_eq!(lines[0], "N   STATUS  Z");
        assert_eq!(lines[1], "a   \u{25cf} live  x");
        assert_eq!(lines[2], "bb  coming  y");
    }
}
