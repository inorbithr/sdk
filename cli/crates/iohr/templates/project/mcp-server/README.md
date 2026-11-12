# @@NAME@@

@@DESCRIPTION@@

An MCP server in @@LANGUAGE_LABEL@@, built on the official MCP SDK (not on a wrapper of ours), at
a version that implements the 2026-07-28 revision of the protocol. It speaks over stdio, so a
host starts it as a child process: there is no port, no listener and no credential.

It ships one tool and one resource so the test and the rules have something to hold:

| Name | Kind | What it does |
| --- | --- | --- |
| `word_count` | tool | Counts the words and characters of a text (at most 10,000 characters). Read-only; returns structured content. |
| `about://server` | resource | One sentence on what the server is. |

## What is here

@@LAYOUT@@
- `.env.example`: the variables the server reads, names only (none yet).
- `IO.md`, `AGENTS.md`: how to work in this folder, for people and agents.
- `.github/`: CI (test, secret scan, SBOM), the signed and attested release, dependency updates, CODEOWNERS.
- `SECURITY.md`: how to report a vulnerability.
- `inorbit.toml`, `.inorbit/space`: the InOrbit project record and the link to its space.

## Run

```sh
@@RUN@@
```

Nothing prints: the process waits for a host to speak MCP on its stdin and answers on its
stdout. That is why the server never writes anything else to stdout (the SDK logs to stderr).
To use it, add it to a host's MCP configuration as a command, for example in a project's
`.mcp.json`:

```json
{ "mcpServers": { "@@NAME@@": { "command": "sh", "args": ["-c", "cd /path/to/@@NAME@@ && @@RUN@@"] } } }
```

## The protocol revision

The 2026-07-28 revision removed the `initialize` handshake: a client calls `server/discover` (or
simply calls), results carry a `resultType`, and lists say how long to keep them (`ttlMs`,
`cacheScope`). The SDK also still answers `initialize`, so a host that has not moved yet works
too. The test checks both. The SDK is pinned to an exact version; Dependabot proposes each
update, and the test says whether the new one still passes.

## Tools you add

A host lets a model call a tool, so each one needs three things, and the test fails when one is
missing: a description (the model reads it), an annotation saying whether it only reads
(`readOnlyHint`) or can destroy (`destructiveHint`), and a bound on every input (a maximum
length, an enum, a range). A tool that writes, sends or deletes needs its own review: say so in
`SECURITY.md`, and never log an argument or a result, because those can be customer data.

## Test

```sh
@@README_TEST@@
```

The test starts the server as a child process over stdio and speaks to it with the SDK's own
client: a connection on 2026-07-28, a connection through the older handshake, `tools/list` (the
tool, the same order twice, the cache fields), `tools/call` and its structured result, a refused
bad argument, the resource, and the rules above.

## Release

Push a tag `v*`. `.github/workflows/release.yml` builds, writes a CycloneDX SBOM, signs every
file with cosign (keyless) and attaches a build-provenance attestation. Its steps and how to
verify a download are in the workflow's header.

## Licence

@@LICENSE@@
