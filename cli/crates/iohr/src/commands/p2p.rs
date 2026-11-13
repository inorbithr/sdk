//! `iohr p2p`: postponed (owner, 2026-11-13). Every verb exits 1 with one line that says so;
//! reports go through the InOrbit reports API instead (eye ADR 0006, RFC 0230 status log).
//! The record and the guard in `crate::p2p` stay, because the reports API reuses them.

use crate::error::Error;

/// The one line every `iohr p2p` verb prints.
pub(crate) const POSTPONED: &str =
    "P2P is postponed; reports go through the InOrbit reports API (eye ADR 0006)";

pub(crate) fn run() -> Result<(), Error> {
    Err(Error::Failed(POSTPONED.to_owned()))
}
