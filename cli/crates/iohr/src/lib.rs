//! The `iohr` command line.
//!
//! The binary in `main.rs` parses arguments and maps errors to exit codes; this library
//! holds everything else so it can be tested and fuzzed. It is not a public API and may
//! change in any release.

#![forbid(unsafe_code)]

pub mod api;
pub mod cli;

mod args;
mod browser;
mod commands;
mod context;
mod error;
mod lock;
mod output;

pub use args::refuse_secrets_in_args;
pub use commands::run;
pub use error::Error;

use iohr_auth::Redacted;

/// What the command line reads from its environment besides flags.
#[derive(Debug, Default)]
pub struct Env {
    /// `IOHR_TOKEN`: an API token used in memory, nothing written (SR-24).
    pub token: Option<Redacted<String>>,
}

impl Env {
    /// Reads the process environment.
    #[must_use]
    pub fn from_process() -> Self {
        let token = std::env::var("IOHR_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty());
        Self {
            token: token.map(|t| Redacted::new(t.trim().to_owned())),
        }
    }
}
