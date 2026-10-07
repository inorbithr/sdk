// swift-tools-version:5.9
// The Swift package of the InOrbit SDK. It sits at the repository root because SwiftPM
// resolves a package by its repository URL and reads `Package.swift` from the root; the
// code lives under `swift/` like every other language's (swift/AGENTS.md, ADR 0016).
import PackageDescription

let package = Package(
    name: "InOrbit",
    platforms: [
        .iOS(.v15),
        .macOS(.v12),
        .tvOS(.v15),
        .watchOS(.v8),
        .visionOS(.v1),
    ],
    products: [
        .library(name: "InOrbit", targets: ["InOrbit"]),
    ],
    targets: [
        .target(
            name: "InOrbit",
            path: "swift/Sources/InOrbit"
        ),
        .testTarget(
            name: "InOrbitTests",
            dependencies: ["InOrbit"],
            path: "swift/Tests/InOrbitTests"
        ),
    ]
)
