// The server: one tool and one resource. Replace them with yours; keep the rules in the test.
//
// Every tool needs a description, an annotation that says whether it only reads or can change
// things, and inputs with a bound (a maximum length, a range). `tests/server.test.ts` fails when
// one is missing, so a tool a host will let a model call is never left unlabelled.
import { McpServer } from '@modelcontextprotocol/server';
import * as z from 'zod/v4';

export function createServer(): McpServer {
  const server = new McpServer(
    { name: '@@NAME@@', version: '0.1.0' },
    { instructions: 'Counts the words and characters in a text. It reads nothing else and changes nothing.' }
  );

  server.registerTool(
    'word_count',
    {
      title: 'Word count',
      description: 'Count the words and the characters in a text (at most 10,000 characters).',
      inputSchema: z.object({ text: z.string().max(10_000).describe('The text to count.') }),
      // The structured result: a host gets it as `structuredContent`, checked against this schema.
      outputSchema: z.object({ words: z.number().int().nonnegative(), characters: z.number().int().nonnegative() }),
      annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false }
    },
    async ({ text }) => {
      const output = { words: text.split(/\s+/).filter(Boolean).length, characters: text.length };
      return { content: [{ type: 'text', text: JSON.stringify(output) }], structuredContent: output };
    }
  );

  server.registerResource(
    'about',
    'about://server',
    { description: 'What this server is and what it can do.', mimeType: 'text/plain' },
    async uri => ({
      contents: [
        {
          uri: uri.href,
          text: '@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process.'
        }
      ]
    })
  );

  return server;
}
