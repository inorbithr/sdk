// swift-tools-version:6.1
import PackageDescription

// The standard library and Foundation alone: no dependency to pin or audit.
let package = Package(
    name: "@@NAME@@",
    platforms: [.macOS("13.0")],
    products: [.executable(name: "@@NAME@@", targets: ["App"])],
    targets: [
        .target(name: "AgentKit"),
        .executableTarget(name: "App", dependencies: ["AgentKit"]),
        .testTarget(name: "AppTests", dependencies: ["AgentKit"]),
    ]
)
