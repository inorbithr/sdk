import Foundation
import XCTest

@testable import InOrbit

#if canImport(FoundationNetworking)
    import FoundationNetworking
#endif

final class RuntimeTests: XCTestCase {
    // MARK: secrets [SR-10]

    func testASecretNeverPrints() {
        let s = Secret("s3cr3t-do-not-print")
        var dumped = ""
        dump(s, to: &dumped)
        var options = ClientOptions(keyId: "ak_1", keySecret: s, scopes: ["identity:read"])
        options.token = Secret("tok-do-not-print")
        var dumpedOptions = ""
        dump(options, to: &dumpedOptions)
        for text in [
            "\(s)", String(describing: s), String(reflecting: s), dumped, "\(options)", String(reflecting: options),
            dumpedOptions,
        ] {
            XCTAssertFalse(text.contains("do-not-print"), text)
        }
        XCTAssertEqual(s.reveal(), "s3cr3t-do-not-print")
    }

    func testAMissingScopeNamesTheKeyNotTheSecret() {
        XCTAssertThrowsError(try Client<Public>(ClientOptions(keyId: "ak_1", keySecret: Secret("s3cr3t")))) { e in
            let message = (e as? ConfigError)?.message ?? ""
            XCTAssertTrue(message.contains("ak_1"), message)
            XCTAssertFalse(message.contains("s3cr3t"), message)
        }
    }

    // MARK: configuration

    func testNoCredentialNamesTheEnvironment() {
        XCTAssertThrowsError(try Client<Public>.fromEnv([:])) { e in
            XCTAssertTrue("\(e)".contains("INORBIT_TOKEN"), "\(e)")
        }
        enum AcmeCi: Profile {
            static let name = "acme-ci"
            static let env = "ACME_CI"
        }
        // A named profile never falls back to the bare credential.
        XCTAssertThrowsError(try Client<AcmeCi>.fromEnv(["INORBIT_TOKEN": "t"])) { e in
            XCTAssertTrue("\(e)".contains("INORBIT_ACME_CI_TOKEN"), "\(e)")
        }
        XCTAssertNoThrow(try Client<AcmeCi>.fromEnv(["INORBIT_ACME_CI_TOKEN": "t"]))
    }

    func testOnlyHTTPSExceptOnLoopback() {
        let bad = URL(string: "http://api.example.com")!
        XCTAssertThrowsError(try Client<Public>(ClientOptions(token: Secret("t"), baseURL: bad)))
        XCTAssertNoThrow(
            try Client<Public>(ClientOptions(token: Secret("t"), baseURL: URL(string: "http://127.0.0.1:9")!)))
        XCTAssertNoThrow(
            try Client<Public>(ClientOptions(token: Secret("t"), baseURL: URL(string: "https://api.example.com")!)))
    }

    func testPlainHTTPGoesOnlyToThisMachine() {
        for host in ["127.example.com", "127.0.0.1.example.com", "localhost.example", "::2", "[::1].example"] {
            XCTAssertFalse(Core.isLoopback(host), host)
        }
        for host in ["localhost", "LocalHost", "127.0.0.1", "127.1.2.3", "::1", "[::1]", "0:0:0:0:0:0:0:1"] {
            XCTAssertTrue(Core.isLoopback(host), host)
        }
        XCTAssertThrowsError(
            try Client<Public>(ClientOptions(token: Secret("t"), baseURL: URL(string: "http://127.example.com")!)))
        XCTAssertNoThrow(
            try Client<Public>(ClientOptions(token: Secret("t"), baseURL: URL(string: "http://[::1]:8080")!)))
    }

    func testARequestNeverPrintsItsToken() {
        let request = HTTPRequest(
            method: .get, url: URL(string: "https://api.example.com/v1/me")!,
            headers: ["authorization": "Bearer tok-do-not-print", "accept": "application/json"],
            body: Data("{}".utf8), timeout: 5)
        var dumped = ""
        dump(request, to: &dumped)
        for text in ["\(request)", String(reflecting: request), dumped] {
            XCTAssertFalse(text.contains("do-not-print"), text)
        }
        XCTAssertTrue("\(request)".contains("authorization: <redacted>"), "\(request)")
    }

    // MARK: answer size

