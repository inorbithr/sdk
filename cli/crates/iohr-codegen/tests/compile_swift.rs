//! The generated Swift surface builds against the `InOrbit` package (the runtime, at the
//! repository root), and a call a profile may not make does not. Runs with
//! `IOHR_TEST_COMPILE=swift` (the swift CI job sets it): it writes a throwaway package
//! depending on the runtime by path and runs `swift build`.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

mod support;

use std::path::Path;
use std::process::Command;

use iohr_codegen::{Language, Options};

use support::{compile_enabled, repo};

const PROGRAM: &str = r#"import InOrbit

/// What any program may write with this surface.
func fine() async throws {
    let personal = try Client<Personal>(ClientOptions(token: Secret("t")))
    _ = try await personal.me()
    _ = try await personal.radar.listDigests(RadarListDigestsParams(limit: 5))
    for try await digest in personal.radar.allListDigests() {
        _ = digest
    }
    let ci = try Client<AcmeCi>.fromEnv()
    _ = try await ci.accounts.getMe()
    _ = try await ci.accounts.getUsage(orgId: "acc_1", AccountsGetUsageParams(from: "2026-09-01"))
    let made = try await ci.events.createEndpoint(body: CreateEndpointRequest(url: "https://example.com"))
    _ = made.value.secret
    for try await event in ci.events.streamEvents(EventsStreamEventsParams(types: "key.created")) {
        _ = event.type
    }
}
"#;

/// The personal cut has no usage:read and no events:read: each call must not compile.
const WRONG: [&str; 2] = [
    "_ = try await personal.accounts.getUsage(orgId: \"acc_1\")",
    "for try await _ in personal.events.streamEvents() {}",
];

fn package(root: &Path, runtime: &Path) {
    let identity = runtime
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();
    std::fs::write(
        root.join("Package.swift"),
        format!(
            r#"// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Check",
    platforms: [.macOS(.v12)],
    dependencies: [.package(path: {runtime:?})],
    targets: [
        .target(
            name: "Check",
            dependencies: [.product(name: "InOrbit", package: "{identity}")],
            path: "Sources/Check"
        )
    ]
)
"#,
            runtime = runtime.display().to_string()
        ),
    )
    .unwrap();
}

fn build(root: &Path) -> (bool, String) {
    let output = Command::new("swift")
        .args(["build", "--package-path"])
        .arg(root)
        .output()
        .unwrap();
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn the_two_profile_surface_builds_and_the_wrong_profile_does_not() {
    if !compile_enabled(Language::Swift) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=swift to run it (CI does); skipping");
        return;
    }
    let runtime = repo().canonicalize().unwrap();
    let (_, files) = support::render("two-profiles", Language::Swift, &Options::default());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    package(root, &runtime);
    let sources = root.join("Sources/Check");
    files.write(&sources.join("Generated"), true).unwrap();
    std::fs::write(sources.join("Program.swift"), PROGRAM).unwrap();
    let (ok, text) = build(root);
    assert!(ok, "swift build failed on the generated surface:\n{text}");
    for call in WRONG {
        std::fs::write(
            sources.join("Wrong.swift"),
            format!(
                "import InOrbit\n\nfunc wrong(personal: Client<Personal>) async throws {{\n    {call}\n}}\n"
            ),
        )
        .unwrap();
        let (ok, text) = build(root);
        assert!(
            !ok,
            "the personal profile may not make this call, yet it built: {call}"
        );
        assert!(
            text.contains("Personal") || text.contains("has no member"),
            "{call} failed for another reason:\n{text}"
        );
    }
}
