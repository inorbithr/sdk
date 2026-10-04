//! The platform's OpenAPI document, as `iohr sdk generate` reads it.
//!
//! Three steps, each a module: [`normalise`] applies the repository's rule N1 and
//! checks the facts the platform states itself (`spec/README.md`), so a generator
//! never sees a quirk and a regression upstream fails instead of being patched; [`hash`]
//! computes the cut hash the gateway stamps in `info.x-iohr-cut`, for a document that
//! has none; [`model`] turns one or several normalised documents into an [`Api`]: the
//! operations, which profiles may call each, and the schemas they use.
//!
//! This crate is part of the `iohr` command line and is not published on its own; its
//! API may change in any release.

#![forbid(unsafe_code)]

mod error;
pub mod hash;
pub mod model;
pub mod normalise;

pub use error::{ModelError, NormaliseError};
pub use hash::cut_hash;
pub use model::{Api, Cut, Media, Method, Operation, Param, ParamIn, Response};
pub use normalise::{Normalised, normalise, public_only, render_json};
