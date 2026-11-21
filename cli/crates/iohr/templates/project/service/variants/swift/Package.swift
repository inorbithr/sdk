// swift-tools-version: 6.0
import PackageDescription

// No dependencies: Foundation and the C socket API only. The library holds the service so the
// tests can start it on a free port; the executable only reads the environment and signals.
let package = Package(
    name: "@@NAME@@",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "@@NAME@@", targets: ["App"])],
    targets: [
        .target(name: "Service", path: "Sources/Service"),
        .executableTarget(name: "App", dependencies: ["Service"], path: "Sources/App"),
        .testTarget(name: "ServiceTests", dependencies: ["Service"], path: "Tests/ServiceTests"),
    ]
)
