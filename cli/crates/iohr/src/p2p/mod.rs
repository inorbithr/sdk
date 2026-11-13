//! Peer-to-peer reports (PRD 0219, RFC 0230, ADR 0242): signed scam reports about a domain
//! or a handle, shared between people's machines without a central store.
//!
//! Slice 1 (this code): the record and its signature (`record`), the flood and smear guard
//! (`guard`), and the background agent `iohr p2p register` starts (`agent`). The transport,
//! the sign-in vouch and the peer exchange are slice 2.

pub(crate) mod agent;
// The record and the guard are proven by their tests now; `publish`, `subscribe` and the
// node use them in slice 2.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the peer exchange in slice 2 (RFC 0230)")
)]
pub(crate) mod guard;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the peer exchange in slice 2 (RFC 0230)")
)]
pub(crate) mod record;

#[cfg(test)]
mod tests;
