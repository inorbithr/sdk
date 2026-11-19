import 'dataset.dart';
import 'scorers.dart';
import 'solver.dart';

/// One case, graded. [error] is empty unless the solver failed.
class Result {
  /// Builds a result.
  Result(this.id, this.answer, this.scores, this.error);

  /// The case's id.
  final String id;

  /// What the solver answered.
  final String answer;

  /// Each scorer's 0 or 1.
  final Map<String, int> scores;

  /// The failure's type, or empty.
  final String error;

  /// Every scorer gave 1 and the solver did not fail.
  bool get passed => error.isEmpty && scores.values.every((s) => s == 1);
}

/// Every result and which solver made them.
class Report {
  /// Builds a report.
  Report(this.solver, this.stub, this.results);

  /// The solver's name.
  final String solver;

  /// True when the solver is not a real model.
  final bool stub;

  /// One result per case.
  final List<Result> results;

  /// How many cases pass.
  int get passes => results.where((r) => r.passed).length;

  /// The share of cases that pass.
  double get passRate => passes / results.length;

  /// How many cases the solver failed on.
  int get errors => results.where((r) => r.error.isNotEmpty).length;

  /// One scorer's mean over the cases.
  double mean(String scorer) =>
      results.fold<int>(0, (a, r) => a + (r.scores[scorer] ?? 0)) /
      results.length;
}

/// Runs the solver over the cases and scores each answer. A solver that fails is an error case
/// with every score 0, never a pass. The error kept is the exception's type, never its message: a
/// provider's message can echo the input.
Report evaluate(List<EvalCase> cases, Solver solver) {
  final results = <Result>[];
  for (final c in cases) {
    var answer = '';
    var error = '';
    try {
      answer = solver.solve(c);
    } catch (e) {
      error = e.runtimeType.toString();
    }
    final scores = {
      for (final e in scorers.entries)
        e.key: error.isEmpty ? e.value(c, answer) : 0,
    };
    results.add(Result(c.id, answer, scores, error));
  }
  return Report(solver.name, solver.stub, results);
}
