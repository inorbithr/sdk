// @@NAME@@: a command line with text and JSON output, exit codes that mean something, and a
// version that says which commit it was built from. The Dart SDK only.
import 'dart:convert';
import 'dart:io';

const version = '0.1.0';

/// Exit codes: the contract scripts rely on.
const exitOk = 0;
const exitFailure = 1;
const exitUsage = 2;

const usage = '''Usage: @@NAME@@ [--json] <command>

Commands:
  greet [NAME]   Say hello to NAME (default: world)
  version        Print the version and the commit it was built from

Options:
      --json     Machine-readable output
  -h, --help     Show this help

Exit codes: 0 success, 1 failure, 2 usage error.
''';

final _commitPattern = RegExp(r'^[0-9a-f]{7,40}$');

/// The commit this runs from: `COMMIT` in the environment (a release build sets it with
/// `dart compile exe -DCOMMIT=...`), else the checkout's git head, else "unknown".
String buildCommit() {
  const stamped = String.fromEnvironment('COMMIT');
  if (_commitPattern.hasMatch(stamped)) return stamped.substring(0, stamped.length > 12 ? 12 : stamped.length);
  try {
    final r = Process.runSync('git', ['rev-parse', '--short=12', 'HEAD'],
        workingDirectory: File(Platform.script.toFilePath()).parent.path);
    final out = (r.stdout as String).trim();
    if (r.exitCode == 0 && _commitPattern.hasMatch(out)) return out;
  } on Object {
    // no git, or not a checkout
  }
  return 'unknown';
}

/// Keeps a hostile argument from shaping the error line: it is cut and escaped.
String quote(String s) {
  final runes = s.runes.toList();
  final shown = runes.length > 40 ? '${String.fromCharCodes(runes.take(40))}...' : s;
  final b = StringBuffer('"');
  for (final r in shown.runes) {
    if (r == 0x22 || r == 0x5c) {
      b.write('\\${String.fromCharCode(r)}');
    } else if (r < 0x20 || r == 0x7f) {
      b.write('\\x${r.toRadixString(16).padLeft(2, '0')}');
    } else {
      b.writeCharCode(r);
    }
  }
  return (b..write('"')).toString();
}

String _emit(Map<String, String> v) {
  // keys sorted: stable output for scripts
  final sorted = Map.fromEntries(v.entries.toList()..sort((a, b) => a.key.compareTo(b.key)));
  return '${jsonEncode(sorted)}\n';
}

/// The whole program, with its output as arguments so a test can read it.
int run(List<String> args, void Function(String) out, void Function(String) err, {String? commit}) {
  int usageError(String msg) {
    err('@@NAME@@: $msg\nrun `@@NAME@@ --help` for the commands\n');
    return exitUsage;
  }

  var asJson = false;
  final rest = <String>[];
  for (final a in args) {
    if (a == '--json') {
      asJson = true;
    } else if (a == '-h' || a == '--help') {
      out(usage);
      return exitOk;
    } else {
      rest.add(a);
    }
  }
  if (rest.isEmpty) {
    err(usage);
    return exitUsage;
  }
  switch (rest[0]) {
    case 'greet':
      if (rest.length > 2) return usageError('greet takes at most one name');
      final name = rest.length == 2 ? rest[1].trim() : 'world';
      if (name.isEmpty || name.runes.length > 64) {
        err('@@NAME@@: the name is 1 to 64 characters of text\n');
        return exitFailure;
      }
      final message = 'Hello, $name';
      out(asJson ? _emit({'message': message}) : '$message\n');
      return exitOk;
    case 'version' || '--version':
      if (rest.length > 1) return usageError('version takes no arguments');
      final c = commit ?? buildCommit();
      out(asJson ? _emit({'name': '@@NAME@@', 'version': version, 'commit': c}) : '@@NAME@@ $version (commit $c)\n');
      return exitOk;
    default:
      return usageError('unknown command ${quote(rest[0])}');
  }
}
