/// The thing under test: it turns a case's input into an answer. The one here
/// (`StubSolver`) is a STUB (a fixed list of keyword rules), labelled as one everywhere it shows up
/// (`stub: true` in the report, a `stub-` name). It exists so the suite runs offline, is free and
/// gives the same answer every time. To grade a real model, write a type with these three members
/// that calls your provider, sign in the way your provider's subscription allows and keep the
/// credential in your secret store, never in this folder. A solver that throws is counted as an
/// error, never a pass.
public protocol Solver {
    /// The solver's name, in the report.
    var name: String { get }
    /// True for anything that is not a real model.
    var stub: Bool { get }
    /// The answer for one case.
    func solve(_ c: EvalCase) throws -> String
}

/// The answers the suite knows, in the order the rules list them.
public let labels = ["billing", "bug", "feature_request", "account", "other"]

private let rules: [(label: String, words: [String])] = [
    ("billing", ["charged", "refund", "invoice", "payment", "card"]),
    ("bug", ["crash", "error", "blank screen"]),
    ("feature_request", ["would be great", "add an option", "feature"]),
    ("account", ["password", "account"]),
]

/// Keyword rules. Not a model: nothing is learned, nothing is called.
public struct StubSolver: Solver {
    public init() {}
    public var name: String { "stub-keyword-rules-v1" }
    public var stub: Bool { true }

    public func solve(_ c: EvalCase) throws -> String {
        let text = c.input.lowercased()
        for rule in rules where rule.words.contains(where: { text.contains($0) }) {
            return rule.label
        }
        return "other"
    }
}
