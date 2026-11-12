package app;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.modelcontextprotocol.client.McpClient;
import io.modelcontextprotocol.client.McpSyncClient;
import io.modelcontextprotocol.client.transport.ServerParameters;
import io.modelcontextprotocol.client.transport.StdioClientTransport;
import io.modelcontextprotocol.json.McpJsonDefaults;
import io.modelcontextprotocol.spec.McpSchema;
import java.nio.file.Path;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

/**
 * The passes-on-init test: the server is started the way a host starts it (a child process over
 * stdio) and spoken to by the official client. It checks the handshake, the tool list and its
 * order, a tool call and its structured result, a refused bad argument, the resource, and the
 * rules every tool must meet.
 *
 * <p>The SDK pinned here (2.1.0) speaks up to 2025-11-25, so this variant checks the initialize
 * handshake; the 2026-07-28 revision (server/discover, ttlMs and cacheScope) is in the 2.2
 * milestones and joins this test when 2.2 is released.
 */
class ServerTest {
    private McpSyncClient client;

    @BeforeEach
    void start() {
        var java = Path.of(System.getProperty("java.home"), "bin", "java").toString();
        var params = ServerParameters.builder(java)
                .args("-cp", System.getProperty("java.class.path"), "app.Main")
                .build();
        client = McpClient.sync(new StdioClientTransport(params, McpJsonDefaults.getMapper()))
                .requestTimeout(Duration.ofSeconds(60))
                .initializationTimeout(Duration.ofSeconds(60))
                .build();
        client.initialize();
    }

    @AfterEach
    void stop() {
        client.closeGracefully();
    }

    @Test
    void theHandshakeNamesTheServer() {
        assertEquals("@@NAME@@", client.getServerInfo().name());
        assertEquals("2025-11-25", client.getCurrentInitializationResult().protocolVersion());
    }

    @Test
    void toolsListNamesTheToolAndComesInTheSameOrderTwice() {
        var first = client.listTools().tools().stream().map(McpSchema.Tool::name).toList();
        var second = client.listTools().tools().stream().map(McpSchema.Tool::name).toList();
        assertEquals(List.of("word_count"), first);
        assertEquals(first, second);
    }

    @Test
    void aToolCallReturnsTheStructuredResult() {
        var result = client.callTool(new McpSchema.CallToolRequest("word_count", Map.of("text", "one two  three")));
        assertFalse(Boolean.TRUE.equals(result.isError()));
        assertEquals(Map.of("words", 3, "characters", 14), result.structuredContent());
    }

    @Test
    void aBadArgumentIsRefusedAndTheToolDoesNotRun() {
        var bad = Map.of(
                "too long", Map.<String, Object>of("text", "x".repeat(Server.MAX_TEXT + 1)),
                "missing", Map.<String, Object>of(),
                "extra", Map.<String, Object>of("text", "a", "other", 1));
        bad.forEach((what, args) -> {
            boolean refused;
            try {
                refused = Boolean.TRUE.equals(client.callTool(new McpSchema.CallToolRequest("word_count", args)).isError());
            } catch (RuntimeException e) {
                refused = true;
            }
            assertTrue(refused, what + ": was not refused");
        });
    }

    @Test
    void theResourceIsListedAndCanBeRead() {
        var listed = client.listResources().resources();
        assertEquals(1, listed.size());
        assertEquals("about://server", listed.get(0).uri());
        var read = client.readResource(new McpSchema.ReadResourceRequest("about://server"));
        var text = (McpSchema.TextResourceContents) read.contents().get(0);
        assertTrue(text.text().contains("counts words"));
    }

    @Test
    @SuppressWarnings("unchecked")
    void everyToolHasADescriptionAnAnnotationAndBoundedInputs() {
        for (var tool : client.listTools().tools()) {
            var name = tool.name();
            assertTrue(tool.description() != null && !tool.description().isEmpty(), name + ": no description");
            var a = tool.annotations();
            assertNotNull(a, name + ": say whether it only reads (readOnlyHint) or whether it destroys (destructiveHint)");
            assertTrue(Boolean.TRUE.equals(a.readOnlyHint()) || a.destructiveHint() != null, name + ": annotation says nothing");
            var props = (Map<String, Map<String, Object>>) tool.inputSchema().getOrDefault("properties", Map.of());
            props.forEach((prop, p) -> {
                var type = p.get("type");
                if ("string".equals(type)) {
                    assertTrue(p.containsKey("maxLength") || p.containsKey("enum") || p.containsKey("pattern"),
                            name + "." + prop + ": bound the string (maxLength, enum or pattern)");
                } else if ("integer".equals(type) || "number".equals(type)) {
                    assertTrue(p.containsKey("minimum") || p.containsKey("maximum"),
                            name + "." + prop + ": bound the number (minimum, maximum)");
                }
            });
        }
    }
}
