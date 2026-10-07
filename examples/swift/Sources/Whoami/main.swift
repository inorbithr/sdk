// Who the API thinks you are (`GET /v1/me`). Reads INORBIT_TOKEN, or INORBIT_KEY_ID,
// INORBIT_KEY_SECRET and INORBIT_SCOPES (identity:read).
import InOrbit

let api = try Client<Public>.fromEnv()
do {
    let me = try await api.me().value
    print("\(me.subject) (\(me.kind)), scopes: \(me.scopes.joined(separator: " "))")
} catch let e as APIError {
    print("\(e.code): \(e.problem) (request id \(e.raw.requestId))")
}
