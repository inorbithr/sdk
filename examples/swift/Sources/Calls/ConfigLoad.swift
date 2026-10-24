// Configuration: options set in code, then the environment. fromEnv takes the options you
// pass as the base and fills the rest from INORBIT_* (INORBIT_TOKEN or the key variables,
// INORBIT_BASE_URL, INORBIT_TOKEN_URL). Swift has no config file or iohr login yet; a
// missing credential throws ConfigError saying which variables to set.
import InOrbit

func configLoad() async throws {
    let client: Client<Public>
    do {
        client = try Client<Public>.fromEnv(ClientOptions(timeout: 10))
    } catch let e as ConfigError {
        print(e.message)
        throw e
    }
    let me = try await client.me().value
    print("\(me.subject), a \(me.kind)")
}
