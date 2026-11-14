//! Peer-to-peer reports (PRD 0219, RFC 0230, ADR 0242): signed scam reports about a domain
//! or a handle, shared between people's machines without a central store.
//!
//! Postponed (owner, 2026-11-13): `iohr p2p` exits 1 and reports go through the InOrbit
//! reports API (eye ADR 0006). Kept and tested here: the record and its signature (`record`),
//! the flood and smear guard (`guard`), which the reports API reuses, and the background
//! agent (`agent`) for when P2P comes back as a second transport.

// Postponed: the spawn path is dead outside tests and, in tests, too (nothing registers).
#[expect(
    dead_code,
    reason = "P2P is postponed (RFC 0230 status log, 2026-11-13)"
)]
pub(crate) mod agent;
// The record and the guard are proven by their tests now; the reports API reuses them.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "reused by the reports API (RFC 0230 status log, 2026-11-13)"
    )
)]
pub(crate) mod guard;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "reused by the reports API (RFC 0230 status log, 2026-11-13)"
    )
)]
pub(crate) mod record;

#[cfg(test)]
mod tests;
