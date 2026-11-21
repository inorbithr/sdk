import 'dart:io';

import 'dataset.dart';
import 'runner.dart';

final _table = RegExp(r'^\[([A-Za-z0-9_.]+)\]$');
final _key = RegExp(r'^([A-Za-z0-9_]+)\s*=\s*([+-]?[0-9]+(\.[0-9]+)?)$');

String _f3(double x) => x.toStringAsFixed(3);

/// What the suite must reach to pass, read from thresholds.toml.
class Bar {
  /// Builds a bar.
  Bar(this.minPassRate, this.maxErrors, this.scorers);

  /// The lowest share of cases that may pass.
  final double minPassRate;

  /// The most solver failures allowed.
  final int maxErrors;

  /// The lowest mean for each scorer.
  final Map<String, double> scorers;

  /// Every line of the bar the report does not reach; empty means the bar is met.
  List<String> missed(Report report) {
    final out = <String>[];
    if (report.passRate < minPassRate) {
      out.add('pass rate ${_f3(report.passRate)} is below ${_f3(minPassRate)}');
    }
    if (report.errors > maxErrors) {
      out.add('${report.errors} error(s), at most $maxErrors allowed');
    }
    final names = scorers.keys.toList()..sort();
    for (final name in names) {
      final mean = report.mean(name);
      if (mean < scorers[name]!) {
        out.add(
          'scorer $name: mean ${_f3(mean)} is below ${_f3(scorers[name]!)}',
        );
      }
    }
    return out;
  }
}

/// Reads the small part of TOML the bar uses: `[table]` lines, `key = number` lines and `#`
/// comments. Dart has no TOML reader in its core libraries, and the bar needs nothing more, so
/// anything else is refused rather than guessed at.
Map<String, Map<String, String>> _tables(String name, String text) {
  final tables = <String, Map<String, String>>{};
  Map<String, String>? current;
  final lines = text.split('\n');
  for (var i = 0; i < lines.length; i++) {
    var line = lines[i];
    final hash = line.indexOf('#');
    if (hash >= 0) line = line.substring(0, hash);
    line = line.trim();
    if (line.isEmpty) continue;
    final table = _table.firstMatch(line);
    if (table != null) {
      final title = table.group(1)!;
      if (tables.containsKey(title)) {
        throw SuiteError(
          '$name: not valid TOML (line ${i + 1}: table [$title] twice)',
        );
      }
      current = tables[title] = {};
      continue;
    }
    final key = _key.firstMatch(line);
    if (key == null || current == null) {
      throw SuiteError(
        '$name: not valid TOML (line ${i + 1}: only `[table]` and `key = number` are read here)',
      );
    }
    if (current.containsKey(key.group(1))) {
      throw SuiteError(
        '$name: not valid TOML (line ${i + 1}: key ${key.group(1)} twice)',
      );
    }
    current[key.group(1)!] = key.group(2)!;
  }
  return tables;
}

/// Reads the bar and checks it guards every scorer.
Bar loadBar(String path, List<String> names) {
  final file = path.split(Platform.pathSeparator).last;
  final String text;
  try {
    text = File(path).readAsStringSync();
  } on FileSystemException catch (e) {
    throw SuiteError('cannot read the bar $path: ${reason(e)}');
  }
  final tables = _tables(file, text);
  final bar = tables['bar'];
  if (bar == null) throw SuiteError('$file: the [bar] table is missing');
  final per = tables['bar.scorers'];
  if (!bar.containsKey('min_pass_rate') ||
      !bar.containsKey('max_errors') ||
      per == null ||
      bar['max_errors']!.contains('.')) {
    throw SuiteError(
      '$file: [bar] needs min_pass_rate, max_errors and [bar.scorers]',
    );
  }
  final rate = double.parse(bar['min_pass_rate']!);
  final errors = int.parse(bar['max_errors']!.replaceFirst('+', ''));
  final range = '$file: every rate is a number from 0 to 1';
  if (rate < 0 || rate > 1) throw SuiteError(range);
  final scorers = <String, double>{};
  for (final e in per.entries) {
    final f = double.parse(e.value);
    if (f < 0 || f > 1) throw SuiteError(range);
    scorers[e.key] = f;
  }
  if (errors < 0) throw SuiteError('$file: max_errors cannot be negative');
  final unguarded = names.where((n) => !scorers.containsKey(n)).toList()
    ..sort();
  final unknown = scorers.keys.where((k) => !names.contains(k)).toList()
    ..sort();
  if (unguarded.isNotEmpty) {
    throw SuiteError(
      '$file: no bar for the scorer(s) ${unguarded.join(', ')}; a scorer with no bar guards nothing',
    );
  }
  if (unknown.isNotEmpty) {
    throw SuiteError(
      '$file: a bar for ${unknown.join(', ')}, which is not a scorer',
    );
  }
  return Bar(rate, errors, scorers);
}
