# Peer to peer

`iohr p2p` puts this machine in touch with other people's machines through a bootstrap node,
so they can share signed scam reports: one domain or one handle and a reason from a fixed
list, nothing else. The first user is the eye app. The decision is
[ADR 0018](../../docs/adr/0018-p2p-transport.md); the security requirement is
[SR-37](../../docs/security/requirements.md). Back to the [iohr overview](../README.md).

Status, 2026-11-13: slice 1 of 4. `register`, `status` and `unregister` work, and the
report record and its flood and smear guard are built and tested. `peers`, `publish`,
`subscribe` and `retract` exit 1 with "not implemented yet" until the peer exchange
(slice 2) lands.

```sh
iohr p2p register --bootstrap https://node.example:7443   # starts the agent and returns
iohr p2p status                                           # is it running, when it last reached the node
iohr p2p unregister                                       # stops it and removes its folder
```

## The commands

| Command | What it does |
|---|---|
| `iohr p2p register [--bootstrap URL]` | Starts the background agent and returns at once. Running it again while the agent runs changes nothing. |
| `iohr p2p status` | Whether the agent runs, its bootstrap node, its process id, when it last reached the node and how many tries it made. "not registered" when there is no agent. |
| `iohr p2p unregister` | Asks the agent to stop, waits up to 5 s, then removes `p2p/` from the config directory. |
| `iohr p2p peers` | The peers this machine knows. Not implemented yet. |
| `iohr p2p publish` | Signs and shares a report. Not implemented yet. |
| `iohr p2p subscribe` | Follows the reports and live warnings peers share. Not implemented yet. |
| `iohr p2p retract` | Takes back a report you signed. Not implemented yet. |

`--json` works on each. None of them needs a sign-in in slice 1, and none calls the API.

## The bootstrap node

`--bootstrap URL` or `IOHR_P2P_BOOTSTRAP` names it; there is no default. The address is
`https://HOST[:PORT]`, plain `http://` only on loopback (for tests), with no user, path,
query or fragment. Anything else exits 2. This is the one host `iohr` talks to besides the
API ([Files, network and dependencies](files-and-dependencies.md)); the exception ends
when the node moves behind the API edge (slice 4).

## The agent

`register` starts the agent detached, in its own process group, with no terminal and no
inherited output. Every second it checks for a stop request; it opens a TCP connection to
the node, waits longer after each failure, and writes its state. It writes no log. Its
only file is `p2p/state.json` in the config directory, mode 0600: the bootstrap address,
its process id, times and counts, never a report or a subject. `status` calls an agent
that has not written its state for 90 s "not running".

## Reports and the guard

A report is one domain (ASCII, an international name in its `xn--` form) or one handle
(`<network>/@<name>` on telegram, instagram, x, tiktok, facebook or discord) and one of
six reasons: `fake_shop`, `phishing`, `impersonation`, `investment_fraud`, `fake_support`,
`malware`. It is at most 1024 bytes and is signed with a key held on the device, which the
person's InOrbit sign-in vouches for. A token is never the signature.

A node checks each report in this order: shape, time, signature, and only then anything
that changes state, so a forged report costs the key it names nothing. Then:

| Limit | Value |
|---|---|
| Reports per key | 10 an hour |
| Reports per reporter, across all their keys | 30 a day |
| Clock difference for a live report | 600 s |
| Distinct reporters before a subject is listed | 3 within 7 days |
| How long a report is kept | 30 days |
| Reports per node | 100,000 |

A replayed report or nonce is refused. Only the key that signed a report can retract it,
and a retraction gives no quota back. Errors and refusals never repeat the subject.
