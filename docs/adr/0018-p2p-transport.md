# 0018. `iohr p2p`: signed scam reports between people, a named bootstrap host

Status: postponed, 2026-11-13. The platform's owner chose an InOrbit reports API, reached
over WSS for live warnings, the same day (the eye app's ADR 0006). Every `iohr p2p` verb
exits 1 and says so; the report record and its guard stay in `iohr`, because the reports
API reuses them. The text below is the design as it stood when it was postponed.

The platform documents behind this ADR are PRD 0219 (what is asked), RFC 0230 (the
design) and ADR 0242 (the transport), in the platform's Decisions space. This ADR records
what they change in this repository.

## Context

On 2026-11-13 the platform's owner asked for peer-to-peer management in `iohr`:
`iohr p2p register` starts a background agent, and `status`, `peers`, `publish`,
`subscribe` and `unregister` sit beside it. The owner's MacBook is the bootstrap node for
now. The first use is the eye app: people share signed scam reports, a domain or a handle
and nothing else, and live warnings when a subject is reported by enough people.

Three constraints shape it:

- A report must be signed by its author through their InOrbit sign-in, and eye never
  stores the token. A token is a bearer credential, so it cannot be the signature: the
  report is signed with a key held on the device, and the sign-in vouches for that key.
- Peers are strangers. The design has to stop one person flooding the network and one
  person (or a few keys of one person) smearing a shop.
- This repository's rule is that the SDK talks only to the public API and the token
  endpoint (`AGENTS.md`).

What exists, 2026-11-13: the approvals device keys (`apk_` key ids, ECDSA P-256 over
SHA-256, `runner::approvals::DeviceKeys` verifies them, `mac_sign::DeviceSigner` signs
with the Secure Enclave key on a Mac), the runner's software P-256 key
(`runner::signing::RunKey`), and the platform's Noise relay of RFC 0193 (the `snow` crate,
in the Mac app). No libp2p anywhere.

## Decision

1. **The record.** `p2p/report/v0` (RFC 0230 section 2) is one domain or one handle
   (`<network>/@<name>`), a reason from a fixed list of six, the time, a 16-byte nonce and
   the device key id, signed over one field per line under the tag `iohr.p2p.v1.Report`,
   the layout the approval decisions already use. No free text travels. A record is at
   most 1024 bytes and refused with any unknown field. Its schema folds into the
   platform's schemas effort (RFC 0222), not a parallel one.
2. **No new crypto.** Signing goes through `mac_sign::DeviceSigner`; `RunKey` implements it
   for machines without a Secure Enclave. Verification is `DeviceKeys::verifies`. The
   sign-in's vouch for a key (RFC 0230 section 3) is the platform's, through the
   approvals device-key registration.
3. **Flood and smear guard** (`p2p::guard`, RFC 0230 section 5). Checks run in a fixed
   order: shape, time, signature, and only then anything that changes state, so a forged
   report spends nothing of the key it names. Limits: 10 reports per key per hour, 30 per
   reporter per day across all their keys, a live report within 600 s of the node's
   clock, stored reports kept 30 days, at most 100,000 reports per node. A subject is
   listed only when 3 distinct reporters (the pseudonym in the vouch, not the key)
   reported it within 7 days. Retraction is the signing key's own and gives no quota
   back.
4. **Transport** (ADR 0242). Version 1 reaches peers through a bootstrap node with Noise
   IK (`snow`) inside TLS, the RFC 0193 pattern; libp2p is deferred until a central node
   is the bottleneck, and then gets its own ADR. The owner chooses through one decision
   ask; slice 1 needs no transport.
5. **The host rule's one exception.** `iohr p2p` may reach the bootstrap hosts the person
   names (`--bootstrap` or `IOHR_P2P_BOOTSTRAP`), `https://` only, plain `http://` on
   loopback for tests, with no user, path, query or fragment, and no other host. The
   exception ends when the node moves behind the API edge (RFC 0230 slice 4).
6. **The agent.** `iohr p2p register` starts `iohr p2p agent` (hidden) detached, in its own
   process group with no inherited output, and returns. The agent writes no log; its only
   output is `p2p/state.json` in the config directory, mode 0600, holding the bootstrap
   address, times and counts, never a report or a subject. `unregister` asks it to stop
   through a stop file, waits up to 5 s for it to say so, then removes the folder.

### Slices

| Slice | What | Here |
|---|---|---|
| 1 | The record, the guard, `register`, `status`, `unregister` | this ADR's pull request |
| 2 | The Noise exchange with the bootstrap node; `peers`, `publish`, `subscribe`, `retract` | after the owner's transport choice and a Security review |
| 3 | eye's client and live warnings | the eye repository |
| 4 | The node behind the API edge, the host exception removed | the platform |

Until slice 2, `peers`, `publish`, `subscribe` and `retract` exit 1 with "not implemented
yet".

## Consequences

- No new runtime dependency in slice 1; `proptest` joins the test dependencies. Slice 2
  adds `snow`, with its reason in `cli/docs/files-and-dependencies.md`.
- The bootstrap node is a single point of failure until slice 4; the platform watches it
  with a monitor (RFC 0230 section 6).
- A report is about someone, so errors and the state file never repeat a subject; tests
  hold every error to that.
- The threat model gains a `iohr p2p` section, and the requirements gain SR-37.
