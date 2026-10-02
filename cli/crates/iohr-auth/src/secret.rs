use std::fmt;

use zeroize::{Zeroize, Zeroizing};

/// A secret value: a token, a refresh token or a key secret.
///
/// `Debug` and `Display` print `<redacted>`, the value is not `Serialize`, and its
/// memory is zeroed when it is dropped (SR-10, SR-11). Read it with
/// [`expose`](Self::expose) at the one place it is sent.
///
/// # Examples
///
/// ```
/// use iohr_auth::Redacted;
///
/// let token = Redacted::new(String::from("eyJhbGciOi..."));
/// assert_eq!(format!("{token:?}"), "<redacted>");
/// assert_eq!(token.expose(), "eyJhbGciOi...");
/// ```
#[derive(Clone)]
pub struct Redacted<T: Zeroize>(Zeroizing<T>);

impl<T: Zeroize> Redacted<T> {
    /// Wraps a secret.
    pub fn new(value: T) -> Self {
        Self(Zeroizing::new(value))
    }

    /// The secret itself. Call it only where the value leaves the process, such as
    /// an `Authorization` header or the credential store.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

/// Secrets arrive in JSON (token answers, the stored session) and are wrapped as they
/// are read. There is no `Serialize`: writing one out is always a deliberate
/// [`expose`](Redacted::expose).
impl<'de, T: Zeroize + serde::Deserialize<'de>> serde::Deserialize<'de> for Redacted<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Self::new)
    }
}

impl<T: Zeroize> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T: Zeroize> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T: Zeroize + PartialEq> PartialEq for Redacted<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}

impl<T: Zeroize + Eq> Eq for Redacted<T> {}

#[cfg(test)]
mod tests {
    use super::Redacted;

    #[test]
    fn every_way_to_print_it_is_redacted() {
        let s = Redacted::new(String::from("s3cr3t-value"));
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
        assert_eq!(Redacted::new(String::from("v")).expose(), "v");
    }
}
