use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 64-bit integer as the API sends it: a decimal string on the wire, a number in
/// Rust (`docs/design.md` section 4).
///
/// It reads both forms, so a field the API later sends as a JSON number keeps working.
///
/// # Examples
///
/// ```
/// use inorbithr::Int64;
///
/// let n: Int64 = serde_json::from_str("\"42\"").unwrap();
/// assert_eq!(n.0, 42);
/// assert_eq!(serde_json::to_string(&Int64(-7)).unwrap(), "\"-7\"");
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Int64(pub i64);

impl From<i64> for Int64 {
    fn from(n: i64) -> Self {
        Self(n)
    }
}

impl From<Int64> for i64 {
    fn from(n: Int64) -> Self {
        n.0
    }
}

impl fmt::Display for Int64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Int64 {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

impl Serialize for Int64 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Int64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Int64;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a 64-bit integer, as a decimal string or a number")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Int64, E> {
                v.parse().map(Int64).map_err(E::custom)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Int64, E> {
                Ok(Int64(v))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Int64, E> {
                i64::try_from(v).map(Int64).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::Int64;

    #[test]
    fn reads_strings_and_numbers_and_writes_strings() {
        let a: Int64 = serde_json::from_str("\"9007199254740993\"").unwrap();
        let b: Int64 = serde_json::from_str("12").unwrap();
        assert_eq!(a.0, 9_007_199_254_740_993);
        assert_eq!(b.0, 12);
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"9007199254740993\"");
        assert!(serde_json::from_str::<Int64>("\"x\"").is_err());
        assert_eq!("5".parse::<Int64>().unwrap(), Int64(5));
    }
}
