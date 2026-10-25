import Foundation

/// The largest event data a stream reads.
let maxEventBytes = 1 << 20

extension Core {
    /// A stream that opens on its first step: dropping it before then sends nothing, and
    /// dropping it later cancels the request (design.md section 7).
    func stream<T: Decodable & Sendable>(
        _ op: Codegen.Operation, as type: T.Type, options: CallOptions
    ) -> AsyncThrowingStream<T, any Error> {
        let lazy = LazyStream<T> { [self] in
            AsyncThrowingStream<T, any Error> { continuation in
                let task = Task {
                    do {
                        try await self.runStream(op, options: options) { continuation.yield($0) }
                        continuation.finish()
                    } catch {
                        continuation.finish(throwing: error)
                    }
                }
                continuation.onTermination = { _ in task.cancel() }
            }
        }
        return AsyncThrowingStream(unfolding: { try await lazy.next() })
    }

    /// Opens the stream as a `GET` with the retries and the token refresh of any call, then
    /// reads its server-sent events until the body ends, an error event, or the idle timeout.
    func runStream<T: Decodable & Sendable>(
        _ op: Codegen.Operation, options: CallOptions, yield: @escaping @Sendable (T) -> Void
    ) async throws {
        let call = plan(op, options)
        var retry = 0
        var attempts = 0
        var refreshed = false
        var refresh = false
        var opened: (HTTPResponseHead, AsyncThrowingStream<Data, any Error>)?
        while opened == nil {
            let token = try await tokens.token(refresh: refresh)
            refresh = false
            let request = HTTPRequest(
                method: op.method, url: url(op),
                headers: headers(op, call, options, token: token, accept: "text/event-stream"),
                body: op.body, timeout: call.timeout)
            attempts += 1
            let head: HTTPResponseHead
            let body: AsyncThrowingStream<Data, any Error>
            do {
                let transport = self.transport
                (head, body) = try await withTimeout(call.timeout) { try await transport.open(request) }
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                let wait = RetryPolicy.delay(retry: retry, retryAfter: nil)
                if mayRetry(retry, call, wait: wait) {
                    try await sleep(wait)
                    retry += 1
                    continue
                }
                throw failure(error, op, call)
            }
            if (200..<300).contains(head.status) {
                opened = (head, body)
                break
            }
            var data = Data()
            for try await chunk in body {
                data.append(chunk)
                if data.count > 16 << 20 { break }
            }
            let raw = RawResponse(
                status: head.status, headers: head.headers, body: data, requestId: call.requestId,
                attempts: attempts, idempotencyKey: call.idempotencyKey)
            if head.status == 401, !refreshed {
                refreshed = true
                refresh = true
                continue
            }
            let error = APIError(raw: raw)
            if RetryPolicy.retryable(status: head.status) {
                let wait = RetryPolicy.delay(retry: retry, retryAfter: error.retryAfter)
                if mayRetry(retry, call, wait: wait) {
                    try await sleep(wait)
                    retry += 1
                    continue
                }
            }
            throw error
        }
        guard let (head, body) = opened else { return }
        let raw = { (bytes: Data) in
            RawResponse(
                status: head.status, headers: head.headers, body: bytes, requestId: call.requestId,
                attempts: attempts, idempotencyKey: call.idempotencyKey)
        }
        var parser = EventStreamParser()
        let guarded = idleGuard(body, idle: streamIdleTimeout)
        do {
            for try await chunk in guarded {
                for event in try parser.feed(chunk) {
                    try deliver(event, op: op, call: call, raw: raw, yield: yield)
                }
            }
        } catch is IdleTimeout {
            throw TimeoutError(
                message:
                    "\(op.name) was silent for \(streamIdleTimeout) s, so the stream ended; open it again (request id \(call.requestId))",
                requestId: call.requestId, idempotencyKey: call.idempotencyKey)
        } catch let e as EventTooLarge {
            throw TooLargeError(
                message: "an event of \(op.name) was larger than \(e.limit) bytes (request id \(call.requestId))",
                requestId: call.requestId, idempotencyKey: call.idempotencyKey)
        } catch let e as TransportFailure {
            throw failure(e, op, call)
        }
    }

    private func deliver<T: Decodable & Sendable>(
        _ event: ServerEvent, op: Codegen.Operation, call: Call, raw: (Data) -> RawResponse,
        yield: (T) -> Void
    ) throws {
        let data = Data(event.data.utf8)
        if event.name == "error" {
            let envelope = try? JSONDecoder().decode(APIError.Envelope.self, from: data)
            let code = Code(rawValue: envelope?.code ?? "unknown")
            throw APIError(
                status: code.status ?? 0, code: code, problem: envelope?.error ?? "",
                details: envelope?.details ?? [], raw: raw(data))
        }
        do {
            yield(try JSONDecoder().decode(T.self, from: data))
        } catch {
            throw DecodeError(
                message:
                    "an event of \(op.name) is not a \(T.self): \(Core.describe(error)) (request id \(call.requestId))",
                requestId: call.requestId, idempotencyKey: call.idempotencyKey)
        }
    }

