// swift-tools-version: 6.0
import PackageDescription

// No dependencies: Foundation only. The library holds the program so the tests can call it
// with their own streams; the executable only passes the real ones.
let package = Package(
    name: "@@NAME@@",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "@@NAME@@", targets: ["App"])],
    targets: [
        .target(name: "Cli", path: "Sources/Cli"),
        .executableTarget(name: "App", dependencies: ["Cli"], path: "Sources/App"),
        .testTarget(name: "CliTests", dependencies: ["Cli"], path: "Tests/CliTests"),
    ]
)
