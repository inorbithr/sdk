// The routes, and the one place a request is traced, logged and counted.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'obs.dart';

/// HOST and PORT from the environment: a bad PORT throws a [FormatException].
({String host, int port}) loadConfig(Map<String, String> env) {
  final raw = (env['PORT'] ?? '').isEmpty ? '8080' : env['PORT']!;
  final port = RegExp(r'^\d+$').hasMatch(raw) ? int.parse(raw) : -1;
  if (port < 0 || port > 65535) {
    throw const FormatException('PORT must be a number from 0 to 65535');
  }
  return (host: (env['HOST'] ?? '').isEmpty ? '127.0.0.1' : env['HOST']!, port: port);
}

class _Reply {
  _Reply(this.route, this.status, this.body, [this.type = 'application/json']);
  final String route;
  final int status;
  final String body;
  final String type;
}

_Reply _json(String route, int status, Object body) => _Reply(route, status, '${jsonEncode(body)}\n');

class Service {
  Service(this.log);
  final Logger log;
  final Metrics metrics = Metrics();

  /// False until the server listens and again as soon as shutdown starts, so a load balancer
  /// stops sending traffic before the process stops.
  bool ready = false;
  HttpServer? _server;

  /// The port it listens on (useful when started on port 0).
  int get port => _server!.port;

  Future<void> start(String host, int port) async {
    final server = await HttpServer.bind(host, port);
    // Slow or idle connections are dropped: a visitor cannot hold a socket forever.
    server.idleTimeout = const Duration(seconds: 15);
    _server = server;
    server.listen(_handle);
    ready = true;
  }

  Future<void> stop() async {
    ready = false;
    await _server?.close(force: true);
  }

  _Reply _respond(Uri uri) {
    switch (uri.path) {
      case '/healthz':
        return _json('/healthz', 200, {'status': 'ok'});
      case '/readyz':
        return ready ? _json('/readyz', 200, {'status': 'ready'}) : _json('/readyz', 503, {'status': 'not ready'});
      case '/metrics':
        return _Reply('/metrics', 200, metrics.render(), 'text/plain; version=0.0.4');
      case '/v1/hello':
        // Your first route: replace it. The name comes from the query string, so it is bounded
        // and echoed back, never logged.
        final given = uri.queryParameters['name'];
        final name = (given == null || given.isEmpty) ? 'world' : given;
        if (name.runes.length > 64) {
          return _json('/v1/hello', 400, {'error': 'name is at most 64 characters of text'});
        }
        return _json('/v1/hello', 200, {'message': 'Hello, $name'});
      default:
        return _json('unmatched', 404, {'error': 'not found'});
    }
  }

  void _handle(HttpRequest req) {
    final span = Span.start(req.headers.value('traceparent'));
    final isGet = req.method == 'GET';
    final reply = isGet ? _respond(req.uri) : _json('unmatched', 405, {'error': 'method not allowed'});
    final bytes = utf8.encode(reply.body);
    req.response
      ..statusCode = reply.status
      ..headers.set('Content-Type', reply.type)
      ..headers.set('traceparent', span.header)
      ..contentLength = bytes.length
      ..add(bytes);
    req.response.close();
    final common = <String, Object>{
      'trace_id': span.traceId,
      'route': reply.route,
      'status': reply.status,
      'duration_ms': span.millis,
    };
    log.info('span', {
      'span_id': span.spanId,
      'name': '${isGet ? 'GET' : 'OTHER'} ${reply.route}',
      ...common,
      if (span.parentId != null) 'parent_span_id': span.parentId!,
    });
    log.info('request', {'method': isGet ? 'GET' : 'OTHER', ...common});
    metrics.inc(reply.route, reply.status);
  }
}
