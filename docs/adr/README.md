# Decisions

One file per decision, numbered, never renumbered. A decision that changes gets a new
record that supersedes the old one; the old one stays with its status updated.

Format: Status, Context, Decision, Consequences, Sources. Keep each under a page.

| # | Decision | Status |
|---|---|---|
| [0001](0001-one-repository.md) | One repository for all languages | accepted |
| [0002](0002-generated-types-handwritten-client.md) | Generated types, handwritten client | accepted |
| [0003](0003-versioning-and-releases.md) | Independent versions, release-please, trusted publishing | accepted |
| [0004](0004-transport-scope.md) | REST first; SSE, socket and MCP when the platform opens them | accepted |
| [0005](0005-ci-and-supply-chain.md) | CI shape and supply-chain rules | accepted |
| [0006](0006-names-and-runtimes.md) | Package names and minimum runtimes | proposed |
| [0007](0007-security-baseline.md) | Security baseline before 1.0 | accepted |
| [0008](0008-key-scopes-and-iohr-identifiers.md) | Per-key scopes and the iohr identifiers | accepted |
| [0009](0009-the-command-line.md) | The command line lives here, in its own workspace | accepted |
| [0010](0010-releasing-the-command-line.md) | Releasing the command line: our own workflow, six targets, WiX 5 | accepted |
| [0011](0011-runtime-and-surface.md) | Runtime and surface: generated operations on a hand-written runtime | accepted |
| [0012](0012-extensions.md) | Extensions: signed OCI artifacts, run as separate processes | accepted |
| [0013](0013-java-and-csharp.md) | Java and C#: names, runtimes, the surface shape | accepted |
| [0014](0014-what-ci-runs-where.md) | What CI runs where: Linux on merges, everything nightly and on releases | accepted |
