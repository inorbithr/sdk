import Foundation

/// The InOrbit API client for profile `P`: built once and shared across tasks. The
/// operations are in the generated surface, as extensions constrained to the profiles whose
/// cut holds them (`client.radar.listDigests()`); this type holds the runtime: the
/// credential, the retries, the errors and the request path.
public final class Client<P: Profile>: Sendable {
    let core: Core

    /// A client built from `options`.
    ///
    /// - Throws: `ConfigError` when the credential is missing or incomplete, or a URL is not
    ///   `https://` (loopback excepted).
    public init(_ options: ClientOptions) throws {
        core = try Core(options)
    }

    /// A client with its credential from the environment: `INORBIT_TOKEN`, or
    /// `INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` and `INORBIT_SCOPES` (for a named profile,
    /// `INORBIT_<PROFILE>_*` and nothing else), and `INORBIT_BASE_URL` and
    /// `INORBIT_TOKEN_URL` when set.
    public static func fromEnv(_ options: ClientOptions = ClientOptions()) throws -> Client<P> {
        try fromEnv(ProcessInfo.processInfo.environment, options: options)
    }

    /// `fromEnv` reading `env` instead of the process environment.
    public static func fromEnv(_ env: [String: String], options: ClientOptions = ClientOptions()) throws -> Client<P> {
        try Client(EnvSettings.fromEnv(env, profileEnv: P.env, base: options))
    }

    /// The request path every unary operation of a surface calls.
    public func request<T: Decodable & Sendable>(
        _ op: Codegen.Operation, as type: T.Type, options: CallOptions = .init()
    ) async throws -> Response<T> {
        let raw = try await core.call(op, options: options)
        do {
            let body = raw.body.isEmpty ? Data("{}".utf8) : raw.body
            return Response(value: try JSONDecoder().decode(T.self, from: body), raw: raw)
        } catch {
            throw DecodeError(
                message:
                    "the answer to \(op.name) is not a \(T.self): \(Core.describe(error)) (request id \(raw.requestId))",
                requestId: raw.requestId, idempotencyKey: raw.idempotencyKey)
        }
    }

    /// The request path every streaming operation of a surface calls: the events as they
    /// arrive, the stream opening on the first step of the loop (design.md section 7).
    public func stream<T: Decodable & Sendable>(
        _ op: Codegen.Operation, as type: T.Type, options: CallOptions = .init()
    ) -> AsyncThrowingStream<T, any Error> {
        core.stream(op, as: type, options: options)
    }
}

/// How a client is configured from the environment.
enum EnvSettings {
    static func fromEnv(_ env: [String: String], profileEnv: String, base: ClientOptions) throws -> ClientOptions {
        let prefix = profileEnv.isEmpty ? "INORBIT_" : "INORBIT_\(profileEnv)_"
        let value = { (name: String) -> String? in
            guard let v = env[prefix + name], !v.isEmpty else { return nil }
            return v
        }
        // Settings fall back to the bare names; credentials never do.
        let setting = { (name: String) -> String? in
            value(name) ?? env["INORBIT_" + name].flatMap { $0.isEmpty ? nil : $0 }
        }
        var options = base
        if options.tokenProvider == nil, options.token == nil, options.keyId == nil {
            if let token = value("TOKEN") {
                options.token = Secret(token)
            } else {
                options.keyId = value("KEY_ID")
                options.keySecret = value("KEY_SECRET").map(Secret.init)
                if options.scopes == nil {
                    options.scopes = value("SCOPES").map {
                        $0.split(whereSeparator: { $0 == " " || $0 == "," }).map(String.init)
                    }
                }
            }
        }
        if options.baseURL == nil, let u = setting("BASE_URL") {
            guard let url = URL(string: u) else {
                throw ConfigError(message: "\(prefix)BASE_URL is not a URL")
            }
            options.baseURL = url
        }
        if options.tokenURL == nil, let u = setting("TOKEN_URL") {
            guard let url = URL(string: u) else {
                throw ConfigError(message: "\(prefix)TOKEN_URL is not a URL")
            }
            options.tokenURL = url
        }
        if options.tokenProvider == nil, options.token == nil, options.keyId == nil, options.keySecret == nil {
            throw ConfigError(
                message:
                    "no credential: set \(prefix)TOKEN, or \(prefix)KEY_ID, \(prefix)KEY_SECRET and \(prefix)SCOPES")
        }
        return options
    }
}

