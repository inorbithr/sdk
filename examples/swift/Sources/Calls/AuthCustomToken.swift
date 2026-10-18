// Auth with a token provider of your own, here a stand-in for a secret store or a PKCE
// sign-in. The SDK asks it before every attempt; refresh is true once the API refused the
// last token. Never log the token.
import Foundation
import InOrbit

struct Vault: TokenProvider {
    func token(refresh: Bool) async throws -> Secret {
        Secret("custom-token-1")
    }
}

func authCustomToken() async throws {
    let env = ProcessInfo.processInfo.environment
    let client = try Client<Public>(
        ClientOptions(
            tokenProvider: Vault(),
            baseURL: env["INORBIT_BASE_URL"].flatMap { URL(string: $0) }))
    let me = try await client.me().value
    print("\(me.subject) signed in with a custom provider")
}
