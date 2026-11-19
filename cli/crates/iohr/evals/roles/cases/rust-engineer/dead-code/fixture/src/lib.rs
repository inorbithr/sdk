//! URL slugs for article titles.

/// The slug of `title`: lowercase ASCII letters and digits joined by `-`.
#[must_use]
pub fn slug(title: &str) -> String {
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

#[allow(dead_code)]
fn legacy_slug(title: &str) -> String {
    title.to_lowercase().replace(' ', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Hello, World 2"), "hello-world-2");
    }
}
