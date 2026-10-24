import Foundation

/// An HTTP answer as it came, for anything the typed value does not carry.
public struct RawResponse: Sendable {
    /// The HTTP status.
    public let status: Int
    /// The response headers, names in lower case.
    public let headers: [String: String]
    /// The body.
    public let body: Data
    /// The `x-request-id` this SDK sent, the same on every attempt of the call [SR-17].
    public let requestId: String
    /// How many attempts the call took.
    public let attempts: Int
    /// The `Idempotency-Key` the call was sent with, on an operation that takes one.
    public let idempotencyKey: String?

    /// The request id the API answered with, if any.
    public var serverRequestId: String? { header("x-request-id") }

    /// Whether the API answered a repeat of an earlier call with the same key
    /// (`Idempotency-Replayed: true`).
    public var idempotencyReplayed: Bool {
        header("idempotency-replayed")?.trimmingCharacters(in: .whitespaces) == "true"
    }

    /// The value of the header `name` (any case).
    public func header(_ name: String) -> String? {
        headers[name.lowercased()]
    }

    /// The body as text.
    public var text: String {
        String(decoding: body, as: UTF8.self)
    }
}

/// An operation's answer: the typed value and the raw answer beside it.
public struct Response<Value: Sendable>: Sendable {
    /// The answer, typed.
    public let value: Value
    /// The answer as it came.
    public let raw: RawResponse
}
