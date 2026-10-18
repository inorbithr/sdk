// Typed error handling. A refused call throws APIError with the HTTP status, a stable code
// to branch on, the problem text and the request id to quote to support.
import InOrbit

func typedError() async throws {
    let api = try Client<Public>.fromEnv()
    do {
        _ = try await api.me()
    } catch let e as APIError where e.code == .forbidden {
        print("refused: status \(e.status), code \(e.code), request \(e.raw.requestId)")
    }
}
