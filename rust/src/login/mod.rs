//! Person sign-in primitives (RFC 0231): the profile table `iohr login` writes, the
//! store a session's tokens live in, PKCE, the claims of an access token, and opening a
//! browser. Behind the `login` cargo feature, so a program that only calls the API with a
//! token pulls in none of the operating-system credential-store crates.
//!
//! The sign-in flows live here too: [`Provider`] (OpenID discovery), [`Login::browser`]
//! and [`Login::device`] (typestate [`Authorization`]), and the [`Session`] they end in,
//! saved with [`Session::save`] and ended with [`Session::sign_out`]. The command line
//! (`iohr`) uses this module, so a program signed in through `iohr` and one using the
//! SDK share one profile table and one store.

pub mod browser;
mod claims;
mod config;
mod error;
mod flow;
mod loopback;
mod page;
mod pkce;
mod provider;
mod session;
mod store;

pub use crate::auth::AUDIENCE;
pub use browser::open as open_browser;
pub use claims::{Claims, ClaimsError, audiences};
pub use config::{
    Config, ConfigError, Kind, Profile, ProfileName, ProfileNameError, Storage, create_new_private,
    create_private_dir, set_str, temp_path, write_private,
};
pub use error::LoginError;
pub use flow::{Authorization, Browser, Device, Granted, Login};
pub use loopback::{MAX_HEAD, READ_WINDOW, bind_loopback};
pub use pkce::{Pkce, RandomError, challenge, constant_time_eq, random_token};
pub use provider::{DEFAULT_CLIENT_ID, DEFAULT_ISSUER, Provider};
pub use session::{Session, SignOut};
pub use store::{
    EntryKey, FileStore, KeyringStore, MemoryStore, QuietStore, Store, StoreError, off_thread,
};
