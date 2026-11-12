// The server: one tool and one resource. Replace them with yours; keep the rules in the test.
//
// Every tool needs a description, an annotation that says whether it only reads or can change
// things, and inputs with a bound (a maximum length, a range). `test/server_test.dart` fails
// when one is missing, so a tool a host will let a model call is never left unlabelled.
import 'dart:async';

import 'package:dart_mcp/server.dart';

base class WordServer extends MCPServer with ToolsSupport, ResourcesSupport {
  WordServer(super.channel)
    : super.fromStreamChannel(
        implementation: Implementation(name: '@@NAME@@', version: '0.1.0'),
        instructions:
            'Counts the words and characters in a text. It reads nothing else and changes nothing.',
      ) {
    // The SDK checks the arguments against the schema (a missing, extra, wrongly typed or too
    // long one is refused with isError) before this runs.
    registerTool(wordCount, _wordCount);
    addResource(
      Resource(
        uri: 'about://server',
        name: 'About this server',
        description: 'What this server is and what it can do.',
        mimeType: 'text/plain',
      ),
      (request) => ReadResourceResult(
        contents: [
          TextResourceContents(
            uri: request.uri,
            text:
                '@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process.',
          ),
        ],
      ),
    );
  }

  static final wordCount = Tool(
    name: 'word_count',
    description:
        'Count the words and the characters in a text (at most 10,000 characters).',
    inputSchema: Schema.object(
      properties: {
        'text': Schema.string(
          description: 'The text to count.',
          maxLength: 10000,
        ),
      },
      required: ['text'],
      additionalProperties: false,
    ),
    annotations: ToolAnnotations(
      readOnlyHint: true,
      destructiveHint: false,
      idempotentHint: true,
      openWorldHint: false,
    ),
  );

  FutureOr<CallToolResult> _wordCount(CallToolRequest request) {
    final text = request.arguments!['text'] as String;
    final words = text.split(RegExp(r'\s+')).where((w) => w.isNotEmpty).length;
    return CallToolResult(
      content: [TextContent(text: '$words words, ${text.length} characters')],
    );
  }
}
