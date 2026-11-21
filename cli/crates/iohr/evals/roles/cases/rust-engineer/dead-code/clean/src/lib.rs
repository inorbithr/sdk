//! URL slugs for article titles.

/// The slug of `title`: lowercase ASCII letters and digits joined by `-`.
#[must_use]
pub fn slug(title: &str) -> String {
    slug_max(title, usize::MAX)
}

/// The slug of `title`, cut to at most `max` bytes at a word boundary.
#[must_use]
pub fn slug_max(title: &str, max: usize) -> String {
    let mut out = String::new();
    for w in title.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let extra = w.len() + usize::from(!out.is_empty());
        if out.len() + extra > max {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w.to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Hello, World 2"), "hello-world-2");
        assert_eq!(slug_max("Hello, World 2", 11), "hello-world");
    }
}
