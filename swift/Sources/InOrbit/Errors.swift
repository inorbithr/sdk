import Foundation

/// What kind of failure an error is (design.md section 5).
public enum ErrorKind: String, Sendable {
    /// The API answered with an error.
    case api
    /// DNS, TCP, TLS, or a reset before an answer.
    case connection
    /// The attempt's timeout, the call's total timeout, or a stream's idle timeout.
    case timeout
    /// The token exchange failed.
    case auth
    /// The client's configuration is wrong.
    case config
    /// An answer or a stream event was larger than the SDK reads.
    case tooLarge = "too_large"
    /// An answer was not what its operation says it is.
    case decode
}

/// Every error this package throws: one family, told apart by type or by `kind`. The
/// message says what failed and what to do, and never holds a secret [SR-10, SR-15].
public protocol InOrbitError: Error, Sendable, CustomStringConvertible, LocalizedError {
    /// The kind of failure.
    var kind: ErrorKind { get }
    /// What failed, for a person.
    var message: String { get }
    /// The `x-request-id` the failed call was sent with, when it got that far [SR-17].
    var requestId: String? { get }
    /// The `Idempotency-Key` the failed call was sent with: repeat the call with it to stay
    /// safe.
    var idempotencyKey: String? { get }
}

extension InOrbitError {
    public var description: String { message }
    public var errorDescription: String? { message }
}

/// An error code: one of `spec/problem.json`'s slugs, or a newer one kept as the API wrote
/// it.
public struct Code: RawRepresentable, Hashable, Sendable, Codable, CustomStringConvertible,
    ExpressibleByStringLiteral
{
    /// The slug.
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }

    public init(stringLiteral value: String) {
        self.rawValue = value
    }

    public init(from decoder: any Decoder) throws {
        rawValue = try decoder.singleValueContainer().decode(String.self)
    }

    public func encode(to encoder: any Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }

    public var description: String { rawValue }

    /// The HTTP status the platform answers this code with (`x-http-status`), or `nil` for a
    /// code this version does not know.
    public var status: Int? { Code.statuses[rawValue] }

    /// Whether this version knows the code.
    public var isKnown: Bool { status != nil }

    public static let badRequest: Code = "bad_request"
    public static let failedPrecondition: Code = "failed_precondition"
    public static let unauthenticated: Code = "unauthenticated"
    public static let forbidden: Code = "forbidden"
    public static let notFound: Code = "not_found"
    public static let methodNotAllowed: Code = "method_not_allowed"
    public static let alreadyExists: Code = "already_exists"
    public static let conflict: Code = "conflict"
    public static let payloadTooLarge: Code = "payload_too_large"
    public static let unsupportedMediaType: Code = "unsupported_media_type"
    public static let unprocessable: Code = "unprocessable"
    public static let rateLimited: Code = "rate_limited"
    public static let quotaExceeded: Code = "quota_exceeded"
    public static let cancelled: Code = "cancelled"
    public static let `internal`: Code = "internal"
    public static let unimplemented: Code = "unimplemented"
    public static let unavailable: Code = "unavailable"
    public static let timeout: Code = "timeout"

    static let statuses: [String: Int] = [
        "bad_request": 400, "failed_precondition": 400, "unauthenticated": 401,
        "forbidden": 403, "not_found": 404, "method_not_allowed": 405,
        "already_exists": 409, "conflict": 409, "payload_too_large": 413,
        "unsupported_media_type": 415, "unprocessable": 422, "rate_limited": 429,
        "quota_exceeded": 429, "cancelled": 499, "internal": 500, "unimplemented": 501,
        "unavailable": 503, "timeout": 504,
    ]

    /// The code a plain-text answer with `status` stands for: the gateway answers some
    /// refusals without the envelope (design.md section 5).
    static func forStatus(_ status: Int) -> Code {
        switch status {
        case 400: return .badRequest
        case 401: return .unauthenticated
        case 403: return .forbidden
        case 404: return .notFound
        case 405: return .methodNotAllowed
        case 409: return .conflict
        case 413: return .payloadTooLarge
        case 415: return .unsupportedMediaType
        case 422: return .unprocessable
        case 429: return .rateLimited
        case 501: return .unimplemented
        case 503: return .unavailable
        case 504: return .timeout
        default: return status >= 500 ? .internal : Code(rawValue: "http_\(status)")
        }
    }
}

/// One entry of an error's `details`; a type this version does not know is kept as it
/// came.
public enum Detail: Sendable, Codable, Equatable {
    /// Which request field is wrong, and why.
    case field(field: String, description: String)
    /// Why the call failed, by reason and domain.
    case info(reason: String, domain: String, metadata: [String: String])
    /// How long to wait before trying again.
    case retry(afterSeconds: Double)
    /// A detail this version does not know.
    case unknown(JSONValue)

    public init(from decoder: any Decoder) throws {
        let raw = try JSONValue(from: decoder)
        let s = { (k: String) in raw[k]?.stringValue ?? "" }
        switch raw["type"]?.stringValue {
        case "field":
            self = .field(field: s("field"), description: s("description"))
        case "info":
            var metadata: [String: String] = [:]
            if case .object(let o)? = raw["metadata"] {
                for (k, v) in o { metadata[k] = v.stringValue ?? "" }
            }
            self = .info(reason: s("reason"), domain: s("domain"), metadata: metadata)
        case "retry":
            self = .retry(afterSeconds: raw["after_seconds"]?.numberValue ?? 0)
        default:
            self = .unknown(raw)
        }
    }

