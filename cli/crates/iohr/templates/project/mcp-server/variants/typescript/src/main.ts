// Run the server over stdio, the way an MCP host starts it: `node src/main.ts`.
// stdout carries the protocol and nothing else: never console.log here, use console.error.
import { serveStdio } from '@modelcontextprotocol/server/stdio';
import { createServer } from './server.ts';

await serveStdio(createServer);
