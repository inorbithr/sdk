import Foundation

/// Where a client's access tokens come from (design.md section 3): the extension point for
/// an app that already holds a token, from a PKCE sign-in or from elsewhere.
public protocol TokenProvider: Sendable {
    /// A valid access token. `refresh` is `true` once the API has refused the last one, and
    /// asks for a new one when the provider can make one.
    func token(refresh: Bool) async throws -> Secret
}

/// A token used as it is: it cannot be refreshed, so a refused one fails the call.
public struct StaticTokenProvider: TokenProvider {
    private let value: Secret

    /// A provider that always answers `token`.
    public init(_ token: Secret) {
        self.value = token
    }

    public func token(refresh: Bool) async throws -> Secret {
        value
    }
}

/// An API key exchanged for 15-minute access tokens with the client credentials grant:
/// cached, refreshed when less than 20 % of its life remains or after a 401, and one
/// exchange at a time however many calls wait for it (single flight).
public actor ClientCredentialsProvider: TokenProvider {
    private let keyId: String
    private let keySecret: Secret
    private let scopes: [String]
    private let tokenURL: URL
    private let transport: any HTTPTransport
    private let userAgent: String
    private let timeout: TimeInterval
    private let maxRetries: Int

    private var cached: (token: Secret, refreshAt: Date)?
    private var inFlight: Task<Exchanged, any Error>?

    init(
        keyId: String, keySecret: Secret, scopes: [String], tokenURL: URL, transport: any HTTPTransport,
        userAgent: String, timeout: TimeInterval, maxRetries: Int
    ) {
        self.keyId = keyId
        self.keySecret = keySecret
        self.scopes = scopes
        self.tokenURL = tokenURL
        self.transport = transport
        self.userAgent = userAgent
        self.timeout = timeout
        self.maxRetries = maxRetries
    }

    public func token(refresh: Bool) async throws -> Secret {
        if !refresh, let cached, Date() < cached.refreshAt {
            return cached.token
        }
        if let inFlight {
            return try await inFlight.value.token
        }
        let task = Task { try await self.exchange() }
        inFlight = task
        defer { inFlight = nil }
        let exchanged = try await task.value
        cached = (exchanged.token, Date().addingTimeInterval(exchanged.expiresIn * 0.8))
        return exchanged.token
    }

    private func exchange() async throws -> Exchanged {
        let basic = Data("\(keyId):\(keySecret.reveal())".utf8).base64EncodedString()
        let form = [
            ("grant_type", "client_credentials"),
            ("audience", "iohr-api"),
            ("scope", scopes.joined(separator: " ")),
        ]
        .map { "\($0.0)=\(Codegen.formEncode($0.1))" }
        .joined(separator: "&")
        let request = HTTPRequest(
            method: .post, url: tokenURL,
            headers: [
                "authorization": "Basic \(basic)",
                "content-type": "application/x-www-form-urlencoded",
                "accept": "application/json",
                "user-agent": userAgent,
            ],
            body: Data(form.utf8), timeout: timeout)
        var retry = 0
        while true {
            let response: HTTPResponse
            do {
                response = try await withTimeout(timeout) { [transport] in try await transport.send(request) }
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                if retry < maxRetries {
                    try await sleep(RetryPolicy.delay(retry: retry, retryAfter: nil))
                    retry += 1
                    continue
                }
                let what =
                    (error as? TransportFailure).map { f -> String in
                        if case .timeout = f { return "timed out" }
                        return "could not connect"
                    } ?? "could not connect"
                throw AuthError(
                    status: 0, error: "",
                    message: "the token exchange for key \(keyId) \(what); check the network and the token URL")
            }
            let status = response.head.status
            if (200..<300).contains(status) {
                guard let body = try? JSONDecoder().decode(TokenAnswer.self, from: response.body),
                    !body.access_token.isEmpty
                else {
                    throw AuthError(
                        status: status, error: "",
                        message: "the token endpoint answered key \(keyId) without an access token")
                }
                return Exchanged(token: Secret(body.access_token), expiresIn: TimeInterval(body.expires_in ?? 900))
            }
            if RetryPolicy.retryable(status: status), retry < maxRetries {
                let after = response.head.headers["retry-after"].flatMap(RetryPolicy.parseRetryAfter)
                try await sleep(RetryPolicy.delay(retry: retry, retryAfter: after))
                retry += 1
                continue
            }
            let problem = try? JSONDecoder().decode(TokenProblem.self, from: response.body)
            let code = problem?.error ?? "HTTP \(status)"
            var message = "the token exchange for key \(keyId) was refused: \(code) (HTTP \(status))"
            if code == "invalid_scope" {
                message += "; the key does not hold one of the scopes asked for: \(scopes.joined(separator: " "))"
            } else if code == "invalid_client" {
                message += "; check the key id and secret, and that the key is not revoked"
            }
            throw AuthError(status: status, error: problem?.error ?? "", message: message)
        }
    }

    private struct TokenAnswer: Decodable {
        let access_token: String
        let expires_in: Int?
    }

    private struct TokenProblem: Decodable {
        let error: String?
    }
}

/// A token and its life, as the exchange answers them.
struct Exchanged: Sendable {
    let token: Secret
    let expiresIn: TimeInterval
}
