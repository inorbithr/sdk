// A server-sent-events stream: the account's events, as they happen, as an
// AsyncThrowingStream. Each event is typed; the loop ends when the server ends the stream.
// Needs events:read.
import InOrbit

func streamSSE() async throws {
    let api = try Client<Public>.fromEnv()
    var n = 0
    for try await event in api.events.streamEvents() {
        n += 1
        print(event.occurredAt, event.type, event.id)
    }
    print("\(n) events")
}
