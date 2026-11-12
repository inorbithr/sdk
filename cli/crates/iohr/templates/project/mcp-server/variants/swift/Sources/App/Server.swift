// The server: one tool and one resource. Replace them with yours; keep the rules in the test.
//
// Every tool needs a description, an annotation that says whether it only reads or can change
// things, and inputs with a bound (a maximum length, a range). ServerTests fails when one is
// missing, so a tool a host will let a model call is never left unlabelled. The SDK does not
// check arguments against the schema, so the handler does.
import MCP

let maxText = 10_000

func makeServer() async -> Server {
    let server = Server(
        name: "@@NAME@@",
        version: "0.1.0",
        instructions: "Counts the words and characters in a text. It reads nothing else and changes nothing.",
        capabilities: .init(resources: .init(subscribe: false, listChanged: false), tools: .init(listChanged: false))
    )

    let wordCount = Tool(
        name: "word_count",
        description: "Count the words and the characters in a text (at most 10,000 characters).",
        inputSchema: .object([
            "type": "object",
            "properties": .object([
                "text": .object([
                    "type": "string",
                    "description": "The text to count.",
                    "maxLength": .int(maxText),
                ])
            ]),
            "required": .array(["text"]),
            "additionalProperties": false,
        ]),
        annotations: .init(
            readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false)
    )

    await server.withMethodHandler(ListTools.self) { _ in .init(tools: [wordCount]) }

    await server.withMethodHandler(CallTool.self) { params in
        guard params.name == "word_count" else {
            return .init(content: [.text(text: "unknown tool", annotations: nil, _meta: nil)], isError: true)
        }
        // missing, not a string, too long, or an argument that is not in the schema: refused.
        guard let arguments = params.arguments, arguments.count == 1,
            let text = arguments["text"]?.stringValue, text.count <= maxText
        else {
            return .init(content: [.text(text: "text is required, a string of at most \(maxText) characters, and nothing else is accepted", annotations: nil, _meta: nil)], isError: true)
        }
        let words = text.split(whereSeparator: { $0.isWhitespace }).count
        return .init(
            content: [.text(text: "\(words) words, \(text.count) characters", annotations: nil, _meta: nil)],
            structuredContent: .object(["words": .int(words), "characters": .int(text.count)]),
            isError: false)
    }

    await server.withMethodHandler(ListResources.self) { _ in
        .init(resources: [
            Resource(name: "about", uri: "about://server", description: "What this server is and what it can do.", mimeType: "text/plain")
        ])
    }

    await server.withMethodHandler(ReadResource.self) { params in
        guard params.uri == "about://server" else {
            throw MCPError.invalidParams("Unknown resource URI: \(params.uri)")
        }
        return .init(contents: [
            .text("@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process.", uri: params.uri, mimeType: "text/plain")
        ])
    }
    return server
}
