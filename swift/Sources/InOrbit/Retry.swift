import Foundation

/// When and how long a failed attempt waits before the next (design.md section 6).
enum RetryPolicy {
    /// The statuses worth another attempt.
    static func retryable(status: Int) -> Bool {
        status == 429 || status == 503 || status == 504
    }

    /// The wait before retry number `retry` (0 for the first): the server's own wait when it
    /// gave one, capped at 60 s, otherwise exponential backoff with full jitter, base 0.5 s,
    /// cap 8 s.
    static func delay(retry: Int, retryAfter: TimeInterval?) -> TimeInterval {
        if let retryAfter { return min(max(retryAfter, 0), 60) }
        let ceiling = min(8, 0.5 * pow(2, Double(retry)))
        return Double.random(in: 0...ceiling)
    }

    /// `Retry-After` as seconds: a number, or an HTTP date.
    static func parseRetryAfter(_ value: String) -> TimeInterval? {
        let v = value.trimmingCharacters(in: .whitespaces)
        if let s = Double(v) { return s }
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "GMT")
        f.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
        guard let d = f.date(from: v) else { return nil }
        return max(d.timeIntervalSinceNow, 0)
    }
}

/// Runs `body` with a time limit; past it, the body is cancelled and `TransportFailure.timeout`
/// is thrown.
func withTimeout<T: Sendable>(_ seconds: TimeInterval, _ body: @escaping @Sendable () async throws -> T) async throws
    -> T
{
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await body() }
        group.addTask {
            try await Task.sleep(nanoseconds: UInt64(max(seconds, 0) * 1_000_000_000))
            throw TransportFailure.timeout
        }
        defer { group.cancelAll() }
        guard let first = try await group.next() else { throw TransportFailure.timeout }
        return first
    }
}

/// Sleeps `seconds`, throwing `CancellationError` when the task is cancelled.
func sleep(_ seconds: TimeInterval) async throws {
    if seconds <= 0 { return }
    try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
}
