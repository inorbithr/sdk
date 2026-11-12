// The passes-on-init test: golden output in text and JSON (golden/, read from the package
// folder, where `swift test` runs), the exit codes, and a version line that carries a commit.
import Foundation
import XCTest

@testable import Cli

private let commit = "0123456789ab"

private func call(_ args: [String], commit: String = commit) -> (out: String, err: String, code: Int32) {
    var out = "", err = ""
    let code = run(args, out: { out += $0 }, err: { err += $0 }, commit: commit)
    return (out, err, code)
}

private func golden(_ file: String) throws -> String {
    try String(contentsOfFile: FileManager.default.currentDirectoryPath + "/golden/" + file, encoding: .utf8)
}

final class CliTests: XCTestCase {
    func testGoldenTextAndJson() throws {
        let cases: [(String, [String])] = [
            ("help.txt", ["--help"]),
            ("greet.txt", ["greet", "Ada"]),
            ("version.txt", ["version"]),
            ("greet.json", ["--json", "greet", "Ada"]),
            ("version.json", ["version", "--json"]),
        ]
        for (file, args) in cases {
            let r = call(args)
            XCTAssertEqual(r.code, Exit.ok, args.joined(separator: " "))
            XCTAssertEqual(r.out, try golden(file), file)
        }
    }

    func testExitCodes() {
        XCTAssertEqual(call(["greet"]).code, Exit.ok)
        XCTAssertEqual(call([]).code, Exit.usage)
        XCTAssertEqual(call(["nope"]).code, Exit.usage)
        XCTAssertEqual(call(["greet", "a", "b"]).code, Exit.usage)
        XCTAssertEqual(call(["greet", "   "]).code, Exit.failure)
        XCTAssertEqual(call(["greet", String(repeating: "x", count: 65)]).code, Exit.failure)
        XCTAssertEqual(call(["nope"]).out, "", "a usage error writes to stderr, not stdout")
    }

    func testTheVersionLineCarriesACommit() {
        let line = call(["version"], commit: buildCommit()).out
        XCTAssertNotNil(line.range(of: #"\(commit ([0-9a-f]{7,40}|unknown)\)\n$"#, options: .regularExpression), line)
    }

    func testAHostileArgumentIsCutAndEscaped() {
        let err = call(["\u{1b}[31mred" + String(repeating: "x", count: 100)]).err
        XCTAssertFalse(err.contains("\u{1b}"))
        XCTAssertLessThan(err.count, 200, err)
    }
}
