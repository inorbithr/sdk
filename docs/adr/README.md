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
| [0015](0015-configuration-and-middleware.md) | Configuration, credentials and the middleware pipeline | accepted |
| [0016](0016-swift.md) | Swift: the package at the repository root, a runtime ahead of its parity | proposed |
| [0017](0017-c.md) | C: a pull transport the caller can replace, our own JSON reader, one arena per answer | proposed |
| [0018](0018-ci-changed-gate-record.md) | A passing `ci:changed` leaves the platform's gate record | proposed |
| [0019](0019-pr-preflight.md) | A pull request preflight: every failing check named in one place, advisory | proposed |
| [0020](0020-built-in-roles-inherit-and-check-before-import.md) | Built-in roles: one root they inherit, one text limit, a check before import | proposed |
| [0021](0021-roles-gates-base-pack-and-a-stack.md) | Built-in roles: gates in the role, the base as a pack, a project's whole stack | proposed |
| [0022](0022-role-evals.md) | Role evals: planted-defect cases, three arms, a landing rule | proposed |
| [0023](0023-atlas-scrum-in-iohr.md) | The scrum board moves into iohr: `iohr atlas scrum` | proposed |
