// Checks the live API's answers against the bundled contract, for every GET
// operation that needs no identifier in its path. Without a key only the public
// ones answer; with INORBIT_API_KEY set (a bearer), the account's ones do too.
// Prints the mismatches found (kinds and field paths, never values) and exits 1 when
// there is any, so it can run on a schedule.
//
// Usage: dart run tool/live_check.dart [--base https://api.inorbit.hr]

import 'dart:convert';
import 'dart:io';

import 'package:inorbit/inorbit.dart';

Future<void> main(List<String> args) async {
  var base = 'https://api.inorbit.hr';
  for (var i = 0; i < args.length; i++) {
    if (args[i] == '--base') base = args[++i];
  }
  final key = Platform.environment['INORBIT_API_KEY'] ?? '';
  final verifier = ContractVerifier.bundled();
  final client = HttpClient()..connectionTimeout = const Duration(seconds: 10);

  final found = <ContractMismatch>{};
  var checked = 0;
  var refused = 0;
  for (final op in verifier.operations) {
    if (op.method != 'GET' || op.path.contains('{')) continue;
    final req = await client.getUrl(Uri.parse('$base${op.path}'));
    req.headers.set('accept', 'application/json');
    if (key.isNotEmpty) req.headers.set('authorization', 'Bearer $key');
    final res = await req.close().timeout(const Duration(seconds: 25));
    final text = await res.transform(utf8.decoder).join();
    if (res.statusCode != 200) {
      refused++;
      stdout.writeln('skip  ${op.id} (${op.path}): HTTP ${res.statusCode}');
      continue;
    }
    checked++;
    Object? body;
    try {
      body = jsonDecode(text);
    } on FormatException {
      stdout.writeln('FAIL  ${op.id} (${op.path}): the answer is not JSON');
      continue;
    }
    final check = verifier.verify(
      method: 'GET',
      path: op.path,
      status: 200,
      body: body,
    );
    stdout.writeln(
      '${check.mismatches.isEmpty ? 'ok   ' : 'FAIL '} ${op.id} (${op.path})',
    );
    for (final m in check.mismatches) {
      stdout.writeln(
        '        ${m.field.isEmpty ? '(answer)' : m.field}: '
        'expected ${m.expected}, got ${m.got}',
      );
    }
    found.addAll(check.mismatches);
  }
  // The public Decisions documents are the one family that answers without a key and has
  // identifiers in its path: follow the first few from the list.
  final listed = await _getJson(
    client,
    '$base/v1/decisions/public/documents',
    key,
  );
  if (listed is Map) {
    final slugs = {
      for (final s in (listed['spaces'] as List? ?? const []))
        if (s is Map) s['space_id']: s['slug'],
    };
    for (final d in (listed['documents'] as List? ?? const []).take(5)) {
      if (d is! Map) continue;
      final path =
          '/v1/decisions/public/spaces/${slugs[d['space_id']]}/${d['kind']}/${d['number']}';
      final body = await _getJson(client, '$base$path', key);
      if (body == null) continue;
      checked++;
      final check = verifier.verify(
        method: 'GET',
        path: path,
        status: 200,
        body: body,
      );
      final op = check.operation?.id ?? '(not in the contract)';
      stdout.writeln(
        '${check.mismatches.isEmpty ? 'ok   ' : 'FAIL '} $op (${check.operation?.path})',
      );
      for (final m in check.mismatches) {
        stdout.writeln(
          '        ${m.field.isEmpty ? '(answer)' : m.field}: '
          'expected ${m.expected}, got ${m.got}',
        );
      }
      found.addAll(check.mismatches);
    }
  }
  client.close();
  stdout.writeln(
    '\n$checked answers checked against contract ${verifier.apiVersion}, '
    '$refused refused (no key or not allowed), ${found.length} mismatches.',
  );
  if (found.isNotEmpty) exitCode = 1;
}

/// GET [url] as JSON, or null when it does not answer 200 with JSON.
Future<Object?> _getJson(HttpClient client, String url, String key) async {
  final req = await client.getUrl(Uri.parse(url));
  req.headers.set('accept', 'application/json');
  if (key.isNotEmpty) req.headers.set('authorization', 'Bearer $key');
  final res = await req.close().timeout(const Duration(seconds: 25));
  final text = await res.transform(utf8.decoder).join();
  if (res.statusCode != 200) return null;
  try {
    return jsonDecode(text);
  } on FormatException {
    return null;
  }
}