    func testAnOversizedChunkedAnswerIsRefusedWhileItStreams() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [EndlessBody.self]
        let transport = URLSessionTransport(configuration: config)
        let request = HTTPRequest(
            method: .get, url: URL(string: "https://endless.test/")!, headers: [:], body: nil, timeout: 30)
        do {
            _ = try await transport.send(request)
            XCTFail("an answer past 16 MiB was read")
        } catch let e as BodyTooLarge {
            XCTAssertEqual(e.limit, 16 << 20)
        }
        // It stopped near the limit, not at the end of the 64 MiB body.
        XCTAssertLessThan(EndlessBody.sent.value, 32 << 20)

        // A caller's own transport that reads it whole is refused by the client.
        var options = ClientOptions(token: Secret("t"), baseURL: URL(string: "https://endless.test")!)
        options.transport = TooBig()
        let client = try Client<Public>(options)
        let op = Codegen.Operation(name: "me", method: .get, path: "/v1/me", template: "/v1/me", scopes: [])
        do {
            _ = try await client.core.call(op, options: CallOptions())
            XCTFail("an answer past 16 MiB was accepted")
        } catch let e as TooLargeError {
            XCTAssertTrue(e.message.contains("16 MiB"), e.message)
        }
    }

    func testTheUserAgentNamesTheSDK() {
        let ua = Core.userAgent(suffix: "my-app/1.0")
        XCTAssertTrue(ua.hasPrefix("inorbithr-sdk-swift/\(SDK.version) swift/"), ua)
        XCTAssertTrue(ua.hasSuffix(" my-app/1.0"), ua)
    }

    // MARK: wire

    func testA64BitIntegerTravelsAsADecimalString() throws {
        let row = UsageRow(amount: 9_007_199_254_740_993, day: "2026-09-01", metric: "calls", service: "radar")
        let data = try JSONEncoder().encode(row)
        let text = String(decoding: data, as: UTF8.self)
        XCTAssertTrue(text.contains("\"amount\":\"9007199254740993\""), text)
        let back = try JSONDecoder().decode(UsageRow.self, from: data)
        XCTAssertEqual(back.amount, 9_007_199_254_740_993)
    }

    func testARequestLeavesOutWhatIsUnset() throws {
        let body = CreateEndpointRequest(url: "https://example.com/hook")
        let object = try JSONSerialization.jsonObject(with: Codegen.json(body)) as? [String: Any]
        XCTAssertEqual(object?.keys.sorted(), ["url"])
    }

    func testAnAnswerKeepsWorkingWithFieldsItDoesNotKnow() throws {
        let data = Data(#"{"subject":"ak_1","kind":"client","scopes":["identity:read"],"brand_new":1}"#.utf8)
        let me = try JSONDecoder().decode(Me.self, from: data)
        XCTAssertEqual(me.subject, "ak_1")
    }

    func testAnUnsetTimestampIsNoValue() {
        XCTAssertNil(Timestamps.parse(""))
        XCTAssertNotNil(Timestamps.parse("2026-10-04T08:00:00Z"))
        XCTAssertNotNil(Timestamps.parse("2026-10-04T08:00:00.123Z"))
    }

    func testAPathParameterIsOneSegment() {
        XCTAssertEqual(Codegen.pathSegment("a/b c"), "a%2Fb%20c")
        XCTAssertEqual(Codegen.pathSegment("org_1.x~y-z"), "org_1.x~y-z")
    }

    // MARK: errors

    func testAnUnknownCodeIsKept() throws {
        let raw = RawResponse(
            status: 400, headers: [:],
            body: Data(#"{"code":"brand_new","error":"New.","details":[{"type":"future"}]}"#.utf8),
            requestId: "r1", attempts: 1, idempotencyKey: nil)
        let e = APIError(raw: raw)
        XCTAssertEqual(e.code.rawValue, "brand_new")
        XCTAssertFalse(e.code.isKnown)
        if case .unknown = e.details.first {} else { XCTFail("an unknown detail is kept: \(e.details)") }
        XCTAssertTrue(e.message.contains("brand_new"), e.message)
    }

    func testAPlainTextAnswerMapsByStatus() {
        let raw = RawResponse(
            status: 403, headers: ["content-type": "text/plain"], body: Data("RBAC: access denied".utf8),
            requestId: "r1", attempts: 1, idempotencyKey: nil)
        let e = APIError(raw: raw)
        XCTAssertEqual(e.code, .forbidden)
        XCTAssertTrue(e.message.contains("RBAC: access denied"))
    }

    func testRetryAfterIsCappedAndBackoffIsBounded() {
        let raw = RawResponse(
            status: 429, headers: ["retry-after": "600"], body: Data(), requestId: "r1", attempts: 1,
            idempotencyKey: nil)
        XCTAssertEqual(APIError(raw: raw).retryAfter, 60)
        for retry in 0..<10 {
            let d = RetryPolicy.delay(retry: retry, retryAfter: nil)
            XCTAssertTrue(d >= 0 && d <= 8, "\(d)")
        }
        XCTAssertEqual(RetryPolicy.parseRetryAfter("3"), 3)
    }

    // MARK: server-sent events

    func testEventsSplitOnEveryLineEnding() throws {
        var p = EventStreamParser()
        var events = try p.feed(Data(": open\n\nid: 7\nretry: 1000\nfoo: bar\ndata: {\"a\":\ndata:  1}\n\n".utf8))
        events += try p.feed(Data("event: error\r\ndata: x\r".utf8))
        events += try p.feed(Data("\n\r\ndata: y\r\r".utf8))
        XCTAssertEqual(
            events,
            [
                ServerEvent(name: "message", data: "{\"a\":\n 1}"),
                ServerEvent(name: "error", data: "x"),
                ServerEvent(name: "message", data: "y"),
            ])
    }

    func testAnEventLargerThanTheLimitFails() {
        var p = EventStreamParser(limit: 8)
        XCTAssertThrowsError(try p.feed(Data("data: 0123456789\n\n".utf8)))
    }

    // MARK: pages

    func testPagesFollowTheTokenAndStopOnARepeat() async throws {
        let calls = Counter()
        let items = Codegen.pages { (token: String?) -> ([Int], String) in
            await calls.add()
            switch token {
            case nil: return ([1, 2], "p2")
            case "p2": return ([3], "p2")
            default: return ([], "")
            }
        }
        var seen: [Int] = []
        for try await i in items { seen.append(i) }
        XCTAssertEqual(seen, [1, 2, 3])
        let n = await calls.value
        XCTAssertEqual(n, 2)
    }

    func testPagesFetchNothingMoreOnceTheLoopStops() async throws {
        let calls = Counter()
        let items = Codegen.pages { (token: String?) -> ([Int], String) in
            await calls.add()
            return ([1, 2], "next-\(token ?? "0")")
        }
        for try await _ in items { break }
        let n = await calls.value
        XCTAssertEqual(n, 1)
    }
}

actor Counter {
    private(set) var value = 0
    func add() { value += 1 }
}

/// Answers a chunked body that never ends, one `chunk` a millisecond, until the client stops.
final class EndlessBody: URLProtocol {
    static let chunk = 64 << 10
    static let sent = Tally()
    private var stopped = Tally()

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let response = HTTPURLResponse(
            url: request.url!, statusCode: 200, httpVersion: "HTTP/1.1",
            headerFields: ["transfer-encoding": "chunked", "content-type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        let block = Data(repeating: 0x20, count: Self.chunk)
        let me = Unchecked(value: self)
        DispatchQueue.global().async {
            let this = me.value
            // 64 MiB at most, so a broken cap fails the test rather than hanging it.
            for _ in 0..<1024 where this.stopped.value == 0 {
                Self.sent.add(Self.chunk)
                this.client?.urlProtocol(this, didLoad: block)
                Thread.sleep(forTimeInterval: 0.001)
            }
            if this.stopped.value == 0 { this.client?.urlProtocolDidFinishLoading(this) }
        }
    }

    override func stopLoading() { stopped.add(1) }
}

/// Hands a value to another thread; the test owns its synchronisation.
struct Unchecked<T>: @unchecked Sendable {
    let value: T
}

/// A transport that answers 16 MiB and one byte, whole.
struct TooBig: HTTPTransport {
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        HTTPResponse(head: HTTPResponseHead(status: 200, headers: [:]), body: Data(count: (16 << 20) + 1))
    }

    func open(_ request: HTTPRequest) async throws -> (HTTPResponseHead, AsyncThrowingStream<Data, any Error>) {
        throw TransportFailure.connection("no streams here")
    }
}

/// A count shared across threads.
final class Tally: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    var value: Int {
        lock.lock()
        defer { lock.unlock() }
        return n
    }
    func add(_ k: Int) {
        lock.lock()
        defer { lock.unlock() }
        n += k
    }
}
