import 'dataset.dart';

/// The thing under test: it turns a case's input into an answer. The one here
/// ([StubSolver]) is a STUB (a fixed list of keyword rules), labelled as one everywhere it shows up
/// (`stub: true` in the report, a `stub-` name). It exists so the suite runs offline, is free and
/// gives the same answer every time. To grade a real model, write a class with these three members
/// that calls your provider, sign in the way your provider's subscription allows and keep the
/// credential in your secret store, never in this folder. A solver that throws is counted as an
/// error, never a pass.
abstract interface class Solver {
  /// The solver's name, in the report.
  String get name;

  /// True for anything that is not a real model.
  bool get stub;

  /// The answer for one case.
  String solve(EvalCase c);
}

/// The answers the suite knows, in the order the rules list them.
const labels = ['billing', 'bug', 'feature_request', 'account', 'other'];

const _rules = <(String, List<String>)>[
  ('billing', ['charged', 'refund', 'invoice', 'payment', 'card']),
  ('bug', ['crash', 'error', 'blank screen']),
  ('feature_request', ['would be great', 'add an option', 'feature']),
  ('account', ['password', 'account']),
];

/// Keyword rules. Not a model: nothing is learned, nothing is called.
class StubSolver implements Solver {
  /// A stub solver.
  const StubSolver();

  @override
  String get name => 'stub-keyword-rules-v1';

  @override
  bool get stub => true;

  @override
  String solve(EvalCase c) {
    final text = c.input.toLowerCase();
    for (final (label, words) in _rules) {
      if (words.any(text.contains)) return label;
    }
    return 'other';
  }
}
