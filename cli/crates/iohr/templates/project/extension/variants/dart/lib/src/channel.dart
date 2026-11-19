// Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).
//
// iohr passes IOHR_EXT_TOKEN_SOCKET (a 0600 Unix socket). The program sends
// `POST /token {"scopes": [...]}` and gets `{"access_token", "expires_at"}` for scopes its
// manifest declares, and a refusal for any other. It is the only credential this project
// has: never read IOHR_TOKEN or run `iohr auth token`.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

const _maxAnswer = 64 * 1024;
const _wait = Duration(seconds: 10);

/// What [token] throws when iohr refuses, or its channel cannot be reached.
class Refused implements Exception {
  Refused(this.message);

  final String message;

  @override
  String toString() => message;
}

/// Returns an access token for [scopes]; the [Refused] message says iohr's reason.
Future<String> token(List<String> scopes, Map<String, String> env) async {
  final path = (env['IOHR_EXT_TOKEN_SOCKET'] ?? '').trim();
  if (path.isEmpty) {
    throw Refused('IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev');
  }
  final raw = <int>[];
  try {
    final socket = await Socket.connect(
      InternetAddress(path, type: InternetAddressType.unix),
      0,
      timeout: _wait,
    );
    final body = utf8.encode(jsonEncode({'scopes': scopes}));
    socket
      ..add(utf8.encode(
        'POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\n'
        'content-length: ${body.length}\r\nconnection: close\r\n\r\n',
      ))
      ..add(body);
    await socket.flush();
    await for (final chunk in socket.timeout(_wait)) {
      raw.addAll(chunk);
      if (raw.length >= _maxAnswer) {
        break;
      }
    }
    socket.destroy();
  } on Object catch (e) {
    throw Refused("cannot reach iohr's token channel: $e");
  }
  final text = utf8.decode(raw.take(_maxAnswer).toList(), allowMalformed: true);
  final cut = text.indexOf('\r\n\r\n');
  final head = cut < 0 ? text : text.substring(0, cut);
  final payload = cut < 0 ? '' : text.substring(cut + 4);
  final fields = head.split(' ');
  var status = fields.length > 1 ? fields[1] : '';
  var answer = <String, Object?>{};
  try {
    final parsed = jsonDecode(payload);
    if (parsed is Map<String, Object?>) {
      answer = parsed;
    }
  } on FormatException {
    // Not JSON: the status below says what happened.
  }
  if (status != '200') {
    if (status.isEmpty) {
      status = 'no answer';
    }
    throw Refused('$status ${answer['error'] ?? ''}: ${answer['message'] ?? ''}');
  }
  final access = answer['access_token'];
  if (access is! String || access.isEmpty) {
    throw Refused("iohr's answer holds no token");
  }
  return access;
}
