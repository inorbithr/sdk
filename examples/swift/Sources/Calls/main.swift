// One program, one example per name: `swift run Calls unary-get`. Each file beside this one
// is a single example and says what it shows in its header comment. The names are the call
// types listed in examples/README.md. The SDK has no WebSocket stream transport, so there
// is no stream-socket example here.
import Foundation

let examples: [String: () async throws -> Void] = [
    "unary-get": unaryGet,
    "list-pagination": listPagination,
    "stream-sse": streamSSE,
    "typed-error": typedError,
    "auth-token": authToken,
    "auth-custom-token": authCustomToken,
    "retries": retries,
    "concurrent-calls": concurrentCalls,
    "config-load": configLoad,
]

guard CommandLine.arguments.count > 1, let example = examples[CommandLine.arguments[1]] else {
    FileHandle.standardError.write(Data("usage: Calls <\(examples.keys.sorted().joined(separator: "|"))>\n".utf8))
    exit(2)
}
try await example()
