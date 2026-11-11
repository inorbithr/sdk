using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;

namespace @@MODULE@@;

/// <summary>
/// Grade the solver on the dataset and check the pass bar. Exit 0 when the bar is met, 1 when it is
/// missed, 2 when the dataset or the bar is wrong. The report says which solver ran and whether it
/// is a stub. It holds case ids and scores only; the inputs and answers stay out of it (they can be
/// customer data), unless you ask for --show, which prints the failed cases to your terminal and
/// nowhere else.
/// </summary>
public static class Suite
{
    private static double Round4(double x) => Math.Round(x * 1e4) / 1e4;

    private static string F3(double x) => x.ToString("F3", CultureInfo.InvariantCulture);

    private static string ReportJson(Report report, IReadOnlyList<string> missed)
    {
        using var stream = new MemoryStream();
        using (var w = new Utf8JsonWriter(stream, new JsonWriterOptions { Indented = true }))
        {
            w.WriteStartObject();
            w.WriteString("solver", report.Solver);
            w.WriteBoolean("stub", report.Stub);
            w.WriteNumber("cases", report.Results.Count);
            w.WriteNumber("pass_rate", Round4(report.PassRate));
            w.WriteNumber("errors", report.Errors);
            w.WriteStartObject("scorers");
            foreach (var n in Scorers.Names())
            {
                w.WriteNumber(n, Round4(report.Mean(n)));
            }
            w.WriteEndObject();
            w.WriteBoolean("bar_met", missed.Count == 0);
            w.WriteStartArray("missed");
            foreach (var m in missed)
            {
                w.WriteStringValue(m);
            }
            w.WriteEndArray();
            w.WriteStartArray("results");
            foreach (var r in report.Results)
            {
                w.WriteStartObject();
                w.WriteString("id", r.Id);
                w.WriteBoolean("passed", r.Passed);
                w.WriteStartObject("scores");
                foreach (var (k, v) in r.Scores)
                {
                    w.WriteNumber(k, v);
                }
                w.WriteEndObject();
                if (r.Error.Length == 0)
                {
                    w.WriteNull("error");
                }
                else
                {
                    w.WriteString("error", r.Error);
                }
                w.WriteEndObject();
            }
            w.WriteEndArray();
            w.WriteEndObject();
        }
        return System.Text.Encoding.UTF8.GetString(stream.ToArray());
    }

    /// <summary>Runs the whole thing and returns the exit code; <paramref name="solver"/> null is the stub.</summary>
    public static int Run(string[] args, TextWriter stdout, TextWriter stderr, ISolver? solver = null)
    {
        ArgumentNullException.ThrowIfNull(args);
        ArgumentNullException.ThrowIfNull(stdout);
        ArgumentNullException.ThrowIfNull(stderr);
        string? dataset = null;
        string? barPath = null;
        var asJson = false;
        var show = false;
        for (var i = 0; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "--dataset" or "--bar":
                    if (i + 1 >= args.Length)
                    {
                        stderr.WriteLine($"usage error: {args[i]} needs a value");
                        return 2;
                    }
                    if (args[i] == "--dataset")
                    {
                        dataset = args[++i];
                    }
                    else
                    {
                        barPath = args[++i];
                    }
                    break;
                case "--json":
                    asJson = true;
                    break;
                case "--show":
                    show = true;
                    break;
                default:
                    stderr.WriteLine($"usage error: unknown argument {args[i]}");
                    return 2;
            }
        }
        solver ??= new StubSolver();
        IReadOnlyList<Case> cases;
        Bar bar;
        try
        {
            cases = Dataset.Load(dataset ?? Dataset.Locate(Dataset.DefaultDataset));
            bar = Bar.Load(barPath ?? Dataset.Locate(Dataset.DefaultBar), Scorers.Names());
        }
        catch (SuiteException e)
        {
            stderr.WriteLine($"suite error: {e.Message}");
            return 2;
        }
        var report = Runner.Evaluate(cases, solver);
        var missed = bar.Missed(report);
        if (asJson)
        {
            stdout.WriteLine(ReportJson(report, missed));
        }
        else
        {
            var label = report.Stub ? "STUB solver, not a model: " : "";
            stdout.WriteLine($"{label}{report.Solver}: {report.Passes}/{report.Results.Count} cases pass");
            foreach (var n in Scorers.Names())
            {
                stdout.WriteLine($"  {n}: mean {F3(report.Mean(n))} (bar {F3(bar.Scorers[n])})");
            }
            stdout.WriteLine(
                $"  pass rate {F3(report.PassRate)} (bar {F3(bar.MinPassRate)}), errors {report.Errors} (at most {bar.MaxErrors})");
            foreach (var r in report.Results.Where(r => !r.Passed))
            {
                var why = r.Error.Length > 0 ? r.Error : string.Join(", ", Scorers.Names().Where(n => r.Scores[n] == 0));
                stdout.WriteLine($"  FAILED {r.Id}: {why}");
            }
            stdout.WriteLine(missed.Count == 0 ? "BAR MET" : "BAR MISSED: " + string.Join("; ", missed));
        }
        if (show)
        {
            var byId = cases.ToDictionary(c => c.Id);
            foreach (var r in report.Results.Where(r => !r.Passed))
            {
                var c = byId[r.Id];
                stderr.WriteLine($"  {r.Id}: input={JsonSerializer.Serialize(c.Input)} expected={JsonSerializer.Serialize(c.Expected)} answer={JsonSerializer.Serialize(r.Answer)}");
            }
        }
        return missed.Count == 0 ? 0 : 1;
    }
}
