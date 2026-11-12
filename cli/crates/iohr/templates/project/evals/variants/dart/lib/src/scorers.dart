import 'dataset.dart';
import 'solver.dart';

/// A scorer takes the case and the solver's answer and returns 0 or 1. It is a pure function: the
/// same case and answer give the same score, with no clock, no randomness and no network, so a
/// change in the score is a change in the solver. (A model that grades another model's answer is a
/// solver of its own: label it a stub in tests and give it a bar of its own.)
typedef Scorer = int Function(EvalCase c, String answer);

/// The answer is the expected label, ignoring case and surrounding space.
int exact(EvalCase c, String answer) =>
    answer.trim().toLowerCase() == c.expected.trim().toLowerCase() ? 1 : 0;

/// The answer is one bare label from the list and nothing else (no sentence, no full stop). A
/// model that is right but chatty breaks the program that reads its answer, so it fails.
int format(EvalCase c, String answer) => labels.contains(answer) ? 1 : 0;

/// Every scorer, by name, in the order the report lists them.
const scorers = <String, Scorer>{'exact': exact, 'format': format};

/// The names of every scorer.
List<String> scorerNames() => scorers.keys.toList();
