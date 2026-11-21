// Observability on the standard library alone: JSON logs, spans that follow the W3C
// traceparent header, and request counters in the Prometheus text format.
//
// Nothing here records a header, a query string or an environment value: a log line and a span
// name the route that matched, the status, the duration and the trace id.
import Foundation

/// A JSON value a log line can hold.
public enum Field: Sendable {
    case string(String)
    case int(Int)
    case double(Double)
}

/// A JSON string literal: quotes, backslashes and control characters escaped.
func jsonString(_ s: String) -> String {
    var out = "\""
    for u in s.unicodeScalars {
        switch u {
        case "\"": out += "\\\""
        case "\\": out += "\\\\"
        case "\n": out += "\\n"
        case "\r": out += "\\r"
        case "\t": out += "\\t"
        default:
            if u.value < 0x20 || u.value == 0x7f {
                out += String(format: "\\u%04x", u.value)
            } else {
                out.unicodeScalars.append(u)
            }
        }
    }
    return out + "\""
}

/// One JSON object per line to a sink.
public final class Logger: Sendable {
    private let sink: @Sendable (String) -> Void

    public init(_ sink: @escaping @Sendable (String) -> Void) { self.sink = sink }

    public func log(_ level: String, _ msg: String, _ fields: [(String, Field)] = []) {
        let ts = ISO8601DateFormatter.string(from: Date(), timeZone: .gmt, formatOptions: [.withInternetDateTime, .withFractionalSeconds])
        var line = "{\"ts\":\(jsonString(ts)),\"level\":\(jsonString(level)),\"msg\":\(jsonString(msg))"
        for (key, value) in fields {
            switch value {
            case .string(let s): line += ",\(jsonString(key)):\(jsonString(s))"
            case .int(let n): line += ",\(jsonString(key)):\(n)"
            case .double(let d): line += ",\(jsonString(key)):\(d)"
            }
        }
        sink(line + "}")
    }

    public func info(_ msg: String, _ fields: [(String, Field)] = []) { log("INFO", msg, fields) }
    public func error(_ msg: String, _ fields: [(String, Field)] = []) { log("ERROR", msg, fields) }
}

private func hex(_ bytes: Int) -> String {
    (0..<bytes).map { _ in String(format: "%02x", UInt8.random(in: 0...255)) }.joined()
}

private func isHex(_ s: Substring) -> Bool {
    s.allSatisfy { $0.isASCII && ($0.isNumber || ("a"..."f").contains($0)) }
}

/// One request's trace: the id it joined (or started) and its own id.
public struct Span: Sendable {
    public let traceId: String
    public let spanId = hex(8)
    public let parentId: String?
    private let started = DispatchTime.now().uptimeNanoseconds

    /// Joins the trace of a valid traceparent header, or starts a new one.
    public init(traceparent: String?) {
        let parts = (traceparent ?? "").split(separator: "-", omittingEmptySubsequences: false)
        if parts.count == 4, parts[0] == "00", parts[1].count == 32, parts[2].count == 16, parts[3].count == 2,
            parts.allSatisfy(isHex), parts[1].contains(where: { $0 != "0" }), parts[2].contains(where: { $0 != "0" })
        {
            traceId = String(parts[1])
            parentId = String(parts[2])
        } else {
            traceId = hex(16)
            parentId = nil
        }
    }

    /// The traceparent to send back (and on to callers): this span is the parent.
    public var header: String { "00-\(traceId)-\(spanId)-01" }

    public var millis: Double {
        (Double(DispatchTime.now().uptimeNanoseconds - started) / 1000).rounded() / 1000
    }
}

/// Request counts by route and status. Routes are matched patterns, never raw paths, so the
/// label set is bounded by the code and holds no user input.
public final class Metrics: @unchecked Sendable {
    private let lock = NSLock()
    private var counts: [String: Int] = [:]

    public init() {}

    public func inc(route: String, status: Int) {
        lock.lock()
        defer { lock.unlock() }
        counts["route=\"\(route)\",status=\"\(status)\"", default: 0] += 1
    }

    public func render() -> String {
        lock.lock()
        defer { lock.unlock() }
        var out = "# HELP http_requests_total Requests served, by route and status.\n# TYPE http_requests_total counter\n"
        for key in counts.keys.sorted() { out += "http_requests_total{\(key)} \(counts[key]!)\n" }
        return out
    }
}
