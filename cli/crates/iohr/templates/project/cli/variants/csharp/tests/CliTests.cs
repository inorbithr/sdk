using System.Text.RegularExpressions;
using App;

// The passes-on-init test, with no test framework to install: golden output in text and JSON
// (golden/), the exit codes, and a version line that carries a commit. Run from the project
// folder; it exits 1 with the first failure.

const string Commit = "0123456789ab";

static (string Out, string Err, int Code) Call(string commit, params string[] args)
{
    var o = new StringWriter();
    var e = new StringWriter();
    var code = Cli.Run(args, o, e, commit);
    return (o.ToString().Replace("\r\n", "\n"), e.ToString().Replace("\r\n", "\n"), code);
}

static void Check(bool ok, string what)
{
    if (!ok)
    {
        Console.Error.WriteLine("FAIL: " + what);
        Environment.Exit(1);
    }
}

static void Golden(string file, params string[] args)
{
    var (o, _, code) = Call(Commit, args);
    var want = File.ReadAllText(Path.Combine("golden", file)).Replace("\r\n", "\n");
    Check(code == 0 && o == want, $"{string.Join(' ', args)}: exit {code}, output {o} differs from golden/{file}");
}

Golden("help.txt", "--help");
Golden("greet.txt", "greet", "Ada");
Golden("version.txt", "version");
Golden("greet.json", "--json", "greet", "Ada");
Golden("version.json", "version", "--json");

foreach (var (code, argv) in new (int, string[])[]
{
    (Cli.Ok, ["greet"]),
    (Cli.Usage, []),
    (Cli.Usage, ["nope"]),
    (Cli.Usage, ["greet", "a", "b"]),
    (Cli.Failure, ["greet", "   "]),
    (Cli.Failure, ["greet", new string('x', 65)]),
})
{
    var got = Call(Commit, argv).Code;
    Check(got == code, $"{string.Join(' ', argv)}: exit {got}, want {code}");
}
Check(Call(Commit, "nope").Out.Length == 0, "a usage error writes to stderr, not stdout");

// The commit a real build stamps (or "unknown" without one) shows up in the version line.
var line = Call(Cli.BuildCommit(), "version").Out.Trim();
Check(Regex.IsMatch(line, @"\(commit ([0-9a-f]{7,40}|unknown)\)$"), "the version line has a commit: " + line);

var hostile = Call(Commit, "\u001b[31mred" + new string('x', 100)).Err;
Check(!hostile.Contains('\u001b') && hostile.Length < 200, "the error echoed its argument raw: " + hostile);

Console.WriteLine("ok: golden text and JSON, exit codes, version commit, hostile argument");
return 0;
