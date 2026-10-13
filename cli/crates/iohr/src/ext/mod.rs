//! Extensions: separate programs that `iohr` installs from an OCI registry, verifies,
//! pins in `iohr-ext.lock` and runs, handing them short-lived tokens over a private
//! channel and never its refresh token (ADR 0012, platform RFC 0028).

pub(crate) mod dev;
pub mod init;
pub mod install;
pub mod layer;
pub mod lint;
pub mod lock;
pub mod manifest;
pub mod oci;
pub(crate) mod run;
pub mod socket;
pub mod trust;
