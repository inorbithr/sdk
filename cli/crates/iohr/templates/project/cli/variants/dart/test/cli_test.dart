// The passes-on-init test, with no test package to fetch: golden output in text and JSON
// (golden/), the exit codes, and a version line that carries a commit. Run from the project
// folder with `dart run test/cli_test.dart`; it exits 1 with the first failure.
import 'dart:io';

import 'package:@@MODULE@@/cli.dart';

const commit = '0123456789ab';

void check(bool ok, String what) {
  if (!ok) {
    stderr.writeln('FAIL: $what');
    exit(1);
  }
}

({String out, String err, int code}) call(List<String> args, {String c = commit}) {
  final o = StringBuffer();
  final e = StringBuffer();
  final code = run(args, o.write, e.write, commit: c);
  return (out: o.toString(), err: e.toString(), code: code);
}

void main() {
  for (final (file, args) in [
    ('help.txt', ['--help']),
    ('greet.txt', ['greet', 'Ada']),
    ('version.txt', ['version']),
    ('greet.json', ['--json', 'greet', 'Ada']),
    ('version.json', ['version', '--json']),
  ]) {
    final r = call(args);
    check(r.code == 0 && r.out == File('golden/$file').readAsStringSync(), '${args.join(' ')} differs from golden/$file');
  }
  for (final (code, args) in [
    (exitOk, ['greet']),
    (exitUsage, <String>[]),
    (exitUsage, ['nope']),
    (exitUsage, ['greet', 'a', 'b']),
    (exitFailure, ['greet', '   ']),
    (exitFailure, ['greet', 'x' * 65]),
  ]) {
    check(call(args).code == code, '${args.join(' ')}: want exit $code');
  }
  check(call(['nope']).out.isEmpty, 'a usage error writes to stderr, not stdout');
  check(RegExp(r'\(commit ([0-9a-f]{7,40}|unknown)\)$').hasMatch(call(['version'], c: buildCommit()).out.trim()),
      'the version line has a commit');
  final hostile = call(['\u001b[31mred${'x' * 100}']).err;
  check(!hostile.contains('\u001b') && hostile.length < 200, 'the error echoed its argument raw: $hostile');
  stdout.writeln('ok: golden text and JSON, exit codes, version commit, hostile argument');
}
