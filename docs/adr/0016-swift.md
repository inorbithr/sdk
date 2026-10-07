# 0016. Swift: the package at the repository root, a runtime ahead of its parity

Status: proposed, 2026-10-07

## Context

The platform's owner asked for a Swift package (2026-10-06) so customers build InOrbit
into their iOS, macOS and server-side Swift apps, the same way the other six languages
do: a hand-written runtime and the surface `iohr sdk generate` writes onto it (ADR 0011).

SwiftPM resolves a dependency by repository URL and reads `Package.swift` from the root
of the repository at a version tag. It has no notion of a package in a subdirectory, and
a version tag must be a semantic version, optionally prefixed with `v`
(`swift/v0.1.0`, release-please's form in this repository, is not one). Two ways fit a
monorepo: a `Package.swift` at the root that points into `swift/`, tagged `vX.Y.Z`; or a
mirror repository (`inorbithr/sdk-swift`) that a release job pushes to.

## Decision

| Language | Distribution | Import | Minimum | CI |
|---|---|---|---|---|
| Swift | SwiftPM, `https://github.com/inorbithr/sdk`, product `InOrbit` | `import InOrbit` | Swift 5.9 tools; iOS 15, macOS 12, tvOS 15, watchOS 8, visionOS 1, Linux | Xcode on `macos-latest`, the `swift:6.1` image on Linux |

- **The package is at the root.** `Package.swift` names `swift/Sources/InOrbit` and
  `swift/Tests/InOrbitTests`; everything else of the language lives under `swift/`.
  release-please versions the `swift` component and tags it `vX.Y.Z` (no component in
  the tag), the one bare tag scheme in this repository. Nothing is published anywhere
  else: SwiftPM fetches the tag. A mirror repository would keep the root clean but adds
  a second repository, a push token and a sync that can drift; it is the fallback if a
  bare tag ever collides with another component.
- **The runtime** uses Foundation and `URLSession` (FoundationNetworking on Linux),
  `async`/`await` and `Codable`, and no other dependency. It builds clean under complete
  strict concurrency checking.
- **The surface** follows the Rust and C# shape: a marker protocol per operation adopted
  by the profile types whose cut holds it, and the operations in constrained extensions
  (`extension RadarHandle where P: AllowsListDigests`), so a call a profile may not make
  does not compile. Models spell out their `Codable`: wire names as keys, a 64-bit
  integer through `WireInt64`, unset optionals left out of a request.
- **The runtime ships ahead of full parity.** The first runtime passes the cases for
  authentication, errors, operations, retries and server-sent events. The cases that
  need the M6 configuration, the middleware pipeline, the TLS and proxy options, and the
  `/v1/ws` socket are `pending: [swift]` against one parity issue (#172), as AGENTS.md
  allows for a runtime that arrives later.

## Consequences

- `swift` joins `pending` in `conformance/case.schema.json`, and the parity issue lists
  the cases it holds.
- The root holds `Package.swift` and SwiftPM's `.build/`, which `.gitignore` drops.
- The first release-please PR for `swift` proposes the first version; until then the
  user agent says `inorbithr-sdk-swift/0.0.0`.

## Sources

- Swift Package Manager: https://www.swift.org/documentation/package-manager/
- SwiftPM version tags (semantic versions, an optional `v`): https://github.com/swiftlang/swift-package-manager/blob/main/Documentation/Usage.md
- ADR 0003, ADR 0011, ADR 0013
