// Concurrent use: one client shared by several tasks in flight with async let. The client is
// Sendable, and callers that need a token at the same moment share one exchange.
import InOrbit

func concurrentCalls() async throws {
    let api = try Client<Public>.fromEnv()
    async let a = api.me()
    async let b = api.me()
    async let c = api.me()
    let subjects = try await [a.value.subject, b.value.subject, c.value.subject]
    print("\(subjects.count) calls at once: \(subjects.joined(separator: ", "))")
}
