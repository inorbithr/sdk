import Foundation

/// A scorer takes the case and the solver's answer and returns 0 or 1. It is a pure function: the
/// same case and answer give the same score, with no clock, no randomness and no network, so a
/// change in the score is a change in the solver. (A model that grades another model's answer is a
/// solver of its own: label it a stub in tests and give it a bar of its own.)
public typealias Scorer = (EvalCase, String) -> Int

/// The answer is the expected label, ignoring case and surrounding space.
public func exact(_ c: EvalCase, _ answer: String) -> Int {
    let trim = { (s: String) in s.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
    return trim(answer) == trim(c.expected) ? 1 : 0
}

/// The answer is one bare label from the list and nothing else (no sentence, no full stop). A
/// model that is right but chatty breaks the program that reads its answer, so it fails.
public func format(_ c: EvalCase, _ answer: String) -> Int {
    labels.contains(answer) ? 1 : 0
}

/// Every scorer, by name, in the order the report lists them.
public let scorers: [(name: String, score: Scorer)] = [
    ("exact", exact),
    ("format", format),
]

/// The names of every scorer.
public func scorerNames() -> [String] { scorers.map(\.name) }
