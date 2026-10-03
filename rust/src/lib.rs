//! The InOrbit API client for Rust.
//!
//! This crate is the **runtime**: the client, its credentials, retries, errors and
//! hooks. The operations you call sit on top of it as a **surface**: the public one
//! this crate ships with, or one `iohr sdk generate` writes for your own credentials,
//! with exactly the operations they may call (platform RFC 0020, ADR 0011).
//!
//! ```no_run
//! use inorbithr::{Client, Method, Operation};
//!
//! # async fn run() -> Result<(), inorbithr::Error> {
//! // INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES, or INORBIT_TOKEN.
//! let client: Client = Client::from_env()?;
//! let me: inorbithr::Response<serde_json::Value> =
//!     client.request(Operation::new(Method::Get, "/v1/me")).await?;
//! println!("{} ({} attempt)", me.value["subject"], me.raw.attempts);
//! # Ok(())
//! # }
//! ```
//!
//! What the client does for you, in every language the SDK ships in
//! (`docs/design.md`):
//!
//! - exchanges an API key for a short-lived token, caches it and refreshes it, or uses
//!   an API token as it is;
//! - retries what is safe to retry (`429`, `503`, `504`, connection failures) with
//!   `Retry-After` honoured, on idempotent calls only;
//! - answers with one error family, [`Error`], carrying the API's error `code` and
//!   `details`;
//! - never prints a secret: [`Secret`] redacts and zeroes on drop.

#![forbid(unsafe_code)]

mod auth;
mod client;
pub mod error;
mod hooks;
mod int64;
mod profile;
mod retry;
mod secret;

pub use auth::{AUDIENCE, ClientCredentials, DEFAULT_TOKEN_URL, StaticToken, Token, TokenProvider};
pub use client::{Client, ClientBuilder, DEFAULT_BASE_URL, Method, Operation, Response};
pub use error::{ApiError, AuthError, Code, ConfigError, Detail, Error, Headers, RawResponse};
pub use hooks::{Attempt, Hook};
pub use int64::Int64;
pub use profile::{Profile, Public};
pub use secret::Secret;

/// What a generated surface imports: the client, the profile types, the operation
/// builder and the error.
pub mod prelude {
    pub use crate::client::{Client, Method, Operation, Response};
    pub use crate::error::Error;
    pub use crate::int64::Int64;
    pub use crate::profile::{Profile, Public};
}

/// The contract between this runtime and the code `iohr sdk generate` writes. A
/// generated surface asserts [`VERSION`](__codegen::VERSION) at compile time, so a
/// runtime that no longer matches fails the build with a clear message instead of a
/// confusing one.
#[doc(hidden)]
pub mod __codegen {
    /// Bumped when a generated surface written for an older runtime would not compile
    /// or would behave differently.
    pub const VERSION: u32 = 1;
}
