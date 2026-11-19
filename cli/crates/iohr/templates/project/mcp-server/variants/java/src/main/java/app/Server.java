package app;

import io.modelcontextprotocol.server.McpServer;
import io.modelcontextprotocol.server.McpServerFeatures;
import io.modelcontextprotocol.server.McpSyncServer;
import io.modelcontextprotocol.spec.McpSchema;
import io.modelcontextprotocol.spec.McpServerTransportProvider;
import java.util.List;
import java.util.Map;

/**
 * The server: one tool and one resource. Replace them with yours; keep the rules in the test.
 *
 * <p>Every tool needs a description, an annotation that says whether it only reads or can change
 * things, and inputs with a bound (a maximum length, a range). ServerTest fails when one is
 * missing, so a tool a host will let a model call is never left unlabelled.
 */
public final class Server {
    public static final int MAX_TEXT = 10_000;
    static final String ABOUT =
            "@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process.";

    private Server() {}

    public static McpSyncServer build(McpServerTransportProvider transport) {
        return McpServer.sync(transport)
                .serverInfo("@@NAME@@", "0.1.0")
                .instructions("Counts the words and characters in a text. It reads nothing else and changes nothing.")
                .capabilities(McpSchema.ServerCapabilities.builder()
                        .tools(false)
                        .resources(false, false)
                        .build())
                .tools(wordCount())
                .resources(about())
                .build();
    }

    static McpServerFeatures.SyncToolSpecification wordCount() {
        // The SDK checks every call against this schema before the handler runs.
        Map<String, Object> input = Map.of(
                "type", "object",
                "properties", Map.of("text", Map.of(
                        "type", "string",
                        "description", "The text to count.",
                        "maxLength", MAX_TEXT)),
                "required", List.of("text"),
                "additionalProperties", false);
        var tool = McpSchema.Tool.builder("word_count", input)
                .description("Count the words and the characters in a text (at most 10,000 characters).")
                .annotations(new McpSchema.ToolAnnotations(null, true, false, true, false, null))
                .build();
        return McpServerFeatures.SyncToolSpecification.builder()
                .tool(tool)
                .callHandler((exchange, request) -> {
                    var text = (String) request.arguments().get("text");
                    int words = text.isBlank() ? 0 : text.trim().split("\\s+").length;
                    int characters = text.codePointCount(0, text.length());
                    return McpSchema.CallToolResult.builder()
                            .addTextContent("{\"words\":" + words + ",\"characters\":" + characters + "}")
                            .structuredContent(Map.of("words", words, "characters", characters))
                            .build();
                })
                .build();
    }

    static McpServerFeatures.SyncResourceSpecification about() {
        var resource = McpSchema.Resource.builder()
                .uri("about://server")
                .name("about")
                .description("What this server is and what it can do.")
                .mimeType("text/plain")
                .build();
        return new McpServerFeatures.SyncResourceSpecification(resource, (exchange, request) ->
                new McpSchema.ReadResourceResult(
                        List.of(new McpSchema.TextResourceContents(request.uri(), "text/plain", ABOUT, null))));
    }
}
