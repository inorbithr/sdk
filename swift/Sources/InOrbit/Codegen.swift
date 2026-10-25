import Foundation

/// What generated surfaces call: the runtime's contract with `iohr sdk generate` (ADR 0011).
/// Application code does not need it.
public enum Codegen {
    /// The contract version. A surface names it, so one generated for another version does
    /// not compile: run `iohr sdk generate` again.
    public enum Version1 {}

    /// One call as a surface describes it to the runtime's request path.
    public struct Operation: Sendable {
        /// What hooks and errors name it: `accounts.get_me`, or `me`.
        public var name: String
        /// The method.
        public var method: HTTPMethod
        /// The path with its parameters filled in and percent-encoded.
        public var path: String
        /// The path template, as the document has it.
        public var template: String
        /// The scopes the operation needs.
        public var scopes: [String]
        /// Marked idempotent although its method is not: retried like a `GET`.
        public var idempotent = false
        /// Takes an `Idempotency-Key`: one is sent with every call, the same on every
        /// attempt, and the call is retried (docs/config.md section 7.5).
        public var takesIdempotencyKey = false
        /// The query parameters by wire name, each with its values (none when unset).
        public var query: [(String, [String])] = []
        /// The JSON body.
        public var body: Data?
        /// The RPC a `/v1/ws` call frame names, for a stream.
        public var rpc = ""

        public init(name: String, method: HTTPMethod, path: String, template: String, scopes: [String]) {
            self.name = name
            self.method = method
            self.path = path
            self.template = template
            self.scopes = scopes
        }
    }

    /// `value` percent-encoded as one path segment: a `/` or a space never changes the route.
    public static func pathSegment(_ value: String) -> String {
        percentEncode(value)
    }

    /// `value` for an `application/x-www-form-urlencoded` body or a query string.
    static func formEncode(_ value: String) -> String {
        percentEncode(value)
    }

    private static let unreserved = CharacterSet(
        charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")

    private static func percentEncode(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: unreserved) ?? value
    }

    /// A query parameter's values: none when it is unset.
    public static func queryValues(_ value: String?) -> [String] { value.map { [$0] } ?? [] }
    public static func queryValues(_ value: Int32?) -> [String] { value.map { [String($0)] } ?? [] }
    public static func queryValues(_ value: Int64?) -> [String] { value.map { [String($0)] } ?? [] }
    public static func queryValues(_ value: Double?) -> [String] { value.map { [String($0)] } ?? [] }
    public static func queryValues(_ value: Bool?) -> [String] { value.map { [$0 ? "true" : "false"] } ?? [] }
    public static func queryValues(_ value: [String]?) -> [String] { value ?? [] }
    public static func queryValues(_ value: [Int64]?) -> [String] { value?.map { String($0) } ?? [] }
    public static func queryValues(_ value: [Int32]?) -> [String] { value?.map { String($0) } ?? [] }

    /// `value` as a JSON body; what is unset is left out.
    public static func json<T: Encodable>(_ value: T) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return try encoder.encode(value)
    }

    /// Every item of a paged list (design.md section 9): `fetch` answers one page's items and
    /// the next page's token for a token (`nil` for the first page). The walk follows the
    /// token until it is empty or names the page just read, stops at the first error, and
    /// fetches nothing more once the loop stops.
    public static func pages<T: Sendable>(
        _ fetch: @escaping @Sendable (String?) async throws -> ([T], String)
    ) -> AsyncThrowingStream<T, any Error> {
        let walk = PageWalk(fetch)
        return AsyncThrowingStream(unfolding: { try await walk.next() })
    }
}

/// The state of one walk over pages.
private actor PageWalk<T: Sendable> {
    private let fetch: @Sendable (String?) async throws -> ([T], String)
    private var buffer: [T] = []
    private var token: String?
    private var started = false
    private var done = false

    init(_ fetch: @escaping @Sendable (String?) async throws -> ([T], String)) {
        self.fetch = fetch
    }

    func next() async throws -> T? {
        while buffer.isEmpty {
            if done { return nil }
            try Task.checkCancellation()
            let asked = token
            let (items, next) = try await fetch(started ? asked : nil)
            started = true
            buffer = items
            if next.isEmpty || next == asked {
                done = true
            } else {
                token = next
            }
        }
        return buffer.removeFirst()
    }
}
