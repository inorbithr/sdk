import 'dart:convert';
import 'dart:io';

import 'package:@@MODULE@@/@@MODULE@@.dart';
import 'package:test/test.dart';

/// The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an answer
/// is wrong, and the harness refuses a broken dataset or bar instead of grading less.
(int, String, String) run(List<String> args, {Solver? solver}) {
  final out = StringBuffer();
  final err = StringBuffer();
  final code = runSuite(args, out, err, solver: solver);
  return (code, out.toString(), err.toString());
}

String tmp(String name, String text) {
  final dir = Directory.systemTemp.createTempSync('evals-');
  final file = File('${dir.path}/$name')..writeAsStringSync(text);
  return file.path;
}

class FixedSolver implements Solver {
  FixedSolver(this.name, this._solve);

  @override
  final String name;
  final String Function(EvalCase) _solve;

  @override
  bool get stub => true;

  @override
  String solve(EvalCase c) => _solve(c);
}

void main() {
  final cases = loadCases(defaultDataset);
  final theBar = loadBar(defaultBar, scorerNames());

  test('the fixture meets its bar with no network', () {
    // Nothing here can reach a network: the package has no dependency and the stub is local rules.
    final (code, out, _) = run([]);
    expect(code, 0, reason: out);
    expect(out, contains('BAR MET'));
  });

  test('the dataset is a dozen labelled cases covering every label', () {
    expect(cases, hasLength(12));
    for (final label in labels) {
      expect(cases.any((c) => c.expected == label), isTrue, reason: label);
    }
  });

  test('the report says stub and holds ids and scores, never inputs', () {
    final (code, out, _) = run(['--json']);
    expect(code, 0);
    final report = jsonDecode(out) as Map<String, Object?>;
    expect(report['stub'], isTrue);
    expect(report['bar_met'], isTrue);
    expect(report['pass_rate'], 1.0);
    expect(report['solver'] as String, startsWith('stub-'));
    for (final c in cases) {
      expect(out, isNot(contains(c.input)));
    }
  });

  test('two runs give the same report', () {
    expect(run(['--json']), run(['--json']));
  });

  test('lowering one expected answer fails the suite and names the case', () {
    final lines = File(defaultDataset).readAsLinesSync();
    final first = lines[0].replaceFirst(
      '"expected": "billing"',
      '"expected": "other"',
    );
    expect(first, isNot(lines[0]));
    lines[0] = first;
    final file = tmp('cases.jsonl', '${lines.join('\n')}\n');
    final (code, out, _) = run(['--dataset', file]);
    expect(code, 1, reason: out);
    expect(out, contains('BAR MISSED'));
    expect(out, contains('FAILED billing-double-charge'));
  });

  test('a chatty answer fails the format scorer even when it is right', () {
    final chatty = FixedSolver(
      'stub-chatty',
      (c) => 'The label is ${const StubSolver().solve(c)}.',
    );
    final report = evaluate(cases, chatty);
    expect(report.mean('format'), 0.0);
    expect(theBar.missed(report), isNotEmpty);
  });

  test('a solver that throws is an error and never a pass', () {
    final broken = FixedSolver(
      'stub-broken',
      (_) => throw StateError('provider down'),
    );
    final report = evaluate(cases, broken);
    expect(report.errors, 12);
    expect(report.passRate, 0.0);
    expect(theBar.missed(report).any((m) => m.contains('error')), isTrue);
    expect(run([], solver: broken).$1, 1);
  });

  test('a broken dataset stops the run', () {
    const row = '{"id": "a", "input": "i", "expected": "bug"}\n';
    final broken = {
      'twice': '$row$row',
      'input': '{"id": "a", "input": "", "expected": "bug"}\n',
      'no cases': '\n',
      'not JSON': 'not json\n',
    };
    broken.forEach((why, text) {
      final (code, _, err) = run(['--dataset', tmp('c.jsonl', text)]);
      expect(code, 2, reason: why);
      expect(err, contains(why));
    });
  });

  test('a scorer with no bar stops the run', () {
    final file = tmp(
      't.toml',
      '[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n',
    );
    final (code, _, err) = run(['--bar', file]);
    expect(code, 2);
    expect(err, contains('no bar for the scorer(s) format'));
  });

  test('a bar that is not understood stops the run', () {
    for (final text in [
      '[bar]\nmin_pass_rate = high\n',
      'min_pass_rate = 1\n',
      '[bar]\nmin_pass_rate = 2\nmax_errors = 0\n[bar.scorers]\nexact = 1\nformat = 1\n',
    ]) {
      expect(run(['--bar', tmp('t.toml', text)]).$1, 2, reason: text);
    }
  });

  test('a missing bar or dataset stops the run', () {
    expect(run(['--bar', '/nonexistent/t.toml']).$1, 2);
    expect(run(['--dataset', '/nonexistent/c.jsonl']).$1, 2);
  });

  test('the scorers', () {
    final billing = cases[0];
    expect(exact(billing, ' Billing\n'), 1);
    expect(exact(billing, 'bug'), 0);
    expect(format(billing, 'billing'), 1);
    for (final bad in ['Billing', 'billing.', 'billing, bug', '']) {
      expect(format(billing, bad), 0, reason: bad);
    }
  });

  test('every scorer has a bar', () {
    expect(theBar.scorers.keys.toList()..sort(), scorerNames()..sort());
  });
}
