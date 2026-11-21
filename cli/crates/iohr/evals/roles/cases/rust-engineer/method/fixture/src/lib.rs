//! Reads the service's port setting.

use std::fmt;

/// Why the setting did not load.
#[derive(Debug)]
pub enum ConfigError {
    /// The variable is not set.
    Missing(&'static str),
    /// The value is not a port.
    BadPort(std::num::ParseIntError),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "{name} is not set"),
            Self::BadPort(e) => write!(f, "the port is not a number: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// The port in `value`, the content of the PORT variable.
///
/// # Errors
///
/// Returns [`ConfigError`] when it is missing or not a port.
pub fn port(value: Option<&str>) -> Result<u16, ConfigError> {
    value.ok_or(ConfigError::Missing("PORT"))?.parse().map_err(ConfigError::BadPort)
}
