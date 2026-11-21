// The passes-on-init test: boots the service on a free port and checks what a service owes.
import Foundation
import XCTest
#if canImport(FoundationNetworking)
    import FoundationNetworking
#endif

@testable import Service

private final class Lines: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String] = []
    func add(_ l: String) { lock.withLock { items.append(l) } }
    func clear() { lock.withLock { items.removeAll() } }
    var all: [String] { lock.withLock { items } }
}

final class ServiceTests: XCTestCase {
    private let lines = Lines()
    private var svc: Service!
    private var base = ""

    override func setUpWithError() throws {
        svc = Service(log: Logger { [lines] in lines.add($0) })
        let port = try svc.start(Config(host: "127.0.0.1", port: 0))
        base = "http://127.0.0.1:\(port)"
    }

    override func tearDown() { svc.stop() }

    private func get(_ path: String, headers: [String: String] = [:]) async throws -> (Int, Data, HTTPURLResponse) {
        var req = URLRequest(url: URL(string: base + path)!)
        for (k, v) in headers { req.setValue(v, forHTTPHeaderField: k) }
        let (data, res) = try await URLSession.shared.data(for: req)
        let http = res as! HTTPURLResponse
        return (http.statusCode, data, http)
    }

    private func records() throws -> [[String: Any]] {
        try lines.all.map { try JSONSerialization.jsonObject(with: Data($0.utf8)) as! [String: Any] }
    }

    func testProbesAnswer() async throws {
        let health = try await get("/healthz").0
        XCTAssertEqual(health, 200)
        let ready = try await get("/readyz").0
        XCTAssertEqual(ready, 200)
        let hello = try await get("/v1/hello?name=Ada")
        XCTAssertEqual(try JSONSerialization.jsonObject(with: hello.1) as? [String: String], ["message": "Hello, Ada"])
        let missing = try await get("/nope").0
        XCTAssertEqual(missing, 404)
    }

    func testOneSpanWithTheRouteAndTheTraceIdAndNoSecret() async throws {
        lines.clear()
        let trace = "4bf92f3577b34da6a3ce929d0e0e4736"
        let res = try await get(
            "/v1/hello?name=secret-query-value",
            headers: ["traceparent": "00-\(trace)-00f067aa0ba902b7-01", "authorization": "Bearer secret-header-value"])
        XCTAssertTrue((res.2.value(forHTTPHeaderField: "traceparent") ?? "").hasPrefix("00-\(trace)-"))
        let spans = try records().filter { $0["msg"] as? String == "span" }
        let requests = try records().filter { $0["msg"] as? String == "request" }
        XCTAssertEqual(spans.count, 1)
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(spans[0]["route"] as? String, "/v1/hello")
        XCTAssertEqual(spans[0]["trace_id"] as? String, trace)
        XCTAssertEqual(spans[0]["parent_span_id"] as? String, "00f067aa0ba902b7")
        XCTAssertEqual(requests[0]["trace_id"] as? String, trace)
        let all = lines.all.joined()
        for secret in ["secret-query-value", "secret-header-value"] {
            XCTAssertFalse(all.contains(secret), "the log holds \(secret)")
        }
        for value in ProcessInfo.processInfo.environment.values where value.count >= 12 {
            XCTAssertFalse(all.contains(value), "the log holds an environment value")
        }
    }

    func testNotReadyWhileShuttingDown() async throws {
        svc.ready = false
        let notReady = try await get("/readyz").0
        XCTAssertEqual(notReady, 503)
        let health = try await get("/healthz").0
        XCTAssertEqual(health, 200)
        svc.ready = true
    }

    func testUnknownPathIsNeverLoggedAndMetricsCountByRoute() async throws {
        lines.clear()
        _ = try await get("/private/user-1234")
        _ = try await get("/healthz")
        XCTAssertFalse(lines.all.joined().contains("user-1234"))
        let metrics = String(decoding: try await get("/metrics").1, as: UTF8.self)
        XCTAssertTrue(metrics.contains("route=\"unmatched\",status=\"404\""))
        XCTAssertTrue(metrics.contains("route=\"/healthz\",status=\"200\""))
    }

    func testConfigurationHasDefaultsAndRefusesABadPort() {
        XCTAssertEqual(try Config.load([:]), Config(host: "127.0.0.1", port: 8080))
        XCTAssertThrowsError(try Config.load(["PORT": "99999"]))
    }
}
