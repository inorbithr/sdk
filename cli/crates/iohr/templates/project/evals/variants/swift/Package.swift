// swift-tools-version:5.9
// @@NAME@@: an offline eval suite with a pass bar. Foundation only, no package dependency.
import PackageDescription

let package = Package(
    name: "@@NAME@@",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "@@NAME@@", targets: ["@@MODULE@@_run"]),
    ],
    targets: [
        .target(name: "@@MODULE@@"),
        .executableTarget(name: "@@MODULE@@_run", dependencies: ["@@MODULE@@"]),
        .testTarget(name: "@@MODULE@@Tests", dependencies: ["@@MODULE@@"]),
    ]
)
