import 'dart:convert';

import 'bar.dart';
import 'dataset.dart';
import 'runner.dart';
import 'scorers.dart';
import 'solver.dart';

double _round4(double x) => (x * 1e4).round() / 1e4;

String _f3(double x) => x.toStringAsFixed(3);

String _reportJson(Report report, List<String> missed) {
  final doc = <String, Object?>{
    'solver': report.solver,
    'stub': report.stub,
    'cases': report.results.length,
    'pass_rate': _round4(report.passRate),
    'errors': report.errors,
    'scorers': {for (final n in scorerNames()) n: _round4(report.mean(n))},
    'bar_met': missed.isEmpty,
    'missed': missed,
    'results': [
      for (final r in report.results)
        {
          'id': r.id,
          'passed': r.passed,
          'scores': r.scores,
          'error': r.error.isEmpty ? null : r.error,
        },
    ],
  };
  return const JsonEncoder.withIndent('  ').convert(doc);
}

/// Grades the solver on the dataset and checks the pass bar. Returns 0 when the bar is met, 1 when
/// it is missed, 2 when the dataset or the bar is wrong. The report says which solver ran and
/// whether it is a stub. It holds case ids and scores only; the inputs and answers stay out of it
/// (they can be customer data), unless you ask for --show, which prints the failed cases to your
/// terminal and nowhere else. A null [solver] is the stub.
int runSuite(
  List<String> args,
  StringSink out,
  StringSink err, {
  Solver? solver,
}) {
  var dataset = defaultDataset;
  var barPath = defaultBar;
  var asJson = false;
  var show = false;
  for (var i = 0; i < args.length; i++) {
    switch (args[i]) {
      case '--dataset' || '--bar':
        if (i + 1 >= args.length) {
          err.writeln('usage error: ${args[i]} needs a value');
          return 2;
        }
        if (args[i] == '--dataset') {
          dataset = args[++i];
        } else {
          barPath = args[++i];
        }
      case '--json':
        asJson = true;
      case '--show':
        show = true;
      default:
        err.writeln('usage error: unknown argument ${args[i]}');
        return 2;
    }
  }
  final theSolver = solver ?? const StubSolver();
  final List<EvalCase> cases;
  final Bar bar;
  try {
    cases = loadCases(dataset);
    bar = loadBar(barPath, scorerNames());
  } on SuiteError catch (e) {
    err.writeln('suite error: ${e.message}');
    return 2;
  }
  final report = evaluate(cases, theSolver);
  final missed = bar.missed(report);
  if (asJson) {
    out.writeln(_reportJson(report, missed));
  } else {
    final label = report.stub ? 'STUB solver, not a model: ' : '';
    out.writeln(
      '$label${report.solver}: ${report.passes}/${report.results.length} cases pass',
    );
    for (final n in scorerNames()) {
      out.writeln(
        '  $n: mean ${_f3(report.mean(n))} (bar ${_f3(bar.scorers[n]!)})',
      );
    }
    out.writeln(
      '  pass rate ${_f3(report.passRate)} (bar ${_f3(bar.minPassRate)}), '
      'errors ${report.errors} (at most ${bar.maxErrors})',
    );
    for (final r in report.results.where((r) => !r.passed)) {
      final why = r.error.isNotEmpty
          ? r.error
          : scorerNames().where((n) => r.scores[n] == 0).join(', ');
      out.writeln('  FAILED ${r.id}: $why');
    }
    out.writeln(
      missed.isEmpty ? 'BAR MET' : 'BAR MISSED: ${missed.join('; ')}',
    );
  }
  if (show) {
    final byId = {for (final c in cases) c.id: c};
    for (final r in report.results.where((r) => !r.passed)) {
      final c = byId[r.id]!;
      err.writeln(
        '  ${r.id}: input=${jsonEncode(c.input)} expected=${jsonEncode(c.expected)} '
        'answer=${jsonEncode(r.answer)}',
      );
    }
  }
  return missed.isEmpty ? 0 : 1;
}
