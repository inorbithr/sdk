//! Person sign-in primitives (RFC 0231): the profile table `iohr login` writes, the
//! store a session's tokens live in, PKCE, the claims of an access token, and opening a
//! browser. Behind the `login` cargo feature, so a program that only calls the API with a
//! token pulls in none of the operating-system credential-store crates.
//!
//! These are the pieces the command line's sign-in is built from; the flows that use them
//! come in a later slice. The command line (`iohr`) uses this module, so a program signed
//! in through `iohr` and one using the SDK share one profile table and one store.

pub mod browser;
mod claims;
mod config;
mod pkce;
mod store;

pub use crate::auth::AUDIENCE;
pub use claims::{Claims, ClaimsError, audiences};
pub use config::{
    Config, ConfigError, Kind, Profile, ProfileName, ProfileNameError, Storage, create_new_private,
    create_private_dir, set_str, temp_path, write_private,
};
pub use pkce::{Pkce, RandomError, challenge, constant_time_eq, random_token};
pub use store::{
    EntryKey, FileStore, KeyringStore, MemoryStore, QuietStore, Store, StoreError, off_thread,
};
