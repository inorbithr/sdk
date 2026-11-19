import Foundation

/// What the suite must reach to pass, read from thresholds.toml.
public struct Bar {
    public let minPassRate: Double
    public let maxErrors: Int
    public let scorers: [String: Double]

    /// Every line of the bar the report does not reach; empty means the bar is met.
    public func missed(_ report: Report) -> [String] {
        var out: [String] = []
        if report.passRate < minPassRate {
            out.append("pass rate \(f3(report.passRate)) is below \(f3(minPassRate))")
        }
        if report.errors > maxErrors {
            out.append("\(report.errors) error(s), at most \(maxErrors) allowed")
        }
        for name in scorers.keys.sorted() {
            let mean = report.mean(name)
            if let min = scorers[name], mean < min {
                out.append("scorer \(name): mean \(f3(mean)) is below \(f3(min))")
            }
        }
        return out
    }
}

func f3(_ x: Double) -> String { String(format: "%.3f", x) }

private func match(_ pattern: String, _ line: String) -> [String]? {
    guard let re = try? NSRegularExpression(pattern: pattern) else { return nil }
    let range = NSRange(line.startIndex..., in: line)
    guard let m = re.firstMatch(in: line, range: range) else { return nil }
    return (1..<m.numberOfRanges).map { i in
        Range(m.range(at: i), in: line).map { String(line[$0]) } ?? ""
    }
}

/// Reads the small part of TOML the bar uses: `[table]` lines, `key = number` lines and `#`
/// comments. Foundation has no TOML reader, and the bar needs nothing more, so anything else is
/// refused rather than guessed at.
private func tables(_ name: String, _ text: String) throws -> [String: [String: String]] {
    var tables: [String: [String: String]] = [:]
    var current: String?
    for (index, raw) in text.components(separatedBy: "\n").enumerated() {
        var line = raw
        if let hash = line.firstIndex(of: "#") { line = String(line[..<hash]) }
        line = line.trimmingCharacters(in: .whitespacesAndNewlines)
        if line.isEmpty { continue }
        if let table = match(#"^\[([A-Za-z0-9_.]+)\]$"#, line) {
            if tables[table[0]] != nil {
                throw SuiteError("\(name): not valid TOML (line \(index + 1): table [\(table[0])] twice)")
            }
            tables[table[0]] = [:]
            current = table[0]
            continue
        }
        guard let key = match(#"^([A-Za-z0-9_]+)\s*=\s*([+-]?[0-9]+(\.[0-9]+)?)$"#, line), let table = current else {
            throw SuiteError(
                "\(name): not valid TOML (line \(index + 1): only `[table]` and `key = number` are read here)")
        }
        if tables[table]?[key[0]] != nil {
            throw SuiteError("\(name): not valid TOML (line \(index + 1): key \(key[0]) twice)")
        }
        tables[table]?[key[0]] = key[1]
    }
    return tables
}

/// Reads the bar and checks it guards every scorer.
public func loadBar(_ path: String, names: [String]) throws -> Bar {
    let file = (path as NSString).lastPathComponent
    let parsed = try tables(file, try readText(path, what: "bar"))
    guard let bar = parsed["bar"] else { throw SuiteError("\(file): the [bar] table is missing") }
    guard let rawRate = bar["min_pass_rate"], let rawErrors = bar["max_errors"],
        let per = parsed["bar.scorers"], !rawErrors.contains(".")
    else {
        throw SuiteError("\(file): [bar] needs min_pass_rate, max_errors and [bar.scorers]")
    }
    let range = "\(file): every rate is a number from 0 to 1"
    guard let rate = Double(rawRate), (0...1).contains(rate) else { throw SuiteError(range) }
    var scorers: [String: Double] = [:]
    for (k, v) in per {
        guard let f = Double(v), (0...1).contains(f) else { throw SuiteError(range) }
        scorers[k] = f
    }
    guard let errors = Int(rawErrors.replacingOccurrences(of: "+", with: "")), errors >= 0 else {
        throw SuiteError("\(file): max_errors cannot be negative")
    }
    let unguarded = names.filter { scorers[$0] == nil }.sorted()
    let unknown = scorers.keys.filter { !names.contains($0) }.sorted()
    if !unguarded.isEmpty {
        throw SuiteError(
            "\(file): no bar for the scorer(s) \(unguarded.joined(separator: ", ")); a scorer with no bar guards nothing")
    }
    if !unknown.isEmpty {
        throw SuiteError("\(file): a bar for \(unknown.joined(separator: ", ")), which is not a scorer")
    }
    return Bar(minPassRate: rate, maxErrors: errors, scorers: scorers)
}
