using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Xunit;

namespace @@MODULE@@.Tests;

/// <summary>
/// The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an answer
/// is wrong, and the harness refuses a broken dataset or bar instead of grading less.
/// </summary>
public sealed class SuiteTests
{
    private sealed class FixedSolver(string name, Func<Case, string> solve) : ISolver
    {
        public string Name => name;

        public bool Stub => true;

        public string Solve(Case c) => solve(c);
    }

    private static (int Code, string Out, string Err) Run(ISolver? solver, params string[] args)
    {
        using var o = new StringWriter();
        using var e = new StringWriter();
        var code = Suite.Run(args, o, e, solver);
        return (code, o.ToString(), e.ToString());
    }

    private static (int Code, string Out, string Err) Run(params string[] args) => Run(null, args);

    private static string Tmp(string name, string text)
    {
        var dir = Directory.CreateTempSubdirectory("evals-").FullName;
        var file = Path.Combine(dir, name);
        File.WriteAllText(file, text);
        return file;
    }

    private static IReadOnlyList<Case> Cases() => Dataset.Load(Dataset.Locate(Dataset.DefaultDataset));

    private static Bar TheBar() => Bar.Load(Dataset.Locate(Dataset.DefaultBar), Scorers.Names());

    [Fact]
    public void The_fixture_meets_its_bar_with_no_network()
    {
        // Nothing here can reach a network: the project has no dependency and the stub is local rules.
        var r = Run();
        Assert.True(r.Code == 0, r.Out + r.Err);
        Assert.Contains("BAR MET", r.Out, StringComparison.Ordinal);
    }

    [Fact]
    public void The_dataset_is_a_dozen_labelled_cases_covering_every_label()
    {
        var cases = Cases();
        Assert.Equal(12, cases.Count);
        foreach (var label in StubSolver.Labels)
        {
            Assert.Contains(cases, c => c.Expected == label);
        }
    }

    [Fact]
    public void The_report_says_stub_and_holds_ids_and_scores_never_inputs()
    {
        var r = Run("--json");
        Assert.Equal(0, r.Code);
        using var doc = System.Text.Json.JsonDocument.Parse(r.Out);
        var root = doc.RootElement;
        Assert.True(root.GetProperty("stub").GetBoolean());
        Assert.True(root.GetProperty("bar_met").GetBoolean());
        Assert.Equal(1.0, root.GetProperty("pass_rate").GetDouble());
        Assert.StartsWith("stub-", root.GetProperty("solver").GetString(), StringComparison.Ordinal);
        foreach (var c in Cases())
        {
            Assert.DoesNotContain(c.Input, r.Out, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void Two_runs_give_the_same_report() => Assert.Equal(Run("--json"), Run("--json"));

    [Fact]
    public void Lowering_one_expected_answer_fails_the_suite_and_names_the_case()
    {
        var lines = File.ReadAllLines(Dataset.Locate(Dataset.DefaultDataset)).ToList();
        var first = lines[0].Replace("\"expected\": \"billing\"", "\"expected\": \"other\"", StringComparison.Ordinal);
        Assert.NotEqual(lines[0], first);
        lines[0] = first;
        var file = Tmp("cases.jsonl", string.Join('\n', lines) + "\n");
        var r = Run("--dataset", file);
        Assert.True(r.Code == 1, r.Out);
        Assert.Contains("BAR MISSED", r.Out, StringComparison.Ordinal);
        Assert.Contains("FAILED billing-double-charge", r.Out, StringComparison.Ordinal);
    }

    [Fact]
    public void A_chatty_answer_fails_the_format_scorer_even_when_it_is_right()
    {
        var stub = new StubSolver();
        var chatty = new FixedSolver("stub-chatty", c => $"The label is {stub.Solve(c)}.");
        var report = Runner.Evaluate(Cases(), chatty);
        Assert.Equal(0.0, report.Mean("format"));
        Assert.NotEmpty(TheBar().Missed(report));
    }

    [Fact]
    public void A_solver_that_throws_is_an_error_and_never_a_pass()
    {
        var broken = new FixedSolver("stub-broken", _ => throw new InvalidOperationException("provider down"));
        var report = Runner.Evaluate(Cases(), broken);
        Assert.Equal(12, report.Errors);
        Assert.Equal(0.0, report.PassRate);
        Assert.Contains(TheBar().Missed(report), m => m.Contains("error", StringComparison.Ordinal));
        Assert.Equal(1, Run(broken).Code);
    }

    [Fact]
    public void A_broken_dataset_stops_the_run()
    {
        const string Row = "{\"id\": \"a\", \"input\": \"i\", \"expected\": \"bug\"}\n";
        var broken = new Dictionary<string, string>
        {
            ["twice"] = Row + Row,
            ["input"] = "{\"id\": \"a\", \"input\": \"\", \"expected\": \"bug\"}\n",
            ["no cases"] = "\n",
            ["not JSON"] = "not json\n",
        };
        foreach (var (why, text) in broken)
        {
            var r = Run("--dataset", Tmp("c.jsonl", text));
            Assert.True(r.Code == 2, why);
            Assert.True(r.Err.Contains(why, StringComparison.Ordinal), $"{why}: {r.Err}");
        }
    }

    [Fact]
    public void A_scorer_with_no_bar_stops_the_run()
    {
        var file = Tmp("t.toml", "[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n");
        var r = Run("--bar", file);
        Assert.Equal(2, r.Code);
        Assert.Contains("no bar for the scorer(s) format", r.Err, StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("[bar]\nmin_pass_rate = high\n")]
    [InlineData("min_pass_rate = 1\n")]
    [InlineData("[bar]\nmin_pass_rate = 2\nmax_errors = 0\n[bar.scorers]\nexact = 1\nformat = 1\n")]
    public void A_bar_that_is_not_understood_stops_the_run(string text) =>
        Assert.Equal(2, Run("--bar", Tmp("t.toml", text)).Code);

    [Fact]
    public void A_missing_bar_or_dataset_stops_the_run()
    {
        Assert.Equal(2, Run("--bar", "/nonexistent/t.toml").Code);
        Assert.Equal(2, Run("--dataset", "/nonexistent/c.jsonl").Code);
    }

    [Fact]
    public void The_scorers()
    {
        var billing = Cases()[0];
        Assert.Equal(1, Scorers.Exact(billing, " Billing\n"));
        Assert.Equal(0, Scorers.Exact(billing, "bug"));
        Assert.Equal(1, Scorers.Format(billing, "billing"));
        foreach (var bad in new[] { "Billing", "billing.", "billing, bug", "" })
        {
            Assert.Equal(0, Scorers.Format(billing, bad));
        }
    }

    [Fact]
    public void Every_scorer_has_a_bar() =>
        Assert.Equal(Scorers.Names().Order(StringComparer.Ordinal), TheBar().Scorers.Keys.Order(StringComparer.Ordinal));
}
