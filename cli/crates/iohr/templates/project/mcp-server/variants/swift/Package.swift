// swift-tools-version:6.1
import PackageDescription

let package = Package(
    name: "@@NAME@@",
    platforms: [.macOS("13.0")],
    products: [.executable(name: "@@NAME@@", targets: ["App"])],
    dependencies: [
        // The official Swift SDK, pinned to an exact release (Package.resolved pins the revision).
        .package(url: "https://github.com/modelcontextprotocol/swift-sdk.git", exact: "0.12.1")
    ],
    targets: [
        .executableTarget(name: "App", dependencies: [.product(name: "MCP", package: "swift-sdk")]),
        .testTarget(
            name: "AppTests",
            dependencies: ["App", .product(name: "MCP", package: "swift-sdk")]),
    ]
)
