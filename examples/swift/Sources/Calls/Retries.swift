// The SDK retries for you. The API answers 429 with Retry-After; the SDK waits that long and
// tries again, and the answer says how many attempts it took. Tune it with maxRetries.
import InOrbit

func retries() async throws {
    let api = try Client<Public>.fromEnv(ClientOptions(maxRetries: 3))
    let answer = try await api.me()
    print("\(answer.value.subject) after \(answer.raw.attempts) attempts")
}
