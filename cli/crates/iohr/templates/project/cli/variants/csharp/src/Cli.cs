using System.Reflection;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace App;

/// <summary>
/// @@NAME@@: a command line with text and JSON output, exit codes that mean something, and a
/// version that says which commit it was built from. The .NET libraries only.
/// </summary>
public static class Cli
{
    public const string Version = "0.1.0";

    /// <summary>Exit codes: the contract scripts rely on.</summary>
    public const int Ok = 0;
    public const int Failure = 1;
    public const int Usage = 2;

    private static readonly Regex CommitPattern = new("^[0-9a-f]{7,40}$");

    private static readonly JsonSerializerOptions JsonOptions = new() { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

    public const string UsageText = """
        Usage: @@NAME@@ [--json] <command>

        Commands:
          greet [NAME]   Say hello to NAME (default: world)
          version        Print the version and the commit it was built from

        Options:
              --json     Machine-readable output
          -h, --help     Show this help

        Exit codes: 0 success, 1 failure, 2 usage error.

        """;

    /// <summary>
    /// The commit this was built from: the project stamps it into the informational version
    /// ("0.1.0+0123456789ab") from git; without one it is "unknown".
    /// </summary>
    public static string BuildCommit()
    {
        var info = typeof(Cli).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion;
        var at = info?.IndexOf('+') ?? -1;
        if (info is not null && at >= 0)
        {
            var commit = info[(at + 1)..];
            if (CommitPattern.IsMatch(commit))
            {
                return commit.Length > 12 ? commit[..12] : commit;
            }
        }
        return "unknown";
    }

    /// <summary>The whole program, with its streams as arguments so a test can read them.</summary>
    public static int Run(string[] args, TextWriter stdout, TextWriter stderr, string commit)
    {
        var asJson = false;
        var rest = new List<string>();
        foreach (var a in args)
        {
            switch (a)
            {
                case "--json":
                    asJson = true;
                    break;
                case "-h" or "--help":
                    stdout.Write(UsageText);
                    return Ok;
                default:
                    rest.Add(a);
                    break;
            }
        }
        if (rest.Count == 0)
        {
            stderr.Write(UsageText);
            return Usage;
        }
        switch (rest[0])
        {
            case "greet":
                return Greet(rest.Skip(1).ToList(), asJson, stdout, stderr);
            case "version" or "--version":
                return rest.Count > 1
                    ? UsageError(stderr, "version takes no arguments")
                    : PrintVersion(asJson, stdout, commit);
            default:
                return UsageError(stderr, "unknown command " + Quote(rest[0]));
        }
    }

    private static int UsageError(TextWriter stderr, string msg)
    {
        stderr.WriteLine($"@@NAME@@: {msg}");
        stderr.WriteLine("run `@@NAME@@ --help` for the commands");
        return Usage;
    }

    /// <summary>Keeps a hostile argument from shaping the error line: it is cut and escaped.</summary>
    private static string Quote(string s)
    {
        var elements = new System.Globalization.StringInfo(s);
        var shown = elements.LengthInTextElements > 40 ? elements.SubstringByTextElements(0, 40) + "..." : s;
        var b = new StringBuilder("\"");
        foreach (var c in shown)
        {
            if (c is '"' or '\\')
            {
                b.Append('\\').Append(c);
            }
            else if (c < 0x20 || c == 0x7f)
            {
                b.Append($"\\x{(int)c:x2}");
            }
            else
            {
                b.Append(c);
            }
        }
        return b.Append('"').ToString();
    }

    private static string Str(string s) => JsonSerializer.Serialize(s, JsonOptions);

    private static int Greet(List<string> args, bool asJson, TextWriter stdout, TextWriter stderr)
    {
        if (args.Count > 1)
        {
            return UsageError(stderr, "greet takes at most one name");
        }
        var name = args.Count == 1 ? args[0].Trim() : "world";
        if (name.Length == 0 || new System.Globalization.StringInfo(name).LengthInTextElements > 64)
        {
            stderr.WriteLine("@@NAME@@: the name is 1 to 64 characters of text");
            return Failure;
        }
        var msg = "Hello, " + name;
        // Keys are written in sorted order: stable output for scripts.
        stdout.WriteLine(asJson ? "{\"message\":" + Str(msg) + "}" : msg);
        return Ok;
    }

    private static int PrintVersion(bool asJson, TextWriter stdout, string commit)
    {
        stdout.WriteLine(asJson
            ? "{\"commit\":" + Str(commit) + ",\"name\":\"@@NAME@@\",\"version\":\"" + Version + "\"}"
            : $"@@NAME@@ {Version} (commit {commit})");
        return Ok;
    }
}
