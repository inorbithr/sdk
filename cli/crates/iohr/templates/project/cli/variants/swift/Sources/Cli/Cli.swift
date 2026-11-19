// @@NAME@@: a command line with text and JSON output, exit codes that mean something, and a
// version that says which commit it was built from. Foundation only.
import Foundation

public let version = "0.1.0"

/// Exit codes: the contract scripts rely on.
public enum Exit {
    public static let ok: Int32 = 0
    public static let failure: Int32 = 1
    public static let usage: Int32 = 2
}

public let usageText = """
    Usage: @@NAME@@ [--json] <command>

    Commands:
      greet [NAME]   Say hello to NAME (default: world)
      version        Print the version and the commit it was built from

    Options:
          --json     Machine-readable output
      -h, --help     Show this help

    Exit codes: 0 success, 1 failure, 2 usage error.

    """

/// Keeps a hostile argument from shaping the error line: it is cut and escaped.
func quote(_ s: String) -> String {
    let scalars = Array(s.unicodeScalars)
    var shown = scalars.count > 40 ? String(String.UnicodeScalarView(scalars.prefix(40))) + "..." : s
    shown = shown.unicodeScalars.reduce(into: "") { out, u in
        if u == "\"" || u == "\\" {
            out += "\\" + String(u)
        } else if u.value < 0x20 || u.value == 0x7f {
            out += String(format: "\\x%02x", u.value)
        } else {
            out.unicodeScalars.append(u)
        }
    }
    return "\"" + shown + "\""
}

private func jsonString(_ s: String) -> String {
    var out = "\""
    for u in s.unicodeScalars {
        switch u {
        case "\"": out += "\\\""
        case "\\": out += "\\\\"
        default:
            if u.value < 0x20 || u.value == 0x7f {
                out += String(format: "\\u%04x", u.value)
            } else {
                out.unicodeScalars.append(u)
            }
        }
    }
    return out + "\""
}

/// Keys are written in sorted order: stable output for scripts.
private func emit(_ v: [String: String]) -> String {
    "{" + v.keys.sorted().map { "\(jsonString($0)):\(jsonString(v[$0]!))" }.joined(separator: ",") + "}\n"
}

/// The whole program, with its output as arguments so a test can read it.
public func run(_ args: [String], out: (String) -> Void, err: (String) -> Void, commit: String? = nil) -> Int32 {
    func usageError(_ msg: String) -> Int32 {
        err("@@NAME@@: \(msg)\nrun `@@NAME@@ --help` for the commands\n")
        return Exit.usage
    }
    var asJson = false
    var rest: [String] = []
    for a in args {
        if a == "--json" {
            asJson = true
        } else if a == "-h" || a == "--help" {
            out(usageText)
            return Exit.ok
        } else {
            rest.append(a)
        }
    }
    guard let command = rest.first else {
        err(usageText)
        return Exit.usage
    }
    switch command {
    case "greet":
        if rest.count > 2 { return usageError("greet takes at most one name") }
        let name = rest.count == 2 ? rest[1].trimmingCharacters(in: .whitespacesAndNewlines) : "world"
        if name.isEmpty || name.unicodeScalars.count > 64 {
            err("@@NAME@@: the name is 1 to 64 characters of text\n")
            return Exit.failure
        }
        let message = "Hello, " + name
        out(asJson ? emit(["message": message]) : message + "\n")
        return Exit.ok
    case "version", "--version":
        if rest.count > 1 { return usageError("version takes no arguments") }
        let c = commit ?? buildCommit()
        out(asJson ? emit(["name": "@@NAME@@", "version": version, "commit": c]) : "@@NAME@@ \(version) (commit \(c))\n")
        return Exit.ok
    default:
        return usageError("unknown command " + quote(command))
    }
}
