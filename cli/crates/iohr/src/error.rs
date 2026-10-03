use std::fmt;
use std::process::ExitCode;

use iohr_auth::{AuthError, ClaimsError, ConfigError, StoreError};

use inorbithr::Code;

/// Why a command failed, and the exit code it maps to.
///
/// Exit codes are stable: 0 success, 1 a failed call, 2 a usage error, 3 not signed in,
/// 4 forbidden by scope, role or plan.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The command was used wrongly.
    Usage(String),
    /// There is no credential to call with.
    NotSignedIn(String),
    /// A call failed; `hint` says what to do about it.
    Api {
        /// The failure.
        error: inorbithr::Error,
        /// What to do, when the command knows better than the API's message.
        hint: Option<&'static str>,
    },
    /// `iohr sdk check` found the API's cut moved since the surface was generated.
    Drift(String),
    /// Anything else that failed.
    Failed(String),
    /// An extension ran and exited with this code; `iohr` exits with it too.
    Child(u8),
}

impl Error {
    /// The process exit code.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(match self {
            Self::Usage(_) => 2,
            // The runtime wraps the session's own refusal in Provider(text); a 401 is
            // the API refusing the token: both mean "sign in".
            Self::NotSignedIn(_)
            | Self::Api {
                error: inorbithr::Error::Auth(_),
                ..
            } => 3,
            Self::Api {
                error: inorbithr::Error::Api(e),
                ..
            } if e.code == Code::Unauthenticated || e.status == 401 => 3,
            Self::Api {
                error: inorbithr::Error::Api(e),
                ..
            } if e.code == Code::Forbidden || e.status == 403 => 4,
            Self::Api { .. } | Self::Failed(_) | Self::Drift(_) => 1,
            Self::Child(code) => *code,
        })
    }

    pub(crate) fn with_hint(error: inorbithr::Error, hint: &'static str) -> Self {
        Self::Api {
            error,
            hint: Some(hint),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(m) | Self::NotSignedIn(m) | Self::Failed(m) | Self::Drift(m) => {
                f.write_str(m)
            }
            Self::Child(_) => Ok(()),
            Self::Api { error, hint: None } => write!(f, "{error}"),
            Self::Api {
                error,
                hint: Some(h),
            } => write!(f, "{error}\n{h}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<inorbithr::Error> for Error {
    fn from(error: inorbithr::Error) -> Self {
        Self::Api { error, hint: None }
    }
}

impl From<AuthError> for Error {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::NotSignedIn { .. }
            | AuthError::SessionEnded { .. }
            | AuthError::Claims(_) => Self::NotSignedIn(e.to_string()),
            other => Self::Failed(other.to_string()),
        }
    }
}

impl From<ClaimsError> for Error {
    fn from(e: ClaimsError) -> Self {
        Self::NotSignedIn(e.to_string())
    }
}

impl From<StoreError> for Error {
    fn from(e: StoreError) -> Self {
        Self::Failed(e.to_string())
    }
}

impl From<crate::ext::manifest::ManifestError> for Error {
    fn from(e: crate::ext::manifest::ManifestError) -> Self {
        Self::Usage(e.0)
    }
}

impl From<crate::ext::lock::LockError> for Error {
    fn from(e: crate::ext::lock::LockError) -> Self {
        Self::Failed(e.0)
    }
}

impl From<ConfigError> for Error {
    fn from(e: ConfigError) -> Self {
        Self::Failed(e.to_string())
    }
}