/// One client's machinery, shared by every profile view of it.
final class Core: Sendable {
    let baseURL: URL
    let transport: any HTTPTransport
    let tokens: any TokenProvider
    let userAgent: String
    let timeout: TimeInterval
    let totalTimeout: TimeInterval?
    let maxRetries: Int
    let streamIdleTimeout: TimeInterval

    init(_ o: ClientOptions) throws {
        let baseURL = o.baseURL ?? URL(string: "https://api.inorbit.hr")!
        let tokenURL = o.tokenURL ?? URL(string: "https://auth.inorbit.hr/oauth2/token")!
        try Core.checkURL(baseURL, "baseURL")
        try Core.checkURL(tokenURL, "tokenURL")
        guard o.timeout > 0, o.streamIdleTimeout > 0 else {
            throw ConfigError(message: "timeout and streamIdleTimeout must be more than 0 [SR-19]")
        }
        guard o.maxRetries >= 0 else {
            throw ConfigError(message: "maxRetries must be 0 or more")
        }
        self.baseURL = baseURL
        self.transport = o.transport ?? URLSessionTransport()
        self.userAgent = Core.userAgent(suffix: o.userAgentSuffix)
        self.timeout = o.timeout
        self.totalTimeout = o.totalTimeout
        self.maxRetries = o.maxRetries
        self.streamIdleTimeout = o.streamIdleTimeout
        if let p = o.tokenProvider {
            tokens = p
        } else if let t = o.token {
            tokens = StaticTokenProvider(t)
        } else if let id = o.keyId, let secret = o.keySecret, !id.isEmpty, !secret.isEmpty {
            guard let scopes = o.scopes, !scopes.isEmpty else {
                throw ConfigError(
                    message:
                        "key \(id) has no scopes to ask for: set scopes (INORBIT_SCOPES); a token with no scope can call nothing"
                )
            }
            tokens = ClientCredentialsProvider(
                keyId: id, keySecret: secret, scopes: scopes, tokenURL: tokenURL, transport: transport,
                userAgent: userAgent, timeout: o.timeout, maxRetries: o.maxRetries)
        } else {
            throw ConfigError(
                message:
                    "no credential: give keyId, keySecret and scopes, a token, or a tokenProvider (INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES, or INORBIT_TOKEN)"
            )
        }
    }

