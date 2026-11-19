package app;

import io.modelcontextprotocol.json.McpJsonDefaults;
import io.modelcontextprotocol.server.transport.StdioServerTransportProvider;

/**
 * Runs the server over stdio, the way an MCP host starts it. Nothing is printed to stdout but
 * protocol messages, and there is no port and no credential.
 */
public final class Main {
    private Main() {}

    public static void main(String[] args) throws InterruptedException {
        var transport = new StdioServerTransportProvider(McpJsonDefaults.getMapper());
        Server.build(transport);
        // The host ends the process when it is done with it.
        Thread.currentThread().join();
    }
}
