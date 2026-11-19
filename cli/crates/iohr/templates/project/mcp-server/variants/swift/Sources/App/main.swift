// Starts the server over stdio, the way a host does. stdout belongs to the protocol.
import MCP

let server = await makeServer()
try await server.start(transport: StdioTransport())
await server.waitUntilCompleted()
