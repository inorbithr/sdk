//! Finding one entry in a monitor file's text, and cutting it out with everything else kept
//! byte for byte (RFC 0008, ADR 0009).
//!
//! An entry is a `[[check]]` or `[[refuse]]` header with the lines under it, its
//! `[[check.steps]]` included, up to the next entry. The comment lines right above the header,
//! with no blank line between, belong to it; a comment after a blank line, at the top of the
//! file or between entries, belongs to no entry and stays. The cut is checked by parsing: the
//! file after it must hold every other entry unchanged and this one no more, else nothing is cut.

use serde_json::Value;

/// Where one entry sits in a file's text, as 0-based line numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Its first line: the first comment line above the header, else the header.
    pub start: usize,
    /// The header line, `[[check]]` or `[[refuse]]`.
    pub header: usize,
    /// One past its last line, the blank lines after it included.
    pub end: usize,
}

/// The kind of a header line: `check` or `refuse` for an entry, `sub` for a table inside one
/// (`[[check.steps]]`), `other` for any other table, `None` for a line that is no header.
fn header(line: &str) -> Option<&'static str> {
    let t = line.trim();
    let t = t.split_once('#').map_or(t, |(h, _)| h.trim_end());
    match t {
        "[[check]]" => Some("check"),
        "[[refuse]]" => Some("refuse"),
        _ if t.starts_with("[[check.") || t.starts_with("[[refuse.") || t.starts_with("[check.") || t.starts_with("[refuse.") => Some("sub"),
        _ if t.starts_with('[') && t.ends_with(']') && !t.contains('=') => Some("other"),
        _ => None,
    }
}

/// `name = "x"` on this line gives `x`.
fn name_of(line: &str) -> Option<String> {
    let (k, v) = line.split_once('=')?;
    if k.trim() != "name" {
        return None;
    }
    let v = v.trim();
    let v = v.split_once(" #").map_or(v, |(a, _)| a.trim_end());
    let s: String = toml::from_str::<toml::Table>(&format!("v = {v}")).ok()?.get("v")?.as_str()?.to_owned();
    Some(s)
}

/// Where the entry named `name` is in `text`.
#[must_use]
pub fn locate(text: &str, name: &str) -> Option<Span> {
    let lines: Vec<&str> = text.lines().collect();
    let heads: Vec<usize> = (0..lines.len()).filter(|&i| matches!(header(lines[i]), Some("check" | "refuse"))).collect();
    for h in heads {
        // The entry's own keys run to its first sub-table or the next table of any kind.
        let own_end = (h + 1..lines.len()).find(|&i| header(lines[i]).is_some()).unwrap_or(lines.len());
        if !lines[h + 1..own_end].iter().any(|l| name_of(l).as_deref() == Some(name)) {
            continue;
        }
        let mut start = h;
        while start > 0 && lines[start - 1].trim_start().starts_with('#') {
            start -= 1;
        }
        // It ends before the next entry or other table, less the comments that belong to it.
        let next = (h + 1..lines.len()).find(|&i| matches!(header(lines[i]), Some("check" | "refuse" | "other"))).unwrap_or(lines.len());
        let mut last = next;
        while last > h + 1 && {
            let l = lines[last - 1].trim();
            l.is_empty() || l.starts_with('#')
        } {
            last -= 1;
        }
        let mut end = last;
        while end < next && lines[end].trim().is_empty() {
            end += 1;
        }
        return Some(Span { start, header: h, end });
    }
    None
}

/// `text` without the entry named `name`, every other line kept as it was, and the lines cut.
/// Refused when the entry is not there, or when the file after the cut would not hold exactly
/// the other entries unchanged (a header inside a multi-line string, say).
pub fn cut(text: &str, name: &str) -> Result<(String, String), String> {
    let span = locate(text, name).ok_or_else(|| format!("no entry named {name} in this file"))?;
    let lines: Vec<&str> = text.lines().collect();
    let mut kept: Vec<&str> = lines[..span.start].iter().chain(&lines[span.end..]).copied().collect();
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    let removed = lines[span.start..span.end].join("\n").trim_end().to_owned() + "\n";
    let entries = |t: &str| -> Result<Vec<Value>, String> {
        let v = super::parse(t)?;
        Ok(["check", "refuse"].iter().flat_map(|k| v.get(*k).and_then(Value::as_array).cloned().unwrap_or_default()).collect())
    };
    let before = entries(text)?;
    let after = entries(&out).map_err(|e| format!("the file would not parse after the cut ({e}); edit it by hand"))?;
    let want: Vec<&Value> = before.iter().filter(|e| e.get("name").and_then(Value::as_str) != Some(name)).collect();
    if after.iter().collect::<Vec<_>>() != want {
        return Err("the cut would change other entries; edit the file by hand".into());
    }
    Ok((out, removed))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const FILE: &str = "# The file's own note.\n# Two lines.\n\n[[check]]\nname = \"api\"\ntarget = \"http://127.0.0.1:7171/api/health\"\nevery = \"60s\"\n\n# Belongs to cli.\n[[check]]\nname = \"cli\" # the binary\ntarget = { cli = \"eye\" }\nexpect = { max_ms = 100 }\n\n[[check.steps]]\nargv = [\n  \"scrum\",\n  \"--json\",\n]\n\n# A free note about the next part.\n\n[[refuse]]\nname = \"admin\"\ntarget = \"http://127.0.0.1:7171/admin\"\nevery = \"5m\"\nexpect = { status = 400 }\n";

    #[test]
    fn an_entry_is_found_with_its_comment_and_steps_and_cut_keeping_everything_else() {
        let s = locate(FILE, "cli").unwrap();
        let lines: Vec<&str> = FILE.lines().collect();
        assert_eq!(lines[s.start], "# Belongs to cli.");
        assert_eq!(lines[s.header], "[[check]]");
        assert_eq!(lines[s.end], "# A free note about the next part.");

        let (out, removed) = cut(FILE, "cli").unwrap();
        assert!(removed.starts_with("# Belongs to cli.\n[[check]]\nname = \"cli\"") && removed.contains("[[check.steps]]"), "{removed}");
        assert_eq!(out, FILE.replace(&removed, "").replace("\n\n\n", "\n\n"));
        assert!(out.contains("# The file's own note.") && out.contains("# A free note about the next part."));

        let (out, removed) = cut(FILE, "admin").unwrap();
        assert!(out.ends_with("# A free note about the next part.\n"), "{out}");
        assert!(removed.starts_with("[[refuse]]"));

        let (out, _) = cut(FILE, "api").unwrap();
        assert!(out.starts_with("# The file's own note.\n# Two lines.\n\n# Belongs to cli.\n[[check]]"), "{out}");

        assert!(cut(FILE, "nope").unwrap_err().contains("no entry named nope"));
        assert!(locate(FILE, "argv").is_none());
    }

    #[test]
    fn a_cut_that_would_change_another_entry_is_refused() {
        let tricky = "[[check]]\nname = \"a\"\nnote = \"\"\"\n[[check]]\nname = \"b\"\n\"\"\"\ntarget = \"x:1\"\n";
        assert!(cut(tricky, "b").is_err(), "a header inside a string is not an entry");
    }
}
