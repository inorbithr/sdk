//! The two small matchers rules need: a path glob and a command wildcard. No regular
//! expressions and no recursion that can blow up: both walk the text once per `*`.

/// Gitignore-style path glob: `*` and `?` stay inside one path segment, `**` crosses
/// segments, and `**/` also matches no folder at all (`**/.env` matches `.env`).
#[must_use]
pub(crate) fn path(pattern: &str, text: &str) -> bool {
    walk(pattern.as_bytes(), text.as_bytes())
}

fn walk(p: &[u8], t: &[u8]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some(b'*') if p.get(1) == Some(&b'*') => {
            let rest = &p[2..];
            if let Some(after) = rest.strip_prefix(b"/") {
                walk(after, t)
                    || t.iter()
                        .enumerate()
                        .any(|(i, c)| *c == b'/' && walk(after, &t[i + 1..]))
            } else {
                (0..=t.len()).any(|i| walk(rest, &t[i..]))
            }
        }
        Some(b'*') => {
            let rest = &p[1..];
            let mut i = 0;
            loop {
                if walk(rest, &t[i..]) {
                    return true;
                }
                if i < t.len() && t[i] != b'/' {
                    i += 1;
                } else {
                    return false;
                }
            }
        }
        Some(b'?') => t.first().is_some_and(|c| *c != b'/') && walk(&p[1..], &t[1..]),
        Some(c) => t.first() == Some(c) && walk(&p[1..], &t[1..]),
    }
}

/// A command wildcard: `*` matches any text, spaces and slashes included; the rest is literal.
#[must_use]
pub(crate) fn wild(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let Some((first, rest)) = parts.split_first() else {
        return false;
    };
    let Some(mut at) = text.strip_prefix(first).map(|r| text.len() - r.len()) else {
        return false;
    };
    let Some((last, middle)) = rest.split_last() else {
        return text == *first;
    };
    for m in middle {
        match text[at..].find(m) {
            Some(i) => at += i + m.len(),
            None => return false,
        }
    }
    text.len() >= at + last.len() && text[at..].ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_globs() {
        for (p, t) in [
            ("**/.env", ".env"),
            ("**/.env", "a/b/.env"),
            ("**/.env", "/home/me/proj/.env"),
            ("**/.env.*", "x/.env.local"),
            ("src/**", "src/a/b.rs"),
            ("src/*.rs", "src/a.rs"),
            ("**/*secret*", "cfg/my-secret.txt"),
            ("**/.ssh/**", "/home/me/.ssh/id_rsa"),
            ("a/?.txt", "a/b.txt"),
        ] {
            assert!(path(p, t), "{p} should match {t}");
        }
        for (p, t) in [
            ("**/.env", "a/.envx"),
            ("src/*.rs", "src/a/b.rs"),
            ("src/**", "lib/a.rs"),
            ("a/?.txt", "a/bc.txt"),
            ("**/*secret*", "secrets/a.txt"),
        ] {
            assert!(!path(p, t), "{p} should not match {t}");
        }
    }

    #[test]
    fn command_wildcards() {
        assert!(wild("git * main", "git push origin main"));
        assert!(wild("ls*", "ls -la"));
        assert!(wild("*", "anything at all"));
        assert!(wild("echo hi", "echo hi"));
        assert!(!wild("echo hi", "echo hi there"));
        assert!(!wild("git * main", "git push origin dev"));
        assert!(!wild("a*b*c", "a b"));
        assert!(wild("a*b*c", "aXXbYYc"));
    }
}