    /// `source`, ended with `IdleTimeout` once nothing at all arrives for `idle` seconds.
    func idleGuard(_ source: AsyncThrowingStream<Data, any Error>, idle: TimeInterval) -> AsyncThrowingStream<
        Data, any Error
    > {
        AsyncThrowingStream { continuation in
            let clock = ActivityClock()
            let reader = Task {
                do {
                    for try await chunk in source {
                        clock.touch()
                        continuation.yield(chunk)
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            let watchdog = Task {
                let step = min(max(idle / 4, 0.02), 1)
                while !Task.isCancelled {
                    try? await sleep(step)
                    if clock.silence() >= idle {
                        continuation.finish(throwing: IdleTimeout())
                        reader.cancel()
                        return
                    }
                }
            }
            continuation.onTermination = { _ in
                reader.cancel()
                watchdog.cancel()
            }
        }
    }
}

struct IdleTimeout: Error {}

struct EventTooLarge: Error {
    let limit: Int
}

/// When the last byte arrived.
private final class ActivityClock: @unchecked Sendable {
    private let lock = NSLock()
    private var last = Date()

    func touch() {
        lock.lock()
        last = Date()
        lock.unlock()
    }

    func silence() -> TimeInterval {
        lock.lock()
        defer { lock.unlock() }
        return Date().timeIntervalSince(last)
    }
}

/// One dispatched server-sent event.
struct ServerEvent: Equatable {
    /// The `event:` field; `message` when there is none.
    var name: String
    /// The `data:` lines joined with newlines.
    var data: String
}

/// The WHATWG event-stream rules (design.md section 7): lines end with LF, CRLF or CR; a
/// line starting with `:` is a comment; `data:` lines join with a newline, one leading space
/// dropped; `event:` names the event; a blank line dispatches; `id`, `retry` and unknown
/// fields are ignored.
struct EventStreamParser {
    private var line = Data()
    private var pendingCR = false
    private var data = Data()
    private var hasData = false
    private var name = ""
    private let limit: Int

    init(limit: Int = maxEventBytes) {
        self.limit = limit
    }

    /// The events `chunk` completes.
    mutating func feed(_ chunk: Data) throws -> [ServerEvent] {
        var out: [ServerEvent] = []
        for byte in chunk {
            if pendingCR {
                pendingCR = false
                if byte == 0x0A { continue }
            }
            switch byte {
            case 0x0D:
                pendingCR = true
                if let e = try endLine() { out.append(e) }
            case 0x0A:
                if let e = try endLine() { out.append(e) }
            default:
                line.append(byte)
                if line.count > limit { throw EventTooLarge(limit: limit) }
            }
        }
        return out
    }

    private mutating func endLine() throws -> ServerEvent? {
        defer { line.removeAll(keepingCapacity: true) }
        if line.isEmpty {
            defer {
                data.removeAll(keepingCapacity: true)
                hasData = false
                name = ""
            }
            guard hasData else { return nil }
            return ServerEvent(name: name.isEmpty ? "message" : name, data: String(decoding: data, as: UTF8.self))
        }
        if line.first == UInt8(ascii: ":") { return nil }
        let field: Data
        var value: Data
        if let colon = line.firstIndex(of: UInt8(ascii: ":")) {
            field = line[line.startIndex..<colon]
            value = line[line.index(after: colon)...]
            if value.first == UInt8(ascii: " ") { value = value.dropFirst() }
        } else {
            field = line
            value = Data()
        }
        switch String(decoding: field, as: UTF8.self) {
        case "data":
            if hasData { data.append(0x0A) }
            data.append(contentsOf: value)
            hasData = true
            if data.count > limit { throw EventTooLarge(limit: limit) }
        case "event":
            name = String(decoding: value, as: UTF8.self)
        default:
            break
        }
        return nil
    }
}

/// Starts a stream on the first pull. Its one consumer pulls one element at a time, which
/// is what `AsyncThrowingStream(unfolding:)` guarantees, so the iterator is never shared.
private final class LazyStream<T: Sendable>: @unchecked Sendable {
    private let start: @Sendable () -> AsyncThrowingStream<T, any Error>
    private var iterator: AsyncThrowingStream<T, any Error>.AsyncIterator?

    init(_ start: @escaping @Sendable () -> AsyncThrowingStream<T, any Error>) {
        self.start = start
    }

    func next() async throws -> T? {
        if iterator == nil {
            iterator = start().makeAsyncIterator()
        }
        return try await iterator?.next()
    }
}
