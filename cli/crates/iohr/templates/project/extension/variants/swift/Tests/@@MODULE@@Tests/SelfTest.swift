import Foundation
import XCTest

@testable import @@MODULE@@

/// The self-test: `swift test` runs one heartbeat with a STATE and CACHE of its own, then checks
/// what it wrote and where. No package, so there is nothing to download.
/// The global `bind`: XCTestCase has a method of the same name that would shadow it.
private func bindSocket(_ fd: Int32, _ address: UnsafePointer<sockaddr>, _ length: socklen_t) -> Int32 {
    bind(fd, address, length)
}

final class SelfTest: XCTestCase {
    private func temp(_ name: String) throws -> String {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("\(name)-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.path
    }

    private func env(_ state: String, _ cache: String, _ more: [String: String] = [:]) -> [String: String] {
        ["STATE": state, "CACHE": cache].merging(more) { _, new in new }
    }

    func testOneHeartbeatSaysWhoAndOk() throws {
        let state = try temp("state")
        let cache = try temp("cache")
        XCTAssertEqual(runService(environment: env(state, cache, ["IOHR_EXT_ONCE": "1"])), 0)
        let text = try String(contentsOfFile: "\(state)/heartbeat.json", encoding: .utf8)
        let beat = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])
        XCTAssertEqual(beat["id"] as? String, "@@ID@@")
        XCTAssertEqual(beat["state"] as? String, "ok")
        XCTAssertEqual(beat["beats"] as? Int, 1)
        // Granted when you are signed in and the channel is up, refused when not: either way it answered.
        XCTAssertTrue(["granted", "refused"].contains(beat["token"] as? String ?? ""))
        let mode = try FileManager.default.attributesOfItem(atPath: "\(state)/heartbeat.json")[.posixPermissions] as? Int
        XCTAssertEqual(mode, 0o600)
    }

    func testNoStateAndCacheIsAnErrorExit2() {
        XCTAssertEqual(runService(environment: [:]), 2)
    }

    func testWithoutTheChannelTheTokenAskIsRefused() {
        XCTAssertThrowsError(try token(scopes: ["tokens:write"], environment: [:])) { XCTAssertTrue($0 is Refused) }
    }

    func testTheChannelGrantsTheDeclaredScopeAndRefusesAnother() throws {
        let socketPath = try temp("chan") + "/t.sock"
        let server = socket(AF_UNIX, SOCK_STREAM, 0)
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (i, b) in socketPath.utf8.enumerated() { raw[i] = b }
        }
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bindSocket(server, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        XCTAssertEqual(bound, 0)
        XCTAssertEqual(listen(server, 2), 0)
        let fake = Thread {
            for _ in 0..<2 {
                let c = accept(server, nil, nil)
                var buffer = [UInt8](repeating: 0, count: 4096)
                let n = recv(c, &buffer, buffer.count, 0)
                let ok = String(decoding: buffer[0..<max(n, 0)], as: UTF8.self).contains("identity:read")
                let body = ok ? "{\"access_token\":\"t\",\"expires_at\":\"x\"}" : "{\"error\":\"scope\",\"message\":\"not declared\"}"
                let answer = "HTTP/1.1 \(ok ? "200 OK" : "403 Forbidden")\r\ncontent-length: \(body.utf8.count)\r\n\r\n\(body)"
                _ = answer.withCString { send(c, $0, strlen($0), 0) }
                close(c)
            }
        }
        fake.start()
        let e = ["IOHR_EXT_TOKEN_SOCKET": socketPath]
        XCTAssertEqual(try token(scopes: serviceScopes, environment: e), "t")
        XCTAssertThrowsError(try token(scopes: ["tokens:write"], environment: e)) {
            XCTAssertTrue("\($0)".contains("403"))
        }
        close(server)
    }

    func testThePanelServesTheHeartbeatThePageAndA404() throws {
        let state = try temp("state")
        let cache = try temp("cache")
        let page = try temp("page") + "/index.html"
        try "<html><script>fetch('/api/heartbeat')</script></html>".write(toFile: page, atomically: true, encoding: .utf8)
        let latest = Latest()
        latest.set(["id": "@@ID@@", "beats": 1])
        let panel = try servePanel(environment: env(state, cache, ["PANEL_FILE": page]), state: state, latest: latest)
        defer { panel.stop() }
        let written = try String(contentsOfFile: "\(state)/panel.json", encoding: .utf8)
        XCTAssertTrue(written.contains(panel.url))
        XCTAssertTrue(panel.url.hasPrefix("http://127.0.0.1:"))

        func get(_ path: String) throws -> (Int, String) {
            let done = DispatchSemaphore(value: 0)
            var result: (Int, String) = (0, "")
            URLSession.shared.dataTask(with: URL(string: panel.url + path)!) { data, response, _ in
                result = ((response as? HTTPURLResponse)?.statusCode ?? 0, String(decoding: data ?? Data(), as: UTF8.self))
                done.signal()
            }.resume()
            XCTAssertEqual(done.wait(timeout: .now() + 10), .success)
            return result
        }
        let api = try get("api/heartbeat")
        XCTAssertEqual(api.0, 200)
        XCTAssertTrue(api.1.contains("\"beats\":1"))
        XCTAssertTrue(try get("").1.contains("/api/heartbeat"))
        XCTAssertEqual(try get("nope").0, 404)
    }

    func testNothingIsWrittenIntoTheProjectFolder() throws {
        let before = try FileManager.default.contentsOfDirectory(atPath: ".").sorted()
        let state = try temp("state")
        let cache = try temp("cache")
        _ = runService(environment: env(state, cache, ["IOHR_EXT_ONCE": "1"]))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: ".").sorted(), before)
    }
}
