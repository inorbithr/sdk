//! Credentials, profiles and the credential store for the `iohr` command line.
//!
//! A **profile** is a name for one way of calling the InOrbit API: which account the
//! calls count against and what kind of credential makes them. Profiles live in a TOML
//! file that never holds a secret ([`Config`]). Secrets live in a [`Store`], normally
//! the operating system's credential store ([`KeyringStore`]), one entry per profile
//! and account ([`EntryKey`]).
//!
//! Every secret is held in [`Redacted`], which never prints and is zeroed on drop
//! (SR-10, SR-11). Commands are written once against [`Credential`], which hands out a
//! [`Bearer`] for the next request.
//!
//! This crate is part of the `iohr` command line and is not published on its own; its
//! API may change in any release.

#![forbid(unsafe_code)]

mod config;
mod credential;
mod error;
mod session;

pub use config::{
    AskConfig, Config, DoctorConfig, ExtConfig, MemoryConfig, PanelConfig, ProjectConfig,
    RunnerConfig, SyncConfig,
};
pub use credential::{Bearer, Credential, StaticToken};
pub use error::AuthError;
/// A secret that never prints and is zeroed on drop: the SDK's [`inorbithr::Secret`].
pub use inorbithr::Secret as Redacted;
pub use inorbithr::login::ConfigError;
pub use inorbithr::login::{
    Authorization, Browser, Claims, ClaimsError, DEFAULT_CLIENT_ID, DEFAULT_ISSUER, Device,
    EntryKey, FileStore, Granted, KeyringStore, Kind, Login, LoginError, MemoryStore, Profile,
    ProfileName, ProfileNameError, Provider, QuietStore, SignOut, Storage, Store, StoreError,
    create_new_private, create_private_dir, off_thread, temp_path, write_private,
};
pub use session::Session;
