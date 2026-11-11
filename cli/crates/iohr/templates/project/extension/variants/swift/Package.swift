// swift-tools-version:5.9
// @@NAME@@: the demo project's private macOS service. Foundation only, no package dependency.
import PackageDescription

let package = Package(
    name: "@@NAME@@",
    products: [
        .executable(name: "@@NAME@@", targets: ["@@MODULE@@_run"]),
    ],
    targets: [
        .target(name: "@@MODULE@@"),
        .executableTarget(name: "@@MODULE@@_run", dependencies: ["@@MODULE@@"]),
        .testTarget(name: "@@MODULE@@Tests", dependencies: ["@@MODULE@@"]),
    ]
)
