// swift-tools-version:5.9
// The Swift examples, built against the package at the repository root.
import PackageDescription

let package = Package(
    name: "Examples",
    platforms: [.macOS(.v12)],
    dependencies: [.package(name: "InOrbit", path: "../..")],
    targets: [
        .executableTarget(name: "Whoami", dependencies: ["InOrbit"]),
        .executableTarget(name: "ListMonitors", dependencies: ["InOrbit"]),
    ]
)