    /// Only `https://`, except on loopback [SR-07].
    static func checkURL(_ url: URL, _ name: String) throws {
        let host = url.host ?? ""
        let loopback = host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]"
        if url.scheme == "https" || (url.scheme == "http" && loopback) { return }
        throw ConfigError(message: "\(name) must be an https:// URL (http:// only on loopback)")
    }

    static func userAgent(suffix: String?) -> String {
        #if os(iOS)
            let os = "ios"
        #elseif os(macOS)
            let os = "macos"
        #elseif os(tvOS)
            let os = "tvos"
        #elseif os(watchOS)
            let os = "watchos"
        #elseif os(visionOS)
            let os = "visionos"
        #elseif os(Linux)
            let os = "linux"
        #elseif os(Windows)
            let os = "windows"
        #else
            let os = "unknown"
        #endif
        #if arch(arm64)
            let arch = "arm64"
        #elseif arch(x86_64)
            let arch = "x86_64"
        #else
            let arch = "unknown"
        #endif
        #if swift(>=6.0)
            let swift = "6"
        #else
            let swift = "5"
        #endif
        var ua = "inorbithr-sdk-swift/\(SDK.version) swift/\(swift) \(os)/\(arch)"
        if let suffix, !suffix.isEmpty { ua += " \(suffix)" }
        return ua
    }

    static func describe(_ error: any Error) -> String {
        switch error {
        case DecodingError.keyNotFound(let key, _): return "the field \(key.stringValue) is missing"
        case DecodingError.typeMismatch(_, let ctx), DecodingError.valueNotFound(_, let ctx),
            DecodingError.dataCorrupted(let ctx):
            let path = ctx.codingPath.map(\.stringValue).joined(separator: ".")
            return path.isEmpty ? "it is not JSON of that shape" : "the field \(path) has the wrong type"
        default: return "it could not be read"
        }
    }

    func url(_ op: Codegen.Operation) -> URL {
        var text = baseURL.absoluteString
        while text.hasSuffix("/") { text.removeLast() }
        text += op.path
        let pairs = op.query.flatMap { name, values in
            values.map { "\(Codegen.formEncode(name))=\(Codegen.formEncode($0))" }
        }
        if !pairs.isEmpty { text += "?" + pairs.joined(separator: "&") }
        return URL(string: text)!
    }

    /// What one call carries on every attempt.
    struct Call {
        let requestId = UUID().uuidString.lowercased()
        let idempotencyKey: String?
        let retrySafe: Bool
        let deadline: Date?
        let timeout: TimeInterval
    }

    func plan(_ op: Codegen.Operation, _ options: CallOptions) -> Call {
        let key = options.idempotencyKey ?? (op.takesIdempotencyKey ? UUID().uuidString.lowercased() : nil)
        return Call(
            idempotencyKey: key,
            retrySafe: op.method.isIdempotent || op.idempotent || key != nil,
            deadline: totalTimeout.map { Date().addingTimeInterval($0) },
            timeout: options.timeout ?? timeout)
    }

    func headers(_ op: Codegen.Operation, _ call: Call, _ options: CallOptions, token: Secret, accept: String)
        -> [String: String]
    {
        var h = [
            "authorization": "Bearer \(token.reveal())",
            "user-agent": userAgent,
            "x-request-id": call.requestId,
            "accept": accept,
        ]
        if op.body != nil { h["content-type"] = "application/json" }
        if let key = call.idempotencyKey { h["idempotency-key"] = key }
        if let tp = options.traceparent { h["traceparent"] = tp }
        return h
    }

    /// Whether a retry that waits `wait` may still be made before the call's deadline.
    func mayRetry(_ retry: Int, _ call: Call, wait: TimeInterval) -> Bool {
        guard call.retrySafe, retry < maxRetries else { return false }
        if let deadline = call.deadline, Date().addingTimeInterval(wait) >= deadline { return false }
        return true
    }

    /// Sends `op` until it is answered, retried or refreshed as design.md sections 3 and 6
    /// say, and answers the successful raw response; an error answer throws `APIError`.
    func call(_ op: Codegen.Operation, options: CallOptions) async throws -> RawResponse {
        let call = plan(op, options)
        var retry = 0
        var attempts = 0
        var refreshed = false
        var refresh = false
        while true {
            let token = try await tokens.token(refresh: refresh)
            refresh = false
            let request = HTTPRequest(
                method: op.method, url: url(op),
                headers: headers(op, call, options, token: token, accept: "application/json"),
                body: op.body, timeout: call.timeout)
            attempts += 1
            let response: HTTPResponse
            do {
                let transport = self.transport
                response = try await withTimeout(call.timeout) { try await transport.send(request) }
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
            let raw = RawResponse(
                status: response.head.status, headers: response.head.headers, body: response.body,
                requestId: call.requestId, attempts: attempts, idempotencyKey: call.idempotencyKey)
            if (200..<300).contains(raw.status) { return raw }
            if raw.status == 401, !refreshed {
                refreshed = true
                refresh = true
                continue
            }
            let error = APIError(raw: raw)
            if RetryPolicy.retryable(status: raw.status) {
                let wait = RetryPolicy.delay(retry: retry, retryAfter: error.retryAfter)
                if mayRetry(retry, call, wait: wait) {
                    try await sleep(wait)
                    retry += 1
                    continue
                }
            }
            throw error
        }
    }

    func failure(_ error: any Error, _ op: Codegen.Operation, _ call: Call) -> any Error {
        if case TransportFailure.timeout? = error as? TransportFailure {
            return TimeoutError(
                message: "\(op.name) timed out after \(call.timeout) s (request id \(call.requestId))",
                requestId: call.requestId, idempotencyKey: call.idempotencyKey)
        }
        var detail = ""
        if case TransportFailure.connection(let why)? = error as? TransportFailure { detail = ": \(why)" }
        return ConnectionError(
            message: "\(op.name) could not reach the API\(detail) (request id \(call.requestId))",
            requestId: call.requestId, idempotencyKey: call.idempotencyKey)
    }
}
