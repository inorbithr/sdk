// The driver for `conformance/cases`: starts the replay server, loads every case, runs its
// action through the generated public surface, and compares the result and the server's
// verdict with `expect` (conformance/README.md). Without the server binary (`mise run
// conformance:server:build`) it skips, unless IOHR_TEST_REQUIRE_REPLAY=1.

#if os(macOS) || os(Linux)
    import Foundation
    import XCTest

    #if canImport(FoundationNetworking)
        import FoundationNetworking
    #endif

    @testable import InOrbit

    final class ConformanceTests: XCTestCase {
        static let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()

        func testEveryCase() async throws {
            let bin = Self.root.appendingPathComponent("conformance/server/bin/replay")
            guard FileManager.default.isExecutableFile(atPath: bin.path) else {
                let note =
                    "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`"
                XCTAssertNil(ProcessInfo.processInfo.environment["IOHR_TEST_REQUIRE_REPLAY"], note)
                throw XCTSkip(note)
            }
            let replay = try Replay.start(bin, cases: Self.root.appendingPathComponent("conformance/cases"))
            defer { replay.stop() }
            let names = try await replay.cases()
            XCTAssertFalse(names.isEmpty, "the replay server listed no case")
            var ran = 0
            var skipped: [String] = []
            for name in names {
                let loaded = try await replay.load(name)
                let c = loaded["case"] as? [String: Any] ?? [:]
                if (c["pending"] as? [String] ?? []).contains("swift") {
                    skipped.append(name)
                    continue
                }
                ran += 1
                let problems = await run(c, loaded: loaded, replay: replay)
                for p in problems {
                    XCTFail("\(name): \(p)")
                }
            }
            print("conformance: \(ran) cases ran, \(skipped.count) pending for swift")
        }

        // MARK: - one case

        func run(_ c: [String: Any], loaded: [String: Any], replay: Replay) async -> [String] {
            let action = c["action"] as? [String: Any] ?? [:]
            let expect = c["expect"] as? [String: Any] ?? [:]
            let client: Client<Public>
            do {
                client = try build(c, loaded: loaded)
            } catch {
                return ["the client could not be built: \(error)"]
            }
            var problems: [String] = []
            let op = action["op"] as? String ?? ""
            var lastValue: Any?
            var lastRaw: RawResponse?
            var lastError: (any Error)?
            if op == "events.stream_events" {
                let (items, error) = await stream(client, action)
                lastError = error
                if let want = expect["items"] as? [Any] {
                    if !subset(want, items) {
                        problems.append("items: want \(want), got \(items)")
                    }
                }
            } else {
                let times = action["repeat"] as? Int ?? 1
                let concurrent = action["concurrent"] as? Int ?? 1
                if concurrent > 1 {
                    let results = await withTaskGroup(of: Result<(Any, RawResponse), any Error>.self) { group in
                        for _ in 0..<concurrent {
                            group.addTask { await self.call(client, action) }
                        }
                        var all: [Result<(Any, RawResponse), any Error>] = []
                        for await r in group { all.append(r) }
                        return all
                    }
                    for r in results {
                        switch r {
                        case .success(let (v, raw)): (lastValue, lastRaw) = (v, raw)
                        case .failure(let e): lastError = e
                        }
                    }
                } else {
                    for _ in 0..<times {
                        switch await call(client, action) {
                        case .success(let (v, raw)):
                            (lastValue, lastRaw, lastError) = (v, raw, nil)
                        case .failure(let e):
                            lastError = e
                        }
                    }
                }
                if let want = expect["ok"] {
                    if let lastError {
                        problems.append("want a result, got \(lastError)")
                    } else if !subset(want, lastValue as Any) {
                        problems.append("result: want \(want), got \(lastValue as Any)")
                    }
                }
            }
            if let want = expect["error"] as? [String: Any] {
                if let e = lastError as? any InOrbitError {
                    problems += checkError(e, want)
                } else {
                    problems.append("want an error \(want), got \(lastError.map { "\($0)" } ?? "none")")
                }
            } else if expect["ok"] == nil, expect["items"] != nil, let lastError {
                problems.append("want the stream to end cleanly, got \(lastError)")
            }
            if let want = expect["idempotency_key"] as? String {
                let got = lastRaw?.idempotencyKey ?? (lastError as? any InOrbitError)?.idempotencyKey
                if !(want == "*" ? got != nil : got == want) {
                    problems.append("idempotency key: want \(want), got \(got ?? "none")")
                }
            }
            for unsupported in ["probes", "logs", "spans", "rate_limit", "config"] where expect[unsupported] != nil {
                problems.append(
                    "expect.\(unsupported) is not supported by this driver yet; mark the case pending for swift")
            }
            do {
                let verdict = try await replay.result()
                if verdict["status"] as? String != "pass" {
                    problems.append("server verdict: \(verdict)")
                }
                for key in ["attempts", "token_exchanges"] {
                    if let want = expect[key] as? Int, let got = verdict[key] as? Int, want != got {
                        problems.append("\(key): want \(want), got \(got)")
                    }
                }
            } catch {
                problems.append("no verdict: \(error)")
            }
            return problems
        }

        func build(_ c: [String: Any], loaded: [String: Any]) throws -> Client<Public> {
            let o = c["client"] as? [String: Any] ?? [:]
            for unsupported in [
                "config_file", "files", "profile", "credential_sources", "cli", "pipeline", "log", "log_headers",
                "log_allow_headers", "rate_limit", "retry_budget_capacity", "tracing", "no_proxy", "streams",
            ] where o[unsupported] != nil {
                if unsupported == "tracing", o[unsupported] as? Bool == false { continue }
                if unsupported == "streams", o[unsupported] as? String == "sse" { continue }
                throw DriverError("client.\(unsupported) is not supported by this driver yet")
            }
            if let t = o["transport"] as? String, t != "http" {
                throw DriverError("client.transport \(t) is not supported by this driver yet")
            }
            let base = loaded["base_url"] as? String ?? ""
            var options = ClientOptions()
            if let ms = o["timeout_ms"] as? Int { options.timeout = Double(ms) / 1000 }
            if let ms = o["total_timeout_ms"] as? Int { options.totalTimeout = Double(ms) / 1000 }
            if let ms = o["stream_idle_timeout_ms"] as? Int { options.streamIdleTimeout = Double(ms) / 1000 }
            options.maxRetries = o["max_retries"] as? Int ?? 2
            if o["load"] as? Bool == true {
                var env: [String: String] = [:]
                for (k, v) in o["env"] as? [String: String] ?? [:] {
                    env[k] = v.replacingOccurrences(of: "{replay}", with: base)
                }
                return try Client<Public>.fromEnv(env, options: options)
            }
            options.baseURL = URL(string: base)
            options.tokenURL = URL(string: base + "/oauth2/token")
            options.keyId = o["key_id"] as? String ?? "ak_test"
            options.keySecret = Secret(o["key_secret"] as? String ?? "s3cr3t")
            options.scopes = o["scopes"] as? [String] ?? ["identity:read"]
            return try Client<Public>(options)
        }

        func call(_ api: Client<Public>, _ action: [String: Any]) async -> Result<(Any, RawResponse), any Error> {
            let args = action["args"] as? [String: Any] ?? [:]
            let arg = { (k: String) -> String? in args[k].map { "\($0)" } }
            let o = action["options"] as? [String: Any] ?? [:]
            let options = CallOptions(
                timeout: (o["timeout_ms"] as? Int).map { Double($0) / 1000 },
                idempotencyKey: o["idempotency_key"] as? String,
                traceparent: o["traceparent"] as? String)
            do {
                switch action["op"] as? String {
                case "me":
                    return .success(try json(await api.me(options: options)))
                case "accounts.get_me":
                    return .success(try json(await api.accounts.getMe(options: options)))
                case "accounts.get_usage":
                    let q = AccountsGetUsageParams(from: arg("from"), to: arg("to"))
                    return .success(
                        try json(await api.accounts.getUsage(orgId: arg("org_id") ?? "", q, options: options)))
                case "radar.get_digest":
                    return .success(try json(await api.radar.getDigest(id: arg("id") ?? "", options: options)))
                case "events.create_endpoint":
                    let body = try decode(CreateEndpointRequest.self, args)
                    return .success(try json(await api.events.createEndpoint(body: body, options: options)))
                case "events.update_endpoint":
                    var rest = args
                    rest.removeValue(forKey: "endpoint_id")
                    let body = try decode(UpdateEndpointRequest.self, rest)
                    return .success(
                        try json(
                            await api.events.updateEndpoint(
                                endpointId: arg("endpoint_id") ?? "", body: body, options: options)))
                case "events.delete_endpoint":
                    return .success(
                        try json(
                            await api.events.deleteEndpoint(endpointId: arg("endpoint_id") ?? "", options: options)))
                case let other:
                    throw DriverError(
                        "the conformance schema names an op this driver does not know: \(other ?? "none")")
                }
            } catch {
                return .failure(error)
            }
        }

        func stream(_ api: Client<Public>, _ action: [String: Any]) async -> ([Any], (any Error)?) {
            let args = action["args"] as? [String: Any] ?? [:]
            let q = EventsStreamEventsParams(types: args["types"] as? String, accountId: args["account_id"] as? String)
            var items: [Any] = []
            do {
                for try await item in api.events.streamEvents(q) {
                    items.append(try encode(item))
                    if let take = action["take"] as? Int, items.count >= take { break }
                }
            } catch {
                return (items, error)
            }
            try? await Task.sleep(nanoseconds: 100_000_000)
            return (items, nil)
        }

        func checkError(_ e: any InOrbitError, _ want: [String: Any]) -> [String] {
            var problems: [String] = []
            if let kind = want["kind"] as? String, e.kind.rawValue != kind {
                problems.append("error kind: want \(kind), got \(e.kind.rawValue) (\(e.message))")
            }
            if let api = e as? APIError {
                if let code = want["code"] as? String, api.code.rawValue != code {
                    problems.append("error code: want \(code), got \(api.code)")
                }
                if let status = want["status"] as? Int, api.status != status {
                    problems.append("error status: want \(status), got \(api.status)")
                }
            } else if want["code"] != nil || want["status"] != nil {
                problems.append("error: want an API error, got \(e.message)")
            }
            if let s = want["message_contains"] as? String, !e.message.contains(s) {
                problems.append("message: want it to contain \(s), got \(e.message)")
            }
            if let s = want["message_excludes"] as? String, e.message.contains(s) {
                problems.append("message: must not contain \(s), got \(e.message)")
            }
            return problems
        }
    }

    // MARK: - helpers

    struct DriverError: Error, CustomStringConvertible {
        let description: String
        init(_ d: String) { description = d }
    }

    func encode<T: Encodable>(_ value: T) throws -> Any {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value), options: [.fragmentsAllowed])
    }

    func json<T: Encodable & Sendable>(_ r: Response<T>) throws -> (Any, RawResponse) {
        (try encode(r.value), r.raw)
    }

    func decode<T: Decodable>(_ type: T.Type, _ object: [String: Any]) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
    }

    /// `want` is a subset of `got`: objects by key, arrays element by element with the same
    /// length, scalars equal.
    func subset(_ want: Any, _ got: Any) -> Bool {
        if let w = want as? [Any] {
            guard let g = got as? [Any], g.count == w.count else { return false }
            return zip(w, g).allSatisfy { subset($0, $1) }
        }
        if let w = want as? [String: Any] {
            guard let g = got as? [String: Any] else { return false }
            return w.allSatisfy { k, v in g[k].map { subset(v, $0) } ?? false }
        }
        if let w = want as? NSObject, let g = got as? NSObject { return w.isEqual(g) }
        return false
    }

    /// The replay server, one per test run.
    final class Replay: @unchecked Sendable {
        let process: Process
        let url: String

        static func start(_ bin: URL, cases: URL) throws -> Replay {
            let p = Process()
            p.executableURL = bin
            p.arguments = ["--addr", "127.0.0.1:0", "--cases", cases.path]
            let out = Pipe()
            p.standardOutput = out
            try p.run()
            var line = Data()
            let deadline = Date().addingTimeInterval(10)
            while Date() < deadline {
                let byte = out.fileHandleForReading.readData(ofLength: 1)
                if byte.isEmpty || byte == Data([0x0A]) { break }
                line.append(byte)
            }
            let first = String(decoding: line, as: UTF8.self)
            let prefix = "replay: listening on "
            guard first.hasPrefix(prefix) else {
                p.terminate()
                throw DriverError("unexpected first line from the replay server: \(first)")
            }
            return Replay(process: p, url: String(first.dropFirst(prefix.count)).trimmingCharacters(in: .whitespaces))
        }

        init(process: Process, url: String) {
            self.process = process
            self.url = url
        }

        func stop() {
            process.terminate()
        }

        private func control(_ method: String, _ path: String, _ body: Any? = nil) async throws -> Any {
            var r = URLRequest(url: URL(string: url + path)!)
            r.httpMethod = method
            if let body {
                r.httpBody = try JSONSerialization.data(withJSONObject: body)
                r.setValue("application/json", forHTTPHeaderField: "content-type")
            }
            let (data, _) = try await URLSession.shared.data(for: r)
            return try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
        }

        func cases() async throws -> [String] {
            let v = try await control("GET", "/_cases")
            if let list = v as? [String] { return list }
            if let o = v as? [String: Any], let list = o["cases"] as? [String] { return list }
            throw DriverError("unexpected /_cases answer: \(v)")
        }

        func load(_ name: String) async throws -> [String: Any] {
            guard let v = try await control("POST", "/_case", ["name": name]) as? [String: Any] else {
                throw DriverError("unexpected /_case answer")
            }
            return v
        }

        func result() async throws -> [String: Any] {
            guard let v = try await control("GET", "/_result") as? [String: Any] else {
                throw DriverError("unexpected /_result answer")
            }
            return v
        }
    }
#endif
