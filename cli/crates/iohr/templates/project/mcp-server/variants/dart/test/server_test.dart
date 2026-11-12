// The passes-on-init test: the server is started the way a host starts it (a child process over
// stdio) and spoken to by the SDK's client. It checks the handshake, the tool list and its
// order, a tool call, a refused bad argument, the resource, and the rules every tool must meet.
// dart_mcp 0.5.2 speaks up to 2025-11-25, so it uses `initialize`; the 2026-07-28 checks
// (server/discover, ttlMs, cacheScope) come when the SDK has them.
import 'dart:async';
import 'dart:io';

import 'package:dart_mcp/client.dart';
import 'package:dart_mcp/stdio.dart';
import 'package:test/test.dart';

late Process process;
late ServerConnection server;
late InitializeResult hello;

void main() {
  setUpAll(() async {
    process = await Process.start(Platform.resolvedExecutable, [
      'run',
      'bin/main.dart',
    ]);
    unawaited(process.stderr.drain<void>());
    final client = MCPClient(Implementation(name: 'test', version: '0'));
    server = client.connectServer(
      stdioChannel(input: process.stdout, output: process.stdin),
    );
    hello = await server.initialize(
      InitializeRequest(
        protocolVersion: ProtocolVersion.latestSupported,
        capabilities: client.capabilities,
        clientInfo: client.implementation,
      ),
    );
    server.notifyInitialized();
  });

  tearDownAll(() async {
    await server.shutdown();
    process.kill();
  });

  test('the handshake names the server', () {
    expect(hello.serverInfo.name, '@@NAME@@');
    expect(hello.protocolVersion, ProtocolVersion.latestSupported);
  });

  test('tools/list names the tool and comes in the same order twice', () async {
    final first = (await server.listTools()).tools.map((t) => t.name).toList();
    final second = (await server.listTools()).tools.map((t) => t.name).toList();
    expect(first, ['word_count']);
    expect(second, first);
  });

  test('a tool call returns the count', () async {
    final r = await server.callTool(
      CallToolRequest(name: 'word_count', arguments: {'text': 'one two  three'}),
    );
    expect(r.isError, isNot(true));
    expect((r.content.first as TextContent).text, '3 words, 14 characters');
  });

  test('a bad argument is refused and the tool does not run', () async {
    final bad = <String, Map<String, Object?>>{
      'too long': {'text': 'x' * 10001},
      'missing': {},
      'extra': {'text': 'a', 'other': 1},
      'wrong type': {'text': 1},
    };
    for (final entry in bad.entries) {
      try {
        final r = await server.callTool(
          CallToolRequest(name: 'word_count', arguments: entry.value),
        );
        expect(r.isError, true, reason: '${entry.key}: was not refused');
      } catch (_) {
        // a protocol error is a refusal too
      }
    }
  });

  test('the resource is listed and can be read', () async {
    final listed = (await server.listResources()).resources;
    expect(listed.map((r) => r.uri), ['about://server']);
    final read = await server.readResource(
      ReadResourceRequest(uri: 'about://server'),
    );
    expect((read.contents.first as TextResourceContents).text,
        contains('counts words'));
  });

  test('every tool has a description, an annotation and bounded inputs', () async {
    for (final tool in (await server.listTools()).tools) {
      expect(tool.description ?? '', isNotEmpty, reason: '${tool.name}: no description');
      final a = tool.toolAnnotations;
      expect(a?.readOnlyHint == true || a?.destructiveHint != null, isTrue,
          reason: '${tool.name}: say whether it only reads (readOnlyHint) or destroys (destructiveHint)');
      final props = tool.inputSchema.properties ?? {};
      props.forEach((name, schema) {
        final p = schema as Map<String, Object?>;
        switch (p['type']) {
          case 'string':
            expect(p['maxLength'] != null || p['enum'] != null || p['pattern'] != null, isTrue,
                reason: '${tool.name}.$name: bound the string (maxLength, enum or pattern)');
          case 'integer' || 'number':
            expect(p['minimum'] != null || p['maximum'] != null, isTrue,
                reason: '${tool.name}.$name: bound the number (minimum, maximum)');
        }
      });
    }
  });
}
