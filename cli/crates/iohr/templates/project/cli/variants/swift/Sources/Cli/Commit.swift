import Foundation

/// The commit a release build stamps in: its build step rewrites this one line with the full
/// hash (`sed -i 's/stampedCommit = ""/stampedCommit = "'"$(git rev-parse HEAD)"'"/'`).
/// Left empty, the program asks git in the folder it runs from, and says "unknown" without one.
let stampedCommit = ""

func isCommit(_ s: String) -> Bool {
    (7...40).contains(s.count) && s.allSatisfy { $0.isASCII && ($0.isNumber || ("a"..."f").contains($0)) }
}

public func buildCommit() -> String {
    if isCommit(stampedCommit) { return String(stampedCommit.prefix(12)) }
    let git = Process()
    git.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    git.arguments = ["git", "rev-parse", "--short=12", "HEAD"]
    git.currentDirectoryURL = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
    let pipe = Pipe()
    git.standardOutput = pipe
    git.standardError = FileHandle.nullDevice
    do {
        try git.run()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        git.waitUntilExit()
        let out = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        if git.terminationStatus == 0, isCommit(out) { return out }
    } catch {
        // no git, or not a checkout
    }
    return "unknown"
}
