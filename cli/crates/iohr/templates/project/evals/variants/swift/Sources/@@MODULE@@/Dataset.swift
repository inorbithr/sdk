import Foundation

/// The dataset or the bar is wrong. The run stops and exits 2: nothing was graded.
public struct SuiteError: Error, CustomStringConvertible {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var description: String { message }
}

/// One line of the dataset: an id, an input and the expected answer.
public struct EvalCase: Equatable {
    public let id: String
    public let input: String
    public let expected: String

    public init(id: String, input: String, expected: String) {
        self.id = id
        self.input = input
        self.expected = expected
    }
}

/// The default dataset, relative to the project folder.
public let defaultDataset = "datasets/tickets.jsonl"

/// The default bar, relative to the project folder.
public let defaultBar = "thresholds.toml"

func readText(_ path: String, what: String) throws -> String {
    guard FileManager.default.fileExists(atPath: path) else {
        throw SuiteError("cannot read the \(what) \(path): no such file or directory")
    }
    do {
        return try String(contentsOfFile: path, encoding: .utf8)
    } catch {
        throw SuiteError("cannot read the \(what) \(path): \(error.localizedDescription)")
    }
}

/// Reads the dataset (JSON Lines). A broken one fails loudly instead of quietly scoring fewer cases.
public func loadCases(_ path: String) throws -> [EvalCase] {
    let text = try readText(path, what: "dataset")
    let name = (path as NSString).lastPathComponent
    var cases: [EvalCase] = []
    var seen = Set<String>()
    for (index, line) in text.components(separatedBy: "\n").enumerated() {
        let number = index + 1
        if line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { continue }
        let parsed: Any
        do {
            parsed = try JSONSerialization.jsonObject(with: Data(line.utf8))
        } catch {
            throw SuiteError("\(name) line \(number): not JSON (\(error.localizedDescription))")
        }
        guard let row = parsed as? [String: Any] else {
            throw SuiteError("\(name) line \(number): expected an object")
        }
        func field(_ key: String) throws -> String {
            if let value = row[key] as? String,
                !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            {
                return value
            }
            throw SuiteError("\(name) line \(number): `\(key)` must be a non-empty string")
        }
        let id = try field("id")
        let input = try field("input")
        let expected = try field("expected")
        if !seen.insert(id).inserted {
            throw SuiteError("\(name) line \(number): the id `\(id)` is used twice")
        }
        cases.append(EvalCase(id: id, input: input, expected: expected))
    }
    if cases.isEmpty {
        throw SuiteError("\(name) has no cases: an empty suite passes nothing")
    }
    return cases
}
