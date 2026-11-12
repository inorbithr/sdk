// The passes-on-init test, with no test package to fetch: boots the service on a free port and
// checks what a service owes. Run `dart run test/service_test.dart`; it exits 1 on the first
// failure.
import 'dart:convert';
import 'dart:io';

import 'package:@@MODULE@@/obs.dart';
import 'package:@@MODULE@@/service.dart';

void check(bool ok, String what) {
  if (!ok) {
    stderr.writeln('FAIL: $what');
    exit(1);
  }
}

Future<(int, String, HttpHeaders)> get(HttpClient c, int port, String path, [Map<String, String> headers = const {}]) async {
  final req = await c.getUrl(Uri.parse('http://127.0.0.1:$port$path'));
  headers.forEach(req.headers.set);
  final res = await req.close();
  return (res.statusCode, await utf8.decodeStream(res), res.headers);
}

Future<void> main() async {
  final lines = <String>[];
  final svc = Service(Logger(lines.add));
  await svc.start('127.0.0.1', 0);
  final client = HttpClient();
  final port = svc.port;
  try {
    // probes answer
    check((await get(client, port, '/healthz')).$1 == 200, '/healthz');
    check((await get(client, port, '/readyz')).$1 == 200, '/readyz');
    final hello = await get(client, port, '/v1/hello?name=Ada');
    check(jsonDecode(hello.$2)['message'] == 'Hello, Ada', 'hello body ${hello.$2}');
    check((await get(client, port, '/nope')).$1 == 404, 'unknown path is 404');

    // one span with the route and the trace id in the log, and nothing secret
    lines.clear();
    const trace = '4bf92f3577b34da6a3ce929d0e0e4736';
    final res = await get(client, port, '/v1/hello?name=secret-query-value', {
      'traceparent': '00-$trace-00f067aa0ba902b7-01',
      'authorization': 'Bearer secret-header-value',
    });
    check(res.$3.value('traceparent')!.startsWith('00-$trace-'), 'traceparent comes back');
    final records = lines.map((l) => jsonDecode(l) as Map<String, dynamic>).toList();
    final spans = records.where((r) => r['msg'] == 'span').toList();
    final requests = records.where((r) => r['msg'] == 'request').toList();
    check(spans.length == 1 && requests.length == 1, 'one span and one request line');
    check(spans[0]['route'] == '/v1/hello' && spans[0]['trace_id'] == trace, 'span route and trace id');
    check(spans[0]['parent_span_id'] == '00f067aa0ba902b7', 'span parent');
    check(requests[0]['trace_id'] == trace, 'request trace id');
    final all = lines.join();
    for (final secret in ['secret-query-value', 'secret-header-value']) {
      check(!all.contains(secret), 'the log holds $secret');
    }
    for (final v in Platform.environment.values) {
      check(v.length < 12 || !all.contains(v), 'the log holds an environment value');
    }

    // not ready while shutting down
    svc.ready = false;
    check((await get(client, port, '/readyz')).$1 == 503, 'not ready is 503');
    check((await get(client, port, '/healthz')).$1 == 200, 'still healthy while not ready');
    svc.ready = true;

    // an unknown path is never logged, and metrics count by route
    lines.clear();
    await get(client, port, '/private/user-1234');
    await get(client, port, '/healthz');
    check(!lines.join().contains('user-1234'), 'a raw path reached the log');
    final metrics = (await get(client, port, '/metrics')).$2;
    check(metrics.contains('route="unmatched",status="404"'), 'unmatched counter');
    check(metrics.contains('route="/healthz",status="200"'), 'healthz counter');

    // configuration has defaults and refuses a bad port
    check(loadConfig({}).port == 8080 && loadConfig({}).host == '127.0.0.1', 'defaults');
    var refused = false;
    try {
      loadConfig({'PORT': '99999'});
    } on FormatException {
      refused = true;
    }
    check(refused, 'a bad port is refused');
  } finally {
    client.close(force: true);
    await svc.stop();
  }
  stdout.writeln('ok: 4 checks passed');
}
