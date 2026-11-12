// The self-test: `dart run test/selftest.dart` runs one heartbeat with a STATE and CACHE of its
// own (or the folders `iohr ext dev --test` and InOrbit Desktop's smoke test give), then checks
// what it wrote and where. No package, so there is nothing to download.

import 'dart:convert';
import 'dart:io';

import 'package:@@MODULE@@/service.dart';

var _failed = 0;

void _check(bool ok, String what) {
  if (!ok) {
    _failed++;
    stdout.writeln('FAIL $what');
  } else {
    stdout.writeln('ok   $what');
  }
}

Map<String, String> _env(Directory state, Directory cache, [Map<String, String> more = const {}]) => {
      'STATE': state.path,
      'CACHE': cache.path,
      'IOHR_EXT_TOKEN_SOCKET': '',
      ...more,
    };

Future<void> main() async {
  final state = Directory.systemTemp.createTempSync('state-');
  final cache = Directory.systemTemp.createTempSync('cache-');
  final here = Directory.current.listSync().map((e) => e.path).toSet();

  final code = await run(_env(state, cache, {'IOHR_EXT_ONCE': '1'}));
  _check(code == 0, 'one heartbeat exits 0');
  final beat = jsonDecode(File('${state.path}/heartbeat.json').readAsStringSync()) as Map<String, Object?>;
  _check(beat['id'] == '@@ID@@' && beat['state'] == 'ok' && beat['beats'] == 1, 'the heartbeat says who and ok');
  // Granted when you are signed in and the channel is up, refused when not: either way it answered.
  _check(beat['token'] == 'granted' || beat['token'] == 'refused', 'the token ask was answered');
  _check(Directory.current.listSync().map((e) => e.path).toSet().difference(here).isEmpty,
      'nothing was written into the project folder');

  Object? refusal;
  try {
    await token(['tokens:write'], {'IOHR_EXT_TOKEN_SOCKET': ''});
  } on Refused catch (e) {
    refusal = e;
  }
  _check(refusal != null, 'without the channel the token ask is refused');

  // A fake channel: the one scope is granted, any other is refused.
  final socket = '${Directory.systemTemp.createTempSync('chan-').path}/t.sock';
  final channel = await ServerSocket.bind(InternetAddress(socket, type: InternetAddressType.unix), 0);
  channel.listen((client) async {
    final request = utf8.decode(await client.first);
    final ok = request.contains('identity:read');
    final body = ok ? '{"access_token":"t","expires_at":"x"}' : '{"error":"scope","message":"not declared"}';
    client.write('HTTP/1.1 ${ok ? '200 OK' : '403 Forbidden'}\r\ncontent-length: ${body.length}\r\n\r\n$body');
    await client.close();
  });
  final env = {'IOHR_EXT_TOKEN_SOCKET': socket};
  _check(await token(scopes, env) == 't', 'a declared scope is granted by the channel');
  Object? refused;
  try {
    await token(['tokens:write'], env);
  } on Refused catch (e) {
    refused = e;
  }
  _check(refused.toString().contains('403'), 'an undeclared scope is refused by the channel');
  await channel.close();

  final server = await serve(_env(state, cache, {'PANEL_FILE': 'panel/index.html'}), state.path, () => beat);
  final client = HttpClient();
  final api = await (await client.getUrl(Uri.parse('http://127.0.0.1:${server.port}/api/heartbeat'))).close();
  final apiBody = await utf8.decodeStream(api);
  _check(api.statusCode == 200 && apiBody.contains('"beats":1'), 'the panel serves the heartbeat as JSON');
  final page = await (await client.getUrl(Uri.parse('http://127.0.0.1:${server.port}/'))).close();
  _check((await utf8.decodeStream(page)).contains('/api/heartbeat'), 'the panel page reads /api/heartbeat');
  final missing = await (await client.getUrl(Uri.parse('http://127.0.0.1:${server.port}/nope'))).close();
  await missing.drain<void>();
  _check(missing.statusCode == 404, 'anything else is a 404');
  client.close(force: true);
  await server.close(force: true);

  stdout.writeln(_failed == 0 ? 'selftest passed' : '$_failed check(s) failed');
  exit(_failed == 0 ? 0 : 1);
}
