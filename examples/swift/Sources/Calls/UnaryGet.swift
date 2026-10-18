// One unary GET: who the API thinks you are. The answer carries the typed value and the raw
// response (request id, attempts). Reads INORBIT_TOKEN, or INORBIT_KEY_ID, INORBIT_KEY_SECRET
// and INORBIT_SCOPES (identity:read).
import InOrbit

func unaryGet() async throws {
    let api = try Client<Public>.fromEnv()
    let answer = try await api.me()
    print("subject \(answer.value.subject), scopes \(answer.value.scopes.joined(separator: " ")), request \(answer.raw.requestId)")
}
