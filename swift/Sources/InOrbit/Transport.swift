import Foundation

#if canImport(FoundationNetworking)
    import FoundationNetworking
#endif

/// An HTTP method.
public enum HTTPMethod: String, Sendable {
    case get = "GET"
    case post = "POST"
    case put = "PUT"
    case patch = "PATCH"
    case delete = "DELETE"
    case head = "HEAD"

    /// Whether repeating the request is safe by the method alone.
    public var isIdempotent: Bool {
        switch self {
        case .get, .head, .put, .delete: return true
        case .post, .patch: return false
        }
    }
}

/// One request as the transport sends it.
public struct HTTPRequest: Sendable {
    /// The method.
    public var method: HTTPMethod
    /// The full URL.
    public var url: URL
    /// The headers, names in lower case.
    public var headers: [String: String]
    /// The body, if any.
    public var body: Data?
    /// How long the attempt may take.
    public var timeout: TimeInterval
}

/// An answer's status and headers.
public struct HTTPResponseHead: Sendable {
    /// The HTTP status.
    public var status: Int
    /// The headers, names in lower case.
    public var headers: [String: String]

    public init(status: Int, headers: [String: String]) {
        self.status = status
        self.headers = headers
    }
}

/// A whole answer.
public struct HTTPResponse: Sendable {
    /// The status and headers.
    public var head: HTTPResponseHead
    /// The body.
    public var body: Data

    public init(head: HTTPResponseHead, body: Data) {
        self.head = head
        self.body = body
    }
}

/// Why a transport could not deliver a request: the connection failed, or the attempt
/// timed out. Anything else a transport throws is treated as a connection failure.
public enum TransportFailure: Error, Sendable {
    /// DNS, TCP, TLS, or a reset before the answer.
    case connection(String)
    /// The attempt took longer than its timeout.
    case timeout
}

/// Sends requests. `URLSessionTransport` is the default; a custom one routes through a
/// proxy, pins keys, or answers from a test. A transport must verify TLS and never offer a
/// way to turn that off [SR-01, SR-02].
public protocol HTTPTransport: Sendable {
    /// Sends `request` and reads the whole answer.
    func send(_ request: HTTPRequest) async throws -> HTTPResponse
    /// Sends `request` and answers once the head arrives, with the body as it streams in;
    /// the stream ends when the body does. Dropping the stream cancels the request.
    func open(_ request: HTTPRequest) async throws -> (HTTPResponseHead, AsyncThrowingStream<Data, any Error>)
}

/// The default transport, on `URLSession`: an ephemeral session (no cookies, no cache,
/// nothing on disk), TLS 1.2 at least, redirects not followed, so a token is only ever
/// sent to the host it was meant for.
public final class URLSessionTransport: HTTPTransport, @unchecked Sendable {
    private let session: URLSession
    private let delegate: SessionDelegate

    /// A transport on a session of its own.
    public init() {
        let config = URLSessionConfiguration.ephemeral
        config.httpCookieStorage = nil
        config.httpShouldSetCookies = false
        config.urlCache = nil
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        #if !canImport(FoundationNetworking)
            config.tlsMinimumSupportedProtocolVersion = .TLSv12
        #endif
        let delegate = SessionDelegate()
        self.delegate = delegate
        self.session = URLSession(configuration: config, delegate: delegate, delegateQueue: nil)
    }

    deinit {
        session.finishTasksAndInvalidate()
    }

