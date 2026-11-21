/// One case, graded. `error` is empty unless the solver failed.
public struct CaseResult {
    public let id: String
    public let answer: String
    public let scores: [String: Int]
    public let error: String

    /// Every scorer gave 1 and the solver did not fail.
    public var passed: Bool { error.isEmpty && scores.values.allSatisfy { $0 == 1 } }
}

/// Every result and which solver made them.
public struct Report {
    public let solver: String
    public let stub: Bool
    public let results: [CaseResult]

    public var passes: Int { results.filter(\.passed).count }
    public var passRate: Double { Double(passes) / Double(results.count) }
    public var errors: Int { results.filter { !$0.error.isEmpty }.count }

    /// One scorer's mean over the cases.
    public func mean(_ scorer: String) -> Double {
        Double(results.reduce(0) { $0 + ($1.scores[scorer] ?? 0) }) / Double(results.count)
    }
}

/// Runs the solver over the cases and scores each answer. A solver that fails is an error case
/// with every score 0, never a pass. The error kept is the error's type, never its message: a
/// provider's message can echo the input.
public func evaluate(_ cases: [EvalCase], _ solver: Solver) -> Report {
    var results: [CaseResult] = []
    for c in cases {
        var answer = ""
        var error = ""
        do {
            answer = try solver.solve(c)
        } catch let failure {
            error = String(describing: type(of: failure))
        }
        var scores: [String: Int] = [:]
        for s in scorers { scores[s.name] = error.isEmpty ? s.score(c, answer) : 0 }
        results.append(CaseResult(id: c.id, answer: answer, scores: scores, error: error))
    }
    return Report(solver: solver.name, stub: solver.stub, results: results)
}
