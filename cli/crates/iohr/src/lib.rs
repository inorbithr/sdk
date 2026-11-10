//! The `iohr` command line.
//!
//! The binary in `main.rs` parses arguments and maps errors to exit codes; this library
//! holds everything else so it can be tested and fuzzed. It is not a public API and may
//! change in any release.

#![forbid(unsafe_code)]

pub mod atlas;
pub mod cli;
pub mod environments;
pub mod ext;
pub mod front;
pub mod kb;
pub mod memory;
pub mod project;
pub mod roles;
pub mod template;

mod access;
mod actions;
mod args;
mod ask;
mod askback;
mod attach;
mod browser;
mod card;
mod commands;
mod context;
mod error;
mod examples;
mod handoff;
mod headless;
mod health;
mod hints;
mod io_signin;
mod iomd;
mod lock;
mod mac_sign;
mod output;
mod panel;
mod paths;
mod permissions;
mod planner;
mod prompt;
mod runner;
#[cfg(unix)]
mod session;
mod steps;
mod toolchain;
mod ui;
mod usage;

pub use args::refuse_secrets_in_args;
pub use commands::run;

/// Whether bare `iohr` enters the session here (a terminal, colour allowed, no `--json`, not
/// inside a panel) instead of printing the usage report.
#[must_use]
pub fn bare_enters(g: &cli::Global) -> bool {
    panel::bare_enters(g)
}

/// Bare `iohr` inside a running panel: says in one line that the session is already here and
/// which key opens the menu, and returns true (the caller exits 0). False anywhere else.
#[must_use]
pub fn bare_inside(g: &cli::Global) -> bool {
    panel::bare_inside_message(g).is_some_and(|line| {
        output::Out::note(&line);
        true
    })
}
pub use error::Error;
pub use usage::{report_clap as report_usage_clap, report_message as report_usage_message};

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
    /// `IOHR_SIGN_IDENTITY`: how an upgrade signs the new binary on macOS, over
    /// `upgrade_sign_identity` (`auto`, `-` or an identity).
    pub sign_identity: Option<String>,
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
        let sign_identity = std::env::var("IOHR_SIGN_IDENTITY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        Self {
            token,
            profile_tokens,
            ext_registry,
            ext_registry_auth,
            sign_identity,
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
