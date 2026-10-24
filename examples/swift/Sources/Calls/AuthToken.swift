// Auth with a token you already have: an API token from the console or `iohr token create`.
// The SDK sends it as a Bearer on every call and never exchanges it.
import Foundation
import InOrbit

func authToken() async throws {
    let env = ProcessInfo.processInfo.environment
    let client = try Client<Public>(
        ClientOptions(
            token: Secret(env["INORBIT_TOKEN"] ?? ""),
            baseURL: env["INORBIT_BASE_URL"].flatMap { URL(string: $0) }))
    let me = try await client.me().value
    print("\(me.subject) signed in with a token")
}
