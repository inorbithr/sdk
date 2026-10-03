//! The `iohr` command line.
//!
//! The binary in `main.rs` parses arguments and maps errors to exit codes; this library
//! holds everything else so it can be tested and fuzzed. It is not a public API and may
//! change in any release.

#![forbid(unsafe_code)]

pub mod cli;
pub mod ext;

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
#[derive(Debug, Default, Clone)]
pub struct Env {
    /// `IOHR_TOKEN`: an API token used in memory, nothing written (SR-24).
    pub token: Option<Redacted<String>>,
    /// `IOHR_TOKEN_<PROFILE>`: a token standing in for a named profile, for CI, where
    /// `iohr sdk check` runs and a person cannot sign in. Read on demand.
    profile_tokens: std::collections::BTreeMap<String, Redacted<String>>,
    /// `IOHR_EXT_REGISTRY`: the registry for extensions, over `ext.registry`.
    pub ext_registry: Option<String>,
    /// `IOHR_EXT_REGISTRY_AUTH`: `user:password` for a private registry mirror.
    pub ext_registry_auth: Option<Redacted<String>>,
}

impl Env {
    /// Reads the process environment.
    #[must_use]
    pub fn from_process() -> Self {
        let read = |v: String| Redacted::new(v.trim().to_owned());
        let token = std::env::var("IOHR_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty())
            .map(read);
        let profile_tokens = std::env::vars()
            .filter_map(|(k, v)| {
                let name = k.strip_prefix("IOHR_TOKEN_")?;
                (!name.is_empty() && !v.trim().is_empty()).then(|| (name.to_owned(), read(v)))
            })
            .collect();
        let ext_registry = std::env::var("IOHR_EXT_REGISTRY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let ext_registry_auth = std::env::var("IOHR_EXT_REGISTRY_AUTH")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(read);
        Self {
            token,
            profile_tokens,
            ext_registry,
            ext_registry_auth,
        }
    }

    /// The token `IOHR_TOKEN_<PROFILE>` holds for `profile`, if any.
    #[must_use]
    pub fn profile_token(&self, profile: &str) -> Option<&Redacted<String>> {
        self.profile_tokens.get(&env_name(profile))
    }
}

/// `acme-ci` reads `IOHR_TOKEN_ACME_CI`: the name in upper case, `-` as `_`.
#[must_use]
pub fn env_name(profile: &str) -> String {
    profile
        .chars()
        .map(|c| match c {
            '-' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}
