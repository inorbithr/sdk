//! Reads the service's port setting.

/// Why the setting did not load.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The variable is not set.
    #[error("{0} is not set")]
    Missing(&'static str),
    /// The value is not a port.
    #[error("the port is not a number: {0}")]
    BadPort(#[from] std::num::ParseIntError),
}

/// The port in `value`, the content of the PORT variable.
///
/// # Errors
///
/// Returns [`ConfigError`] when it is missing or not a port.
pub fn port(value: Option<&str>) -> Result<u16, ConfigError> {
    Ok(value.ok_or(ConfigError::Missing("PORT"))?.parse()?)
}