    private func urlRequest(_ request: HTTPRequest) -> URLRequest {
        var r = URLRequest(url: request.url)
        r.httpMethod = request.method.rawValue
        r.timeoutInterval = request.timeout
        for (k, v) in request.headers {
            r.setValue(v, forHTTPHeaderField: k)
        }
        r.httpBody = request.body
        return r
    }

    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let req = urlRequest(request)
        let box = TaskBox()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<HTTPResponse, any Error>) in
                let task = session.dataTask(with: req) { data, response, error in
                    if let error {
                        cont.resume(throwing: Self.failure(error))
                        return
                    }
                    guard let http = response as? HTTPURLResponse else {
                        cont.resume(throwing: TransportFailure.connection("the answer was not HTTP"))
                        return
                    }
                    cont.resume(returning: HTTPResponse(head: Self.head(http), body: data ?? Data()))
                }
                box.set(task)
                task.resume()
            }
        } onCancel: {
            box.cancel()
        }
    }

    public func open(_ request: HTTPRequest) async throws -> (HTTPResponseHead, AsyncThrowingStream<Data, any Error>) {
        let req = urlRequest(request)
        let (chunks, chunkContinuation) = AsyncThrowingStream<Data, any Error>.makeStream()
        let box = TaskBox()
        let head = try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<HTTPResponseHead, any Error>) in
                let task = session.dataTask(with: req)
                delegate.register(task, head: cont, chunks: chunkContinuation)
                chunkContinuation.onTermination = { _ in task.cancel() }
                box.set(task)
                task.resume()
            }
        } onCancel: {
            box.cancel()
        }
        return (head, chunks)
    }

    static func head(_ http: HTTPURLResponse) -> HTTPResponseHead {
        var headers: [String: String] = [:]
        for (k, v) in http.allHeaderFields {
            if let k = k as? String, let v = v as? String {
                headers[k.lowercased()] = v
            }
        }
        return HTTPResponseHead(status: http.statusCode, headers: headers)
    }

    static func failure(_ error: any Error) -> any Error {
        if let u = error as? URLError {
            switch u.code {
            case .timedOut: return TransportFailure.timeout
            case .cancelled: return CancellationError()
            default: return TransportFailure.connection(u.localizedDescription)
            }
        }
        return TransportFailure.connection(String(describing: type(of: error)))
    }
}

/// The task of a call, cancelled from wherever the caller cancels.
private final class TaskBox: @unchecked Sendable {
    private let lock = NSLock()
    private var task: URLSessionTask?
    private var cancelled = false

    func set(_ task: URLSessionTask) {
        lock.lock()
        defer { lock.unlock() }
        self.task = task
        if cancelled { task.cancel() }
    }

    func cancel() {
        lock.lock()
        defer { lock.unlock() }
        cancelled = true
        task?.cancel()
    }
}

/// Streams a body as it arrives, and refuses redirects.
private final class SessionDelegate: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private struct Entry {
        var head: CheckedContinuation<HTTPResponseHead, any Error>?
        let chunks: AsyncThrowingStream<Data, any Error>.Continuation
    }

    private let lock = NSLock()
    private var entries: [Int: Entry] = [:]

    func register(
        _ task: URLSessionTask, head: CheckedContinuation<HTTPResponseHead, any Error>,
        chunks: AsyncThrowingStream<Data, any Error>.Continuation
    ) {
        lock.lock()
        defer { lock.unlock() }
        entries[task.taskIdentifier] = Entry(head: head, chunks: chunks)
    }

    private func take(_ id: Int, finish: Bool) -> Entry? {
        lock.lock()
        defer { lock.unlock() }
        if finish { return entries.removeValue(forKey: id) }
        return entries[id]
    }

    func urlSession(
        _ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        lock.lock()
        let head = entries[dataTask.taskIdentifier]?.head
        entries[dataTask.taskIdentifier]?.head = nil
        lock.unlock()
        if let http = response as? HTTPURLResponse {
            head?.resume(returning: URLSessionTransport.head(http))
        } else {
            head?.resume(throwing: TransportFailure.connection("the answer was not HTTP"))
        }
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        take(dataTask.taskIdentifier, finish: false)?.chunks.yield(data)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        guard let entry = take(task.taskIdentifier, finish: true) else { return }
        if let error {
            let failure = URLSessionTransport.failure(error)
            entry.head?.resume(throwing: failure)
            entry.chunks.finish(throwing: failure)
        } else {
            entry.head?.resume(throwing: TransportFailure.connection("the connection closed before an answer"))
            entry.chunks.finish()
        }
    }

    func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}
