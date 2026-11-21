//! Parses the `host:port` addresses users type into the connect box.

use std::fmt;

/// A host and a port.
#[derive(Debug, PartialEq, Eq)]
pub struct Addr {
    pub host: String,
    pub port: u16,
}

/// Why an address did not parse.
#[derive(Debug, PartialEq, Eq)]
pub enum AddrError {
    /// No `:` between the host and the port.
    MissingPort,
    /// The host is empty.
    EmptyHost,
    /// The port is not a number from 0 to 65535.
    BadPort(String),
}

impl fmt::Display for AddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPort => f.write_str("expected host:port"),
            Self::EmptyHost => f.write_str("the host is empty"),
            Self::BadPort(p) => write!(f, "{p:?} is not a port"),
        }
    }
}

impl std::error::Error for AddrError {}

/// Parses `host:port`, as typed by a user.
///
/// # Errors
///
/// Returns an [`AddrError`] when the input is not `host:port`.
pub fn parse(input: &str) -> Result<Addr, AddrError> {
    let (host, port) = input.trim().rsplit_once(':').ok_or(AddrError::MissingPort)?;
    if host.is_empty() {
        return Err(AddrError::EmptyHost);
    }
    let port = port.parse().map_err(|_| AddrError::BadPort(port.to_owned()))?;
    Ok(Addr { host: host.to_owned(), port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_port() {
        assert_eq!(parse("db.local:5432"), Ok(Addr { host: "db.local".into(), port: 5432 }));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert_eq!(parse("db.local"), Err(AddrError::MissingPort));
        assert_eq!(parse("db.local:99999"), Err(AddrError::BadPort("99999".into())));
    }
}
