// @@ID@@: the demo project's private macOS service.
//
// Every minute it writes $STATE/heartbeat.json (the health InOrbit reads) and asks iohr's
// token channel for a token with the one scope its manifest declares. It also serves a tiny
// panel on 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads /api/heartbeat.
//
// Run it with `dart run` under `iohr ext dev`'s STATE and CACHE, or once with IOHR_EXT_ONCE=1
// as the self-test does. Dart core libraries only.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'channel.dart';

/// The extension id, from the manifest.
const id = '@@ID@@';

/// The program's version.
const version = '0.1.0';

/// The one scope the manifest declares.
const scopes = ['identity:read'];

void _writeJson(String state, String name, Object value) {
  final tmp = File('$state/.$name.tmp')..writeAsStringSync(jsonEncode(value));
  // Windows has no chmod; the user's profile guards the file there.
  if (!Platform.isWindows) {
    Process.runSync('/bin/chmod', ['600', tmp.path]);
  }
  tmp.renameSync('$state/$name');
}

String _now() => '${DateTime.now().toUtc().toIso8601String().split('.').first}Z';

/// One heartbeat: asks for a token (kept in memory only, never printed), writes
/// $STATE/heartbeat.json and returns what it wrote.
Future<Map<String, Object?>> heartbeat(Map<String, String> env, String state, int beats) async {
  var tok = 'granted';
  try {
    await token(scopes, env);
  } on Refused catch (e) {
    stderr.writeln('token refused: $e');
    tok = 'refused';
  }
  final now = _now();
  final beat = <String, Object?>{
    'id': id,
    'version': version,
    'state': 'ok',
    'at': now,
    'beats': beats,
    'token': tok,
    'dev': env['IOHR_EXT_DEV'] == '1',
  };
  _writeJson(state, 'heartbeat.json', beat);
  stdout.writeln('heartbeat $beats at $now (token $tok)');
  return beat;
}

/// Where the panel page is: PANEL_FILE, or panel/index.html beside the program, or in the
/// folder it runs from.
File _panelFile(Map<String, String> env) {
  final named = env['PANEL_FILE'];
  if (named != null && named.isNotEmpty) {
    return File(named);
  }
  final beside = File('${File(Platform.resolvedExecutable).parent.path}/panel/index.html');
  return beside.existsSync() ? beside : File('panel/index.html');
}

/// Listens on 127.0.0.1 only; the port is PANEL_PORT or one the system picks, and it is
/// written to $STATE/panel.json so the app knows where the panel is.
Future<HttpServer> serve(
  Map<String, String> env,
  String state,
  Map<String, Object?> Function() latest,
) async {
  final raw = env['PANEL_PORT'];
  final port = raw == null || raw.isEmpty ? 0 : int.tryParse(raw);
  if (port == null) {
    throw FormatException('PANEL_PORT is not a number: $raw');
  }
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, port);
  server.listen((request) async {
    final response = request.response;
    if (request.method == 'GET' && request.uri.path == '/api/heartbeat') {
      response.headers.contentType = ContentType.json;
      response.write(jsonEncode(latest()));
    } else if (request.method == 'GET' && request.uri.path == '/') {
      response.headers.contentType = ContentType.html;
      final page = _panelFile(env);
      response.add(page.existsSync() ? page.readAsBytesSync() : utf8.encode('<p>panel/index.html is missing</p>'));
    } else {
      response.statusCode = HttpStatus.notFound;
    }
    await response.close();
  });
  final url = 'http://127.0.0.1:${server.port}/';
  _writeJson(state, 'panel.json', {'id': id, 'url': url});
  stdout.writeln('panel at $url');
  return server;
}

/// Runs the service until SIGINT or SIGTERM (or once, with IOHR_EXT_ONCE) and returns the exit code.
Future<int> run(Map<String, String> env) async {
  final state = env['STATE'] ?? '';
  if (state.isEmpty || (env['CACHE'] ?? '').isEmpty) {
    stderr.writeln('STATE and CACHE are not set: run this with iohr ext dev');
    return 2;
  }
  final parsed = int.tryParse(env['HEARTBEAT_EVERY'] ?? '');
  final every = parsed != null && parsed > 0 ? parsed : 60;
  final once = (env['IOHR_EXT_ONCE'] ?? '').isNotEmpty;
  stdout.writeln('$id $version starting; state in $state');

  var latest = <String, Object?>{};
  HttpServer? server;
  final stop = Completer<void>();
  final subs = <StreamSubscription<ProcessSignal>>[];
  try {
    if (!once) {
      server = await serve(env, state, () => latest);
      for (final signal in [ProcessSignal.sigterm, ProcessSignal.sigint]) {
        subs.add(signal.watch().listen((_) {
          if (!stop.isCompleted) {
            stop.complete();
          }
        }));
      }
    }
    for (var beats = 1;; beats++) {
      latest = await heartbeat(env, state, beats);
      if (once) {
        return 0;
      }
      await Future.any<void>([stop.future, Future<void>.delayed(Duration(seconds: every))]);
      if (stop.isCompleted) {
        stdout.writeln('stopping');
        return 0;
      }
    }
  } on Object catch (e) {
    stderr.writeln(e);
    return 1;
  } finally {
    for (final sub in subs) {
      await sub.cancel();
    }
    await server?.close(force: true);
  }
}
