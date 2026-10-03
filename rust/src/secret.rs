use std::fmt;

use zeroize::{Zeroize, Zeroizing};

/// A secret value: a key secret or an access token.
///
/// `Debug` and `Display` print `<redacted>`, the value is not `Serialize`, and its
/// memory is zeroed when it is dropped (SR-10, SR-11). Read it with
/// [`expose`](Self::expose) at the one place it is sent.
///
/// # Examples
///
/// ```
/// use inorbithr::Secret;
///
/// let token = Secret::new(String::from("eyJhbGciOi..."));
/// assert_eq!(format!("{token:?}"), "<redacted>");
/// assert_eq!(token.expose(), "eyJhbGciOi...");
/// ```
#[derive(Clone)]
pub struct Secret<T: Zeroize>(Zeroizing<T>);

impl<T: Zeroize> Secret<T> {
    /// Wraps a secret.
    pub fn new(value: T) -> Self {
        Self(Zeroizing::new(value))
    }

    /// The secret itself. Call it only where the value leaves the process, such as
    /// an `Authorization` header.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl From<&str> for Secret<String> {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

/// Secrets arrive in JSON (token answers) and are wrapped as they are read. There is
/// no `Serialize`: writing one out is always a deliberate [`expose`](Secret::expose).
impl<'de, T: Zeroize + serde::Deserialize<'de>> serde::Deserialize<'de> for Secret<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Self::new)
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T: Zeroize> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn every_way_to_print_it_is_redacted() {
        let s = Secret::new(String::from("s3cr3t-value"));
        for shown in [
            format!("{s:?}"),
            format!("{s:#?}"),
            format!("{s}"),
            s.to_string(),
        ] {
            assert_eq!(shown, "<redacted>");
        }
        let nested = Some(vec![s.clone()]);
        assert!(!format!("{nested:?}").contains("s3cr3t"));
    }

    #[test]
    fn the_value_is_there_when_asked_for() {
        assert_eq!(Secret::from("v").expose(), "v");
    }
}
