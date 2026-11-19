import Foundation
import XCTest

@testable import @@MODULE@@

/// The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an answer
/// is wrong, and the harness refuses a broken dataset or bar instead of grading less.
final class SuiteTests: XCTestCase {
    struct Captured: Equatable {
        var code: Int32
        var out: String
        var err: String
    }

    struct Fixed: Solver {
        var name: String
        var stub: Bool { true }
        var body: (EvalCase) throws -> String
        func solve(_ c: EvalCase) throws -> String { try body(c) }
    }

    struct ProviderDown: Error {}

    func run(_ args: [String], solver: Solver? = nil) -> Captured {
        var out = ""
        var err = ""
        let code = runSuite(arguments: args, out: &out, err: &err, solver: solver)
        return Captured(code: code, out: out, err: err)
    }

    func tmp(_ name: String, _ text: String) throws -> String {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("evals-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = dir.appendingPathComponent(name)
        try text.write(to: file, atomically: true, encoding: .utf8)
        return file.path
    }

    func cases() throws -> [EvalCase] { try loadCases(defaultDataset) }
    func theBar() throws -> Bar { try loadBar(defaultBar, names: scorerNames()) }

    func testTheFixtureMeetsItsBarWithNoNetwork() {
        // Nothing here can reach a network: the package has no dependency and the stub is local rules.
        let r = run([])
        XCTAssertEqual(r.code, 0, r.out + r.err)
        XCTAssertTrue(r.out.contains("BAR MET"), r.out)
    }

    func testTheDatasetIsADozenLabelledCasesCoveringEveryLabel() throws {
        let all = try cases()
        XCTAssertEqual(all.count, 12)
        for label in labels { XCTAssertTrue(all.contains { $0.expected == label }, label) }
    }

    func testTheReportSaysStubAndHoldsIdsAndScoresNeverInputs() throws {
        let r = run(["--json"])
        XCTAssertEqual(r.code, 0)
        let report = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(r.out.utf8)) as? [String: Any])
        XCTAssertEqual(report["stub"] as? Bool, true)
        XCTAssertEqual(report["bar_met"] as? Bool, true)
        XCTAssertEqual(report["pass_rate"] as? Double, 1.0)
        XCTAssertTrue((report["solver"] as? String ?? "").hasPrefix("stub-"))
        for c in try cases() { XCTAssertFalse(r.out.contains(c.input), c.input) }
    }

    func testTwoRunsGiveTheSameReport() {
        XCTAssertEqual(run(["--json"]), run(["--json"]))
    }

    func testLoweringOneExpectedAnswerFailsTheSuiteAndNamesTheCase() throws {
        let text = try String(contentsOfFile: defaultDataset, encoding: .utf8)
        var lines = text.components(separatedBy: "\n")
        let first = lines[0].replacingOccurrences(of: "\"expected\": \"billing\"", with: "\"expected\": \"other\"")
        XCTAssertNotEqual(first, lines[0])
        lines[0] = first
        let r = run(["--dataset", try tmp("cases.jsonl", lines.joined(separator: "\n"))])
        XCTAssertEqual(r.code, 1, r.out)
        XCTAssertTrue(r.out.contains("BAR MISSED"), r.out)
        XCTAssertTrue(r.out.contains("FAILED billing-double-charge"), r.out)
    }

    func testAChattyAnswerFailsTheFormatScorerEvenWhenItIsRight() throws {
        let chatty = Fixed(name: "stub-chatty") { c in "The label is \(try StubSolver().solve(c))." }
        let report = evaluate(try cases(), chatty)
        XCTAssertEqual(report.mean("format"), 0.0)
        XCTAssertFalse(try theBar().missed(report).isEmpty)
    }

    func testASolverThatThrowsIsAnErrorAndNeverAPass() throws {
        let broken = Fixed(name: "stub-broken") { _ in throw ProviderDown() }
        let report = evaluate(try cases(), broken)
        XCTAssertEqual(report.errors, 12)
        XCTAssertEqual(report.passRate, 0.0)
        XCTAssertTrue(try theBar().missed(report).contains { $0.contains("error") })
        XCTAssertEqual(run([], solver: broken).code, 1)
    }

    func testABrokenDatasetStopsTheRun() throws {
        let row = "{\"id\": \"a\", \"input\": \"i\", \"expected\": \"bug\"}\n"
        let broken = [
            "twice": row + row,
            "input": "{\"id\": \"a\", \"input\": \"\", \"expected\": \"bug\"}\n",
            "no cases": "\n",
            "not JSON": "not json\n",
        ]
        for (why, text) in broken {
            let r = run(["--dataset", try tmp("c.jsonl", text)])
            XCTAssertEqual(r.code, 2, why)
            XCTAssertTrue(r.err.contains(why), "\(why): \(r.err)")
        }
    }

    func testAScorerWithNoBarStopsTheRun() throws {
        let file = try tmp("t.toml", "[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n")
        let r = run(["--bar", file])
        XCTAssertEqual(r.code, 2)
        XCTAssertTrue(r.err.contains("no bar for the scorer(s) format"), r.err)
    }

    func testABarThatIsNotUnderstoodStopsTheRun() throws {
        for text in [
            "[bar]\nmin_pass_rate = high\n",
            "min_pass_rate = 1\n",
            "[bar]\nmin_pass_rate = 2\nmax_errors = 0\n[bar.scorers]\nexact = 1\nformat = 1\n",
        ] {
            XCTAssertEqual(run(["--bar", try tmp("t.toml", text)]).code, 2, text)
        }
    }

    func testAMissingBarOrDatasetStopsTheRun() {
        XCTAssertEqual(run(["--bar", "/nonexistent/t.toml"]).code, 2)
        XCTAssertEqual(run(["--dataset", "/nonexistent/c.jsonl"]).code, 2)
    }

    func testTheScorers() throws {
        let billing = try cases()[0]
        XCTAssertEqual(exact(billing, " Billing\n"), 1)
        XCTAssertEqual(exact(billing, "bug"), 0)
        XCTAssertEqual(format(billing, "billing"), 1)
        for bad in ["Billing", "billing.", "billing, bug", ""] { XCTAssertEqual(format(billing, bad), 0, bad) }
    }

    func testEveryScorerHasABar() throws {
        XCTAssertEqual(try theBar().scorers.keys.sorted(), scorerNames().sorted())
    }
}
