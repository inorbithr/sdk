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

mod claims;
mod config;
mod credential;
mod error;
mod flow;
pub mod loopback;
mod oidc;
mod page;
mod pkce;
mod secret;
mod session;
mod store;

pub use claims::{Claims, ClaimsError};
pub use config::{
    Config, ConfigError, ExtConfig, Kind, Profile, ProfileName, ProfileNameError, Storage,
};
pub use credential::{Bearer, Credential, StaticToken};
pub use error::AuthError;
pub use flow::{Authorization, Browser, Device, Granted};
pub use oidc::{DEFAULT_CLIENT_ID, DEFAULT_ISSUER, Provider};
pub use secret::Redacted;
pub use session::Session;
pub use store::{EntryKey, FileStore, KeyringStore, MemoryStore, Store, StoreError};
