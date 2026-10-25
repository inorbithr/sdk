import Foundation

/// How a client is built: the credential, where it calls, and its limits. The same names
/// as every other language's (docs/design.md section 2, case adjusted).
public struct ClientOptions: Sendable {
    /// The API key's id (`ak_...`); with `keySecret` and `scopes`.
    public var keyId: String?
    /// The API key's secret.
    public var keySecret: Secret?
    /// The scopes to ask for, a subset of the key's. A token with no scope can call nothing.
    public var scopes: [String]?
    /// An access token used as it is, instead of a key.
    public var token: Secret?
    /// Where tokens come from, for an app that already holds one; wins over the above.
    public var tokenProvider: (any TokenProvider)?
    /// The API, `https://api.inorbit.hr` by default; `https://` except on loopback [SR-07].
    public var baseURL: URL?
    /// The token endpoint, `https://auth.inorbit.hr/oauth2/token` by default.
    public var tokenURL: URL?
    /// One attempt's timeout, 30 s by default [SR-19].
    public var timeout: TimeInterval
    /// The whole call's timeout, retries and their waits included; none by default. A retry
    /// whose wait would pass it is not made.
    public var totalTimeout: TimeInterval?
    /// How many times a call is retried; 0 turns retries off.
    public var maxRetries: Int
    /// How long a stream may be silent (no event, no comment) before it fails, 45 s by
    /// default.
    public var streamIdleTimeout: TimeInterval
    /// The HTTP transport, `URLSession` by default: bring your own for proxies, pinning or
    /// tests.
    public var transport: (any HTTPTransport)?
    /// Appended to the SDK's user agent.
    public var userAgentSuffix: String?

    /// Options; what is left `nil` takes its default.
    public init(
        keyId: String? = nil,
        keySecret: Secret? = nil,
        scopes: [String]? = nil,
        token: Secret? = nil,
        tokenProvider: (any TokenProvider)? = nil,
        baseURL: URL? = nil,
        tokenURL: URL? = nil,
        timeout: TimeInterval = 30,
        totalTimeout: TimeInterval? = nil,
        maxRetries: Int = 2,
        streamIdleTimeout: TimeInterval = 45,
        transport: (any HTTPTransport)? = nil,
        userAgentSuffix: String? = nil
    ) {
        self.keyId = keyId
        self.keySecret = keySecret
        self.scopes = scopes
        self.token = token
        self.tokenProvider = tokenProvider
        self.baseURL = baseURL
        self.tokenURL = tokenURL
        self.timeout = timeout
        self.totalTimeout = totalTimeout
        self.maxRetries = maxRetries
        self.streamIdleTimeout = streamIdleTimeout
        self.transport = transport
        self.userAgentSuffix = userAgentSuffix
    }
}

/// What one call may change.
public struct CallOptions: Sendable {
    /// This call's attempt timeout, instead of the client's.
    public var timeout: TimeInterval?
    /// The `Idempotency-Key` to send on an operation that takes one (by default a fresh one
    /// per call, the same on every attempt) [SR-18].
    public var idempotencyKey: String?
    /// A W3C `traceparent` to send, from the caller's own trace.
    public var traceparent: String?

    /// Call options; what is left `nil` keeps the client's behaviour.
    public init(timeout: TimeInterval? = nil, idempotencyKey: String? = nil, traceparent: String? = nil) {
        self.timeout = timeout
        self.idempotencyKey = idempotencyKey
        self.traceparent = traceparent
    }
}
