// The passes-on-init test: the server is started the way a host starts it (a child process over
// stdio) and spoken to by the official client. It checks the handshake, the tool list and its
// order, a tool call and its structured result, a refused bad argument, the resource, and the
// rules every tool must meet. SDK 0.12.1 speaks up to 2025-11-25, so it uses `initialize`;
// the 2026-07-28 checks (server/discover, ttlMs, cacheScope) come when the SDK has them.
import Foundation
import MCP
import Testing

#if canImport(System)
    import System
#else
    import SystemPackage
#endif

private final class BundleToken {}

/// The built server: the test target depends on it, so SwiftPM builds it beside the test bundle.
/// On macOS the bundle is `<name>PackageTests.xctest` inside the build folder; on Linux the
/// bundle URL is the build folder itself. So look in the bundle's folder and in its parent.
private func serverURL() throws -> URL {
    let bundle = Bundle(for: BundleToken.self).bundleURL
    let folders = [bundle.deletingLastPathComponent(), bundle]
    for folder in folders {
        let candidate = folder.appendingPathComponent("@@NAME@@")
        if FileManager.default.isExecutableFile(atPath: candidate.path) { return candidate }
    }
    throw MCPError.internalError("the server binary is not beside \(bundle.path); run `swift build` first")
}

private struct Running {
    let client: Client
    let process: Process
    let hello: Initialize.Result

    func stop() async {
        await client.disconnect()
        process.terminate()
    }
}

private func start() async throws -> Running {
    let process = Process()
    process.executableURL = try serverURL()
    let toServer = Pipe()
    let fromServer = Pipe()
    process.standardInput = toServer
    process.standardOutput = fromServer
    process.standardError = FileHandle.nullDevice
    try process.run()
    let transport = StdioTransport(
        input: FileDescriptor(rawValue: fromServer.fileHandleForReading.fileDescriptor),
        output: FileDescriptor(rawValue: toServer.fileHandleForWriting.fileDescriptor))
    let client = Client(name: "test", version: "0")
    let hello = try await client.connect(transport: transport)
    return Running(client: client, process: process, hello: hello)
}

@Suite(.serialized)
struct ServerTests {
    @Test func theHandshakeNamesTheServer() async throws {
        let s = try await start()
        defer { Task { await s.stop() } }
        #expect(s.hello.serverInfo.name == "@@NAME@@")
        #expect(s.hello.protocolVersion == "2025-11-25")
    }

    @Test func toolsListNamesTheToolAndComesInTheSameOrderTwice() async throws {
        let s = try await start()
        defer { Task { await s.stop() } }
        let first = try await s.client.listTools().tools.map(\.name)
        let second = try await s.client.listTools().tools.map(\.name)
        #expect(first == ["word_count"])
        #expect(first == second)
    }

    @Test func aToolCallReturnsTheStructuredResult() async throws {
        let s = try await start()
        defer { Task { await s.stop() } }
        let r = try await s.client.callTool(name: "word_count", arguments: ["text": "one two  three"])
        #expect(r.isError != true)
        guard case .text(let text, _, _) = r.content.first else { Issue.record("no text content"); return }
        #expect(text == "3 words, 14 characters")
    }

    @Test func aBadArgumentIsRefusedAndTheToolDoesNotRun() async throws {
        let s = try await start()
        defer { Task { await s.stop() } }
        let bad: [String: [String: Value]] = [
            "too long": ["text": .string(String(repeating: "x", count: 10_001))],
            "missing": [:],
            "extra": ["text": "a", "other": 1],
            "wrong type": ["text": 1],
        ]
        for (name, arguments) in bad {
            do {
                let r = try await s.client.callTool(name: "word_count", arguments: arguments)
                #expect(r.isError == true, "\(name): was not refused")
            } catch {
                // a protocol error is a refusal too
            }
        }
    }

    @Test func theResourceIsListedAndCanBeRead() async throws {
        let s = try await start()
        defer { Task { await s.stop() } }
        let listed = try await s.client.listResources().resources
        #expect(listed.map(\.uri) == ["about://server"])
        let read = try await s.client.readResource(uri: "about://server")
        #expect(read.first?.text?.contains("counts words") == true)
    }

    @Test func everyToolHasADescriptionAnAnnotationAndBoundedInputs() async throws {
        let s = try await start()
        defer { Task { await s.stop() } }
        for tool in try await s.client.listTools().tools {
            #expect(!(tool.description ?? "").isEmpty, "\(tool.name): no description")
            let a = tool.annotations
            #expect(a.readOnlyHint == true || a.destructiveHint != nil,
                "\(tool.name): say whether it only reads (readOnlyHint) or whether it destroys (destructiveHint)")
            guard case .object(let properties)? = tool.inputSchema.objectValue?["properties"] else { continue }
            for (name, property) in properties {
                let p = property.objectValue ?? [:]
                switch p["type"]?.stringValue {
                case "string":
                    #expect(p["maxLength"] != nil || p["enum"] != nil || p["pattern"] != nil,
                        "\(tool.name).\(name): bound the string (maxLength, enum or pattern)")
                case "integer", "number":
                    #expect(p["minimum"] != nil || p["maximum"] != nil, "\(tool.name).\(name): bound the number (minimum, maximum)")
                default: break
                }
            }
        }
    }
}
