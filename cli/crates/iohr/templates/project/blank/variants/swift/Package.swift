// swift-tools-version: 6.0
import PackageDescription

// No dependencies. The library holds the code so the test can import it.
let package = Package(
    name: "@@NAME@@",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "@@NAME@@", targets: ["App"])],
    targets: [
        .target(name: "Hello", path: "Sources/Hello"),
        .executableTarget(name: "App", dependencies: ["Hello"], path: "Sources/App"),
        .testTarget(name: "HelloTests", dependencies: ["Hello"], path: "Tests/HelloTests"),
    ]
)
