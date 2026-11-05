//! The front matter both local folders use: `atlas/` documents and `.inorbit/memory/` entries
//! (RFC 0205, ADR 0216). A line-based `key: value` reader and writer, scalars plus one-line
//! flow lists (`refs: [a, "b, c"]`). One parser, so a file written by one reads in the other.
//!
//! Pure functions over text. Text is never logged.

use sha2::{Digest as _, Sha256};

/// The time now as RFC 3339 in UTC, for the dates front matter carries.
#[must_use]
pub fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The unix seconds of an RFC 3339 time, or `None` when it does not parse.
#[must_use]
pub fn unix_of(rfc3339: &str) -> Option<i64> {
    time::OffsetDateTime::parse(
        rfc3339.trim(),
        &time::format_description::well_known::Rfc3339,
    )
    .ok()
    .map(time::OffsetDateTime::unix_timestamp)
}

/// Splits a document into its front matter (without the `---` lines) and the rest.
#[must_use]
pub fn split(text: &str) -> Option<(&str, &str)> {
    let after = text.strip_prefix("---\n")?;
    let end = after
        .find("\n---\n")
        .map(|i| (i, i + 5))
        .or_else(|| after.strip_suffix("\n---").map(|s| (s.len(), after.len())))?;
    Some((&after[..end.0], &after[end.1..]))
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return inner.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    if let Some(inner) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
    v.to_owned()
}

/// A value as a front matter scalar: plain when it can be, else double-quoted.
#[must_use]
pub fn quote(v: &str) -> String {
    let plain = !v.is_empty()
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.starts_with(|c: char| "\"'#&*!|>%@`[]{},-?".contains(c))
        && !v.ends_with(':')
        && v.trim() == v;
    if plain {
        v.to_owned()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The raw text after `key:` on its front matter line.
fn raw<'a>(fm: &'a str, key: &str) -> Option<&'a str> {
    fm.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then_some(v)
    })
}

/// The value of a front matter key, unquoted.
#[must_use]
pub fn get(text: &str, key: &str) -> Option<String> {
    let (fm, _) = split(text)?;
    raw(fm, key).map(unquote)
}

/// The items of a one-line flow list key (`key: [a, "b, c"]`), unquoted. Empty when the key
/// is missing or is not a list.
#[must_use]
pub fn get_list(text: &str, key: &str) -> Vec<String> {
    let Some((fm, _)) = split(text) else {
        return Vec::new();
    };
    let Some(v) = raw(fm, key) else {
        return Vec::new();
    };
    let Some(inner) = v.trim().strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut cur = String::new();
    let (mut in_q, mut esc) = (false, false);
    for c in inner.chars() {
        match c {
            _ if esc => {
                cur.push(c);
                esc = false;
            }
            '\\' if in_q => {
                cur.push(c);
                esc = true;
            }
            '"' => {
                cur.push(c);
                in_q = !in_q;
            }
            ',' if !in_q => items.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    items.push(cur);
    items
        .iter()
        .map(|i| unquote(i))
        .filter(|i| !i.is_empty())
        .collect()
}

/// The flow list text for `items`: `[a, "b, c"]`.
#[must_use]
pub fn list_value(items: &[String]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|i| {
            let q = quote(i);
            if q == *i && (i.contains(',') || i.contains(']')) {
                format!("\"{}\"", i.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                q
            }
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

fn set_lines(text: &str, lines_to_set: &[(&str, String)]) -> String {
    let Some((fm, rest)) = split(text) else {
        return text.to_owned();
    };
    let mut lines: Vec<String> = fm.lines().map(str::to_owned).collect();
    for (k, v) in lines_to_set {
        let line = format!("{k}: {v}");
        match lines
            .iter()
            .position(|l| l.split_once(':').is_some_and(|(lk, _)| lk.trim() == *k))
        {
            Some(i) => lines[i] = line,
            None => lines.push(line),
        }
    }
    format!("---\n{}\n---\n{rest}", lines.join("\n"))
}

/// The text with the given front matter keys set: an existing line is replaced, a missing
/// key is added at the end of the front matter. A text without front matter is returned as
/// it is.
#[must_use]
pub fn set_keys(text: &str, keys: &[(&str, String)]) -> String {
    let quoted: Vec<(&str, String)> = keys.iter().map(|(k, v)| (*k, quote(v))).collect();
    set_lines(text, &quoted)
}

/// [`set_keys`] for one flow list key.
#[must_use]
pub fn set_list(text: &str, key: &str, items: &[String]) -> String {
    set_lines(text, &[(key, list_value(items))])
}

/// The text without the front matter lines whose key is in `local`.
#[must_use]
pub fn strip_keys(text: &str, local: &[&str]) -> String {
    let Some((fm, rest)) = split(text) else {
        return text.to_owned();
    };
    let kept: Vec<&str> = fm
        .lines()
        .filter(|l| {
            l.split_once(':')
                .is_none_or(|(k, _)| !local.contains(&k.trim()))
        })
        .collect();
    format!("---\n{}\n---\n{rest}", kept.join("\n"))
}

/// The SHA-256 of `text`, in hex.
#[must_use]
pub fn sha_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_round_trip_with_commas_and_quotes() {
        let items = vec![
            "src/a.rs".to_owned(),
            "cargo test, all".to_owned(),
            "say \"hi\"".to_owned(),
        ];
        let doc = set_list("---\nid: m1\n---\nbody\n", "refs", &items);
        assert_eq!(get_list(&doc, "refs"), items);
        assert_eq!(get(&doc, "id").as_deref(), Some("m1"));
        assert!(doc.ends_with("---\nbody\n"));
    }

    #[test]
    fn a_missing_or_scalar_list_key_is_empty() {
        assert_eq!(
            get_list("---\nrefs: a\n---\n", "refs"),
            Vec::<String>::new()
        );
        assert_eq!(get_list("---\nid: 1\n---\n", "refs"), Vec::<String>::new());
        assert_eq!(get_list("no front matter", "refs"), Vec::<String>::new());
        assert_eq!(
            get_list("---\nrefs: []\n---\n", "refs"),
            Vec::<String>::new()
        );
    }
}
