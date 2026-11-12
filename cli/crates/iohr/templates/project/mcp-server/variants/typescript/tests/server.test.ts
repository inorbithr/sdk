// The passes-on-init test: the server is started the way a host starts it (a child process over
// stdio) and spoken to by the official client. It checks the 2026-07-28 revision (no handshake:
// `server/discover`), that a client of the older revision still gets its handshake, the tool
// list and its order, a tool call and its structured result, a refused bad argument, and the
// rules every tool must meet.
import assert from 'node:assert/strict';
import test from 'node:test';
import { Client } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';

async function connect(mode: 'legacy' | { pin: string }): Promise<Client> {
  const client = new Client({ name: 'test', version: '0.0.0' }, { versionNegotiation: { mode } });
  await client.connect(new StdioClientTransport({ command: process.execPath, args: ['src/main.ts'] }));
  return client;
}

async function using(mode: 'legacy' | { pin: string }, body: (c: Client) => Promise<void>): Promise<void> {
  const client = await connect(mode);
  try {
    await body(client);
  } finally {
    await client.close();
  }
}

const modern = { pin: '2026-07-28' };

test('discover answers on the 2026-07-28 revision', () =>
  using(modern, async c => {
    assert.equal(c.getNegotiatedProtocolVersion(), '2026-07-28');
    assert.equal(c.getServerVersion()?.name, '@@NAME@@');
  }));

test('a client of the older revision still gets its handshake', () =>
  // `initialize` was removed in 2026-07-28; the SDK still answers it for 2025-11-25 hosts.
  using('legacy', async c => {
    assert.equal(c.getNegotiatedProtocolVersion(), '2025-11-25');
    assert.equal(c.getServerVersion()?.name, '@@NAME@@');
  }));

test('tools/list names the tool, comes in the same order twice and says how long to keep it', () =>
  using(modern, async c => {
    const first = await c.listTools();
    const second = await c.listTools();
    assert.deepEqual(first.tools.map(t => t.name), ['word_count']);
    assert.deepEqual(first.tools.map(t => t.name), second.tools.map(t => t.name));
    // Lists say how long a client may keep them (2026-07-28).
    assert.ok('ttlMs' in first && 'cacheScope' in first);
  }));

test('a tool call returns the structured result', () =>
  using(modern, async c => {
    const result = await c.callTool({ name: 'word_count', arguments: { text: 'one two  three' } });
    assert.notEqual(result.isError, true);
    assert.deepEqual(result.structuredContent, { words: 3, characters: 14 });
  }));

test('a bad argument is refused and the tool does not run', () =>
  using(modern, async c => {
    const tooLong = await c.callTool({ name: 'word_count', arguments: { text: 'x'.repeat(10_001) } });
    assert.equal(tooLong.isError, true);
    const missing = await c.callTool({ name: 'word_count', arguments: {} });
    assert.equal(missing.isError, true);
  }));

test('the resource is listed and can be read', () =>
  using(modern, async c => {
    const listed = await c.listResources();
    assert.deepEqual(listed.resources.map(r => r.uri), ['about://server']);
    const read = await c.readResource({ uri: 'about://server' });
    assert.match(String((read.contents[0] as { text?: string }).text), /counts words/);
  }));

test('every tool has a description, an annotation and bounded inputs', () =>
  using(modern, async c => {
    for (const tool of (await c.listTools()).tools) {
      assert.ok(tool.description, `${tool.name}: no description`);
      const a = tool.annotations;
      assert.ok(a && a.readOnlyHint !== undefined, `${tool.name}: say whether it only reads (readOnlyHint)`);
      if (!a.readOnlyHint) assert.ok(a.destructiveHint !== undefined, `${tool.name}: say if it destroys (destructiveHint)`);
      const props = (tool.inputSchema.properties ?? {}) as Record<string, Record<string, unknown>>;
      for (const [name, schema] of Object.entries(props)) {
        if (schema.type === 'string') {
          assert.ok(['maxLength', 'enum', 'pattern'].some(k => k in schema), `${tool.name}.${name}: bound the string (maxLength, enum or pattern)`);
        }
        if (schema.type === 'integer' || schema.type === 'number') {
          assert.ok(['minimum', 'maximum'].some(k => k in schema), `${tool.name}.${name}: bound the number (minimum, maximum)`);
        }
      }
    }
  }));
