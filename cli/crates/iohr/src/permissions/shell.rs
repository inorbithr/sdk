//! Reading a shell command line closely enough to match permission rules against it.
//!
//! This is not a shell. It does the one thing a rule needs: cut a line into its simple
//! commands (at `;`, `&`, `&&`, `|`, `||` and a newline, but never inside quotes) and say
//! whether the line does anything a prefix rule cannot see: a command substitution
//! (`$(...)`, backticks), process substitution, `${...}`, a here-document or a redirect that
//! writes. Such a line is never allowed by a rule; the person is asked, or it is denied.
//! Claude Code splits compound commands the same way before it matches a rule
//! (<https://code.claude.com/docs/en/permissions>, "Compound commands").

/// One simple command of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    /// The command as written, trimmed.
    pub(crate) text: String,
    /// Its words with the quotes taken off.
    pub(crate) words: Vec<String>,
}

/// A command line cut into simple commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) parts: Vec<Part>,
    /// The line does something a rule cannot see (see the module docs).
    pub(crate) opaque: bool,
}

/// Redirects that write nowhere a rule would care about (after the file descriptor digit).
const HARMLESS: [&str; 3] = [">&1", ">&2", ">/dev/null"];

/// Cuts `cmd` into simple commands.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the characters keeps quotes, escapes and operators in one place"
)]
pub(crate) fn split(cmd: &str) -> Line {
    let chars: Vec<char> = cmd.chars().collect();
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut opaque = false;
    let (mut single, mut double) = (false, false);
    let mut i = 0;
    let flush_word = |word: &mut String, in_word: &mut bool, words: &mut Vec<String>| {
        if *in_word {
            words.push(std::mem::take(word));
            *in_word = false;
        }
    };
    let flush_part = |text: &mut String, words: &mut Vec<String>, parts: &mut Vec<Part>| {
        let t = text.trim();
        if t.is_empty() {
            words.clear();
        } else {
            parts.push(Part {
                text: t.to_owned(),
                words: std::mem::take(words),
            });
        }
        text.clear();
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if single {
            text.push(c);
            if c == '\'' {
                single = false;
            } else {
                word.push(c);
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            text.push(c);
            if let Some(n) = next {
                text.push(n);
                word.push(n);
                in_word = true;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        // These expand in double quotes too; a variable could hold anything.
        if c == '`' || c == '$' {
            opaque = true;
        }
        if double {
            text.push(c);
            if c == '"' {
                double = false;
            } else {
                word.push(c);
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => {
                single = true;
                in_word = true;
                text.push(c);
            }
            '"' => {
                double = true;
                in_word = true;
                text.push(c);
            }
            ';' | '\n' | '|' | '&' => {
                // `&>` and `>&` are redirects (handled below); everything else separates.
                if c == '&' && next == Some('>') {
                    opaque = true;
                    text.push(c);
                    i += 1;
                    continue;
                }
                flush_word(&mut word, &mut in_word, &mut words);
                flush_part(&mut text, &mut words, &mut parts);
                // `&&`, `||`, `|&` are one operator.
                if matches!(next, Some('&' | '|')) && c != ';' && c != '\n' {
                    i += 1;
                }
            }
            '<' | '>' => {
                let rest: String = chars[i..].iter().collect();
                // A redirect to nowhere (`2>&1`, `>/dev/null`) stays with its command; the
                // file descriptor digit already read as a word goes with it.
                if let Some(r) = HARMLESS.iter().find(|r| rest.starts_with(**r)) {
                    if matches!(word.as_str(), "1" | "2") {
                        word.clear();
                        in_word = false;
                    }
                    text.push_str(r);
                    i += r.chars().count();
                    continue;
                }
                opaque = true;
                text.push(c);
                word.push(c);
                in_word = true;
            }
            c if c.is_whitespace() => {
                flush_word(&mut word, &mut in_word, &mut words);
                text.push(c);
            }
            c => {
                text.push(c);
                word.push(c);
                in_word = true;
            }
        }
        i += 1;
    }
    if single || double {
        opaque = true;
    }
    flush_word(&mut word, &mut in_word, &mut words);
    flush_part(&mut text, &mut words, &mut parts);
    Line { parts, opaque }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(l: &Line) -> Vec<&str> {
        l.parts.iter().map(|p| p.text.as_str()).collect()
    }

    #[test]
    fn a_plain_command_is_one_part() {
        let l = split("ls -la");
        assert_eq!(texts(&l), ["ls -la"]);
        assert_eq!(l.parts[0].words, ["ls", "-la"]);
        assert!(!l.opaque);
    }

    #[test]
    fn operators_cut_the_line_but_quotes_do_not() {
        let l = split("cd a && ls | wc -l; echo 'x; y' \"a && b\"\npwd");
        assert_eq!(
            texts(&l),
            ["cd a", "ls", "wc -l", "echo 'x; y' \"a && b\"", "pwd"]
        );
        assert!(!l.opaque);
        assert_eq!(l.parts[3].words, ["echo", "x; y", "a && b"]);
    }

    #[test]
    fn substitutions_and_redirects_make_a_line_opaque() {
        for cmd in [
            "ls $(rm -rf x)",
            "ls `id`",
            "echo \"$(id)\"",
            "ls ${HOME}",
            "cat $HOME/.ssh/id_rsa",
            "ls > out",
            "cat < in",
            "cat <<EOF",
            "diff <(ls) b",
            "ls &> out",
            "echo 'unclosed",
        ] {
            assert!(split(cmd).opaque, "{cmd}");
        }
    }

    #[test]
    fn single_quotes_hide_substitution() {
        assert!(!split("echo '$(id)'").opaque);
    }

    #[test]
    fn harmless_redirects_stay_with_the_command() {
        for cmd in [
            "ls 2>&1",
            "ls >/dev/null",
            "ls 2>/dev/null",
            "ls -la 2>&1 | head",
        ] {
            let l = split(cmd);
            assert!(!l.opaque, "{cmd}");
            assert!(l.parts[0].text.starts_with("ls"), "{cmd}");
        }
        assert_eq!(texts(&split("ls -la 2>&1 | head")), ["ls -la 2>&1", "head"]);
    }

    #[test]
    fn an_empty_line_has_no_parts() {
        assert_eq!(split("  ; ").parts.len(), 0);
    }
}
