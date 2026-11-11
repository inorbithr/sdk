import 'dart:convert';
import 'dart:io';

/// The dataset or the bar is wrong. The run stops and exits 2: nothing was graded.
class SuiteError implements Exception {
  /// Says what is wrong.
  SuiteError(this.message);

  /// What is wrong.
  final String message;

  @override
  String toString() => message;
}

/// One line of the dataset: an id, an input and the expected answer.
class EvalCase {
  /// Builds a case.
  const EvalCase(this.id, this.input, this.expected);

  /// The case's id, unique in the dataset.
  final String id;

  /// What the solver is given.
  final String input;

  /// The answer the scorers look for.
  final String expected;
}

/// The default dataset, relative to the project folder.
const defaultDataset = 'datasets/tickets.jsonl';

/// The default bar, relative to the project folder.
const defaultBar = 'thresholds.toml';

/// The reason an I/O error carries, without the OS code.
String reason(FileSystemException e) {
  final text = e.osError?.message ?? e.message;
  return text.toLowerCase();
}

/// Reads the dataset (JSON Lines). A broken one fails loudly instead of quietly scoring fewer cases.
List<EvalCase> loadCases(String path) {
  final String text;
  try {
    text = File(path).readAsStringSync();
  } on FileSystemException catch (e) {
    throw SuiteError('cannot read the dataset $path: ${reason(e)}');
  }
  final name = path.split(Platform.pathSeparator).last;
  final cases = <EvalCase>[];
  final seen = <String>{};
  final lines = text.split('\n');
  for (var i = 0; i < lines.length; i++) {
    final number = i + 1;
    if (lines[i].trim().isEmpty) continue;
    final Object? row;
    try {
      row = jsonDecode(lines[i]);
    } on FormatException catch (e) {
      throw SuiteError('$name line $number: not JSON (${e.message})');
    }
    if (row is! Map<String, Object?>) {
      throw SuiteError('$name line $number: expected an object');
    }
    final object = row;
    String field(String key) {
      final v = object[key];
      if (v is String && v.trim().isNotEmpty) return v;
      throw SuiteError('$name line $number: `$key` must be a non-empty string');
    }

    final id = field('id');
    final input = field('input');
    final expected = field('expected');
    if (!seen.add(id)) {
      throw SuiteError('$name line $number: the id `$id` is used twice');
    }
    cases.add(EvalCase(id, input, expected));
  }
  if (cases.isEmpty) {
    throw SuiteError('$name has no cases: an empty suite passes nothing');
  }
  return cases;
}
