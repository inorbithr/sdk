import Foundation

/// Standard output and standard error as text streams, for the program's `main`.
public enum StandardStream: TextOutputStream {
    case out
    case err

    public mutating func write(_ string: String) {
        let data = Data(string.utf8)
        switch self {
        case .out: FileHandle.standardOutput.write(data)
        case .err: FileHandle.standardError.write(data)
        }
    }
}

private func round4(_ x: Double) -> Double { (x * 1e4).rounded() / 1e4 }

private func quote(_ s: String) -> String {
    let data = (try? JSONSerialization.data(withJSONObject: s, options: [.fragmentsAllowed, .withoutEscapingSlashes])) ?? Data()
    return String(decoding: data, as: UTF8.self)
}

private func reportJSON(_ report: Report, _ missed: [String]) -> String {
    let scoreLines = scorerNames().map { "    \(quote($0)): \(round4(report.mean($0)))" }
    let results = report.results.map { r -> String in
        let scores = scorerNames().map { "\(quote($0)): \(r.scores[$0] ?? 0)" }.joined(separator: ", ")
        return "    {\n      \"id\": \(quote(r.id)),\n      \"passed\": \(r.passed),\n      \"scores\": {\(scores)},\n"
            + "      \"error\": \(r.error.isEmpty ? "null" : quote(r.error))\n    }"
    }
    func list(_ items: [String]) -> String { items.isEmpty ? "[]" : "[\n" + items.joined(separator: ",\n") + "\n  ]" }
    return "{\n  \"solver\": \(quote(report.solver)),\n  \"stub\": \(report.stub),\n"
        + "  \"cases\": \(report.results.count),\n  \"pass_rate\": \(round4(report.passRate)),\n"
        + "  \"errors\": \(report.errors),\n  \"scorers\": {\n\(scoreLines.joined(separator: ",\n"))\n  },\n"
        + "  \"bar_met\": \(missed.isEmpty),\n  \"missed\": \(list(missed.map { "    " + quote($0) })),\n"
        + "  \"results\": \(list(results))\n}"
}

/// Grades the solver on the dataset and checks the pass bar. Returns 0 when the bar is met, 1 when
/// it is missed, 2 when the dataset or the bar is wrong. The report says which solver ran and
/// whether it is a stub. It holds case ids and scores only; the inputs and answers stay out of it
/// (they can be customer data), unless you ask for --show, which prints the failed cases to your
/// terminal and nowhere else. A nil `solver` is the stub.
public func runSuite<O: TextOutputStream, E: TextOutputStream>(
    arguments: [String], out: inout O, err: inout E, solver: Solver? = nil
) -> Int32 {
    var dataset = defaultDataset
    var barPath = defaultBar
    var asJson = false
    var show = false
    var i = 0
    while i < arguments.count {
        switch arguments[i] {
        case "--dataset", "--bar":
            guard i + 1 < arguments.count else {
                print("usage error: \(arguments[i]) needs a value", to: &err)
                return 2
            }
            if arguments[i] == "--dataset" { dataset = arguments[i + 1] } else { barPath = arguments[i + 1] }
            i += 1
        case "--json": asJson = true
        case "--show": show = true
        default:
            print("usage error: unknown argument \(arguments[i])", to: &err)
            return 2
        }
        i += 1
    }
    let theSolver: Solver = solver ?? StubSolver()
    let cases: [EvalCase]
    let bar: Bar
    do {
        cases = try loadCases(dataset)
        bar = try loadBar(barPath, names: scorerNames())
    } catch let failure as SuiteError {
        print("suite error: \(failure.message)", to: &err)
        return 2
    } catch {
        print("suite error: \(error.localizedDescription)", to: &err)
        return 2
    }
    let report = evaluate(cases, theSolver)
    let missed = bar.missed(report)
    if asJson {
        print(reportJSON(report, missed), to: &out)
    } else {
        let label = report.stub ? "STUB solver, not a model: " : ""
        print("\(label)\(report.solver): \(report.passes)/\(report.results.count) cases pass", to: &out)
        for n in scorerNames() {
            print("  \(n): mean \(f3(report.mean(n))) (bar \(f3(bar.scorers[n] ?? 0)))", to: &out)
        }
        print(
            "  pass rate \(f3(report.passRate)) (bar \(f3(bar.minPassRate))), errors \(report.errors) (at most \(bar.maxErrors))",
            to: &out)
        for r in report.results where !r.passed {
            let why = r.error.isEmpty ? scorerNames().filter { r.scores[$0] == 0 }.joined(separator: ", ") : r.error
            print("  FAILED \(r.id): \(why)", to: &out)
        }
        print(missed.isEmpty ? "BAR MET" : "BAR MISSED: " + missed.joined(separator: "; "), to: &out)
    }
    if show {
        for r in report.results where !r.passed {
            if let c = cases.first(where: { $0.id == r.id }) {
                print("  \(r.id): input=\(quote(c.input)) expected=\(quote(c.expected)) answer=\(quote(r.answer))", to: &err)
            }
        }
    }
    return missed.isEmpty ? 0 : 1
}
