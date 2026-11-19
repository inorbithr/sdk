//! The command line's own credential types and config tables.
//!
//! The sign-in itself (the flows, the profile table, the credential store, `Secret`) is
//! the SDK's `inorbithr::login` (RFC 0231); this module holds only what is the command
//! line's: [`Config`] with the `[runner]`, `[ask]`, `[panel]` and other tables only
//! `iohr` reads, the [`Credential`] trait every command is written against, and
//! [`AuthError`] with the hints that name `iohr login`.

mod config;
mod credential;
mod error;
mod session;

pub use config::{
    AskConfig, Config, DoctorConfig, ExtConfig, MemoryConfig, PanelConfig, ProjectConfig,
    RunnerConfig, SyncConfig, ToolCeiling,
};
pub use credential::{Bearer, Credential, StaticToken};
pub use error::AuthError;
pub use session::Session;