    public func encode(to encoder: any Encoder) throws {
        let value: JSONValue
        switch self {
        case .field(let field, let description):
            value = .object([
                "type": .string("field"), "field": .string(field),
                "description": .string(description),
            ])
        case .info(let reason, let domain, let metadata):
            value = .object([
                "type": .string("info"), "reason": .string(reason), "domain": .string(domain),
                "metadata": .object(metadata.mapValues { .string($0) }),
            ])
        case .retry(let after):
            value = .object(["type": .string("retry"), "after_seconds": .number(after)])
        case .unknown(let raw):
            value = raw
        }
        try value.encode(to: encoder)
    }
}

/// The API answered with the problem envelope, or with a plain-text error from the gateway.
public struct APIError: InOrbitError {
    public var kind: ErrorKind { .api }
    /// The HTTP status; 0 for an error event in a stream whose code this version does not
    /// know.
    public let status: Int
    /// The error code.
    public let code: Code
    /// The API's own message (`error`), or the gateway's text.
    public let problem: String
    /// The typed details.
    public let details: [Detail]
    /// The answer as it came.
    public let raw: RawResponse
    public let message: String
    public var requestId: String? { raw.requestId }
    public var idempotencyKey: String? { raw.idempotencyKey }
    /// The request id the API answered with, if any.
    public var serverRequestId: String? { raw.serverRequestId }

    /// How long the API asked to wait (`Retry-After` or a `retry` detail), capped at 60 s.
    public var retryAfter: TimeInterval? {
        if let h = raw.header("retry-after"), let s = RetryPolicy.parseRetryAfter(h) {
            return min(s, 60)
        }
        for d in details {
            if case .retry(let after) = d { return min(after, 60) }
        }
        return nil
    }

    init(raw: RawResponse) {
        var code = Code.forStatus(raw.status)
        var problem = ""
        var details: [Detail] = []
        if let envelope = try? JSONDecoder().decode(Envelope.self, from: raw.body),
            !(envelope.code ?? "").isEmpty || !(envelope.error ?? "").isEmpty
        {
            if let c = envelope.code, !c.isEmpty { code = Code(rawValue: c) }
            problem = envelope.error ?? ""
            details = envelope.details ?? []
        } else {
            problem = String(decoding: raw.body, as: UTF8.self)
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        self.init(status: raw.status, code: code, problem: problem, details: details, raw: raw)
    }

    init(status: Int, code: Code, problem: String, details: [Detail], raw: RawResponse) {
        self.status = status
        self.code = code
        self.details = details
        self.raw = raw
        let text = problem.isEmpty ? APIError.gatewayMessage(status) : APIError.truncate(problem)
        self.problem = text
        self.message = "\(text) (\(code), HTTP \(status), request id \(raw.requestId))"
    }

    struct Envelope: Decodable {
        let code: String?
        let error: String?
        let details: [Detail]?
    }

    static func gatewayMessage(_ status: Int) -> String {
        switch status {
        case 401: return "the token was refused: it is missing, expired or revoked"
        case 403: return "this credential may not call this route: its scopes or role do not allow it"
        case 404: return "no such route or resource"
        case 429: return "too many requests; try again shortly"
        default: return status >= 500 ? "the API failed to answer" : "the request was refused"
        }
    }

    static func truncate(_ s: String) -> String {
        s.count > 300 ? String(s.prefix(300)) + "…" : s
    }
}

/// DNS, TCP or TLS failed, or the connection broke before an answer.
public struct ConnectionError: InOrbitError {
    public var kind: ErrorKind { .connection }
    public let message: String
    public internal(set) var requestId: String?
    public internal(set) var idempotencyKey: String?
}

/// An attempt, the call's total timeout, or a stream's idle timeout passed.
public struct TimeoutError: InOrbitError {
    public var kind: ErrorKind { .timeout }
    public let message: String
    public internal(set) var requestId: String?
    public internal(set) var idempotencyKey: String?
}

/// The key could not be exchanged for a token. The message names the key id, never the
/// secret.
public struct AuthError: InOrbitError {
    public var kind: ErrorKind { .auth }
    /// The token endpoint's HTTP status; 0 when it was not reached.
    public let status: Int
    /// The token endpoint's `error` (`invalid_client`, `invalid_scope`), or empty.
    public let error: String
    public let message: String
    public var requestId: String? { nil }
    public var idempotencyKey: String? { nil }
}

/// The client was configured wrong; the message names what is missing.
public struct ConfigError: InOrbitError {
    public var kind: ErrorKind { .config }
    public let message: String
    public var requestId: String? { nil }
    public var idempotencyKey: String? { nil }
}

/// An answer was not what its operation says it is.
public struct DecodeError: InOrbitError {
    public var kind: ErrorKind { .decode }
    public let message: String
    public internal(set) var requestId: String?
    public internal(set) var idempotencyKey: String?
}

/// A stream event was larger than 1 MiB.
public struct TooLargeError: InOrbitError {
    public var kind: ErrorKind { .tooLarge }
    public let message: String
    public internal(set) var requestId: String?
    public internal(set) var idempotencyKey: String?
}
