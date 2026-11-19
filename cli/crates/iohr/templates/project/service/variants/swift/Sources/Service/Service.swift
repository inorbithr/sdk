// The routes, and the one place a request is traced, logged and counted. HTTP/1.1 over the C
// socket API: one thread accepts, each connection is read with a size limit and a timeout,
// answered once and closed.
import Foundation

#if canImport(Glibc)
    import Glibc
    private let streamType = Int32(SOCK_STREAM.rawValue)
#elseif canImport(Darwin)
    import Darwin
    private let streamType = SOCK_STREAM
#endif

public struct ConfigError: Error, CustomStringConvertible {
    public let description: String
}

public struct Config: Sendable, Equatable {
    public var host = "127.0.0.1"
    public var port: UInt16 = 8080

    public init(host: String = "127.0.0.1", port: UInt16 = 8080) {
        self.host = host
        self.port = port
    }

    /// HOST and PORT from the environment: a bad PORT throws.
    public static func load(_ env: [String: String]) throws -> Config {
        var cfg = Config()
        if let h = env["HOST"], !h.isEmpty { cfg.host = h }
        if let p = env["PORT"], !p.isEmpty {
            guard let n = UInt16(p), p.allSatisfy(\.isASCII) else { throw ConfigError(description: "PORT must be a number from 0 to 65535") }
            cfg.port = n
        }
        return cfg
    }
}

public enum ServeError: Error { case cannotListen }

private struct Reply {
    var route: String
    var status: Int
    var body: String
    var type = "application/json"
}

private let reasons = [200: "OK", 400: "Bad Request", 404: "Not Found", 405: "Method Not Allowed", 503: "Service Unavailable"]

private func json(_ route: String, _ status: Int, _ key: String, _ value: String) -> Reply {
    Reply(route: route, status: status, body: "{\(jsonString(key)):\(jsonString(value))}\n")
}

public final class Service: @unchecked Sendable {
    public let log: Logger
    public let metrics = Metrics()
    private let lock = NSLock()
    private var isReady = false
    private var listener: Int32 = -1

    public init(log: Logger) { self.log = log }

    /// False until the server listens and again as soon as shutdown starts, so a load balancer
    /// stops sending traffic before the process stops.
    public var ready: Bool {
        get { lock.withLock { isReady } }
        set { lock.withLock { isReady = newValue } }
    }

    /// Listens and returns the port (useful when started on port 0).
    public func start(_ cfg: Config) throws -> UInt16 {
        let fd = socket(AF_INET, streamType, 0)
        guard fd >= 0 else { throw ServeError.cannotListen }
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &on, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = cfg.port.bigEndian
        guard inet_pton(AF_INET, cfg.host, &addr.sin_addr) == 1 else {
            close(fd)
            throw ServeError.cannotListen
        }
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard bound == 0, listen(fd, 128) == 0 else {
            close(fd)
            throw ServeError.cannotListen
        }
        var actual = sockaddr_in()
        var len = socklen_t(MemoryLayout<sockaddr_in>.size)
        _ = withUnsafeMutablePointer(to: &actual) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &len) }
        }
        lock.withLock { listener = fd }
        Thread.detachNewThread { [self] in acceptLoop(fd) }
        ready = true
        return UInt16(bigEndian: actual.sin_port)
    }

    /// Ready goes false first, then the listener closes; requests already read are answered.
    public func stop() {
        ready = false
        let fd = lock.withLock { () -> Int32 in
            defer { listener = -1 }
            return listener
        }
        if fd >= 0 {
            shutdown(fd, Int32(SHUT_RDWR))
            close(fd)
        }
    }

    private func acceptLoop(_ fd: Int32) {
        while true {
            let client = accept(fd, nil, nil)
            if client < 0 { return }
            Thread.detachNewThread { [self] in serve(client) }
        }
    }

    private func serve(_ fd: Int32) {
        defer { close(fd) }
        // A slow client is cut off: five seconds to send the request, then the socket closes.
        var timeout = timeval(tv_sec: 5, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        var data = Data()
        var buf = [UInt8](repeating: 0, count: 2048)
        let terminator = Data("\r\n\r\n".utf8)
        // Requests are read up to 8 KiB of head; a body is never read.
        while data.range(of: terminator) == nil && data.count < 8192 {
            let n = recv(fd, &buf, buf.count, 0)
            if n <= 0 { return }
            data.append(contentsOf: buf[0..<n])
        }
        let head = String(decoding: data, as: UTF8.self).components(separatedBy: "\r\n")
        let line = (head.first ?? "").split(separator: " ", omittingEmptySubsequences: true)
        let traceparent = head.dropFirst().first { $0.lowercased().hasPrefix("traceparent:") }
            .map { String($0.dropFirst("traceparent:".count)).trimmingCharacters(in: .whitespaces) }
        let span = Span(traceparent: traceparent)
        let isGet = line.count >= 2 && line[0] == "GET"
        let reply = isGet ? respond(String(line[1])) : json("unmatched", 405, "error", "method not allowed")
        // Logged and counted before the reply goes out, so a caller that has its answer can
        // already read the line.
        let method = isGet ? "GET" : "OTHER"
        let common: [(String, Field)] = [
            ("trace_id", .string(span.traceId)), ("route", .string(reply.route)),
            ("status", .int(reply.status)), ("duration_ms", .double(span.millis)),
        ]
        log.info("span", [("span_id", .string(span.spanId)), ("name", .string("\(method) \(reply.route)"))] + common
            + (span.parentId.map { [("parent_span_id", Field.string($0))] } ?? []))
        log.info("request", [("method", .string(method))] + common)
        metrics.inc(route: reply.route, status: reply.status)
        var out = "HTTP/1.1 \(reply.status) \(reasons[reply.status] ?? "OK")\r\n"
        out += "Content-Type: \(reply.type)\r\nContent-Length: \(reply.body.utf8.count)\r\n"
        out += "traceparent: \(span.header)\r\nConnection: close\r\n\r\n\(reply.body)"
        var bytes = Array(out.utf8)[...]
        while !bytes.isEmpty {
            let n = bytes.withUnsafeBytes { send(fd, $0.baseAddress, $0.count, 0) }
            if n <= 0 { break }
            bytes = bytes.dropFirst(n)
        }
    }

    private func respond(_ target: String) -> Reply {
        guard let url = URLComponents(string: target) else { return json("unmatched", 404, "error", "not found") }
        switch url.path {
        case "/healthz": return json("/healthz", 200, "status", "ok")
        case "/readyz":
            return ready ? json("/readyz", 200, "status", "ready") : json("/readyz", 503, "status", "not ready")
        case "/metrics":
            return Reply(route: "/metrics", status: 200, body: metrics.render(), type: "text/plain; version=0.0.4")
        case "/v1/hello":
            // Your first route: replace it. The name comes from the query string, so it is
            // bounded and echoed back, never logged.
            let given = url.queryItems?.first { $0.name == "name" }?.value ?? ""
            let name = given.isEmpty ? "world" : given
            if name.unicodeScalars.count > 64 {
                return json("/v1/hello", 400, "error", "name is at most 64 characters of text")
            }
            return json("/v1/hello", 200, "message", "Hello, \(name)")
        default: return json("unmatched", 404, "error", "not found")
        }
    }
}
