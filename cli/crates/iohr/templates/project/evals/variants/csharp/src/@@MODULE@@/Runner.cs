using System;
using System.Collections.Generic;
using System.Linq;

namespace @@MODULE@@;

/// <summary>One case, graded. <see cref="Error"/> is empty unless the solver failed.</summary>
public sealed record Result(string Id, string Answer, IReadOnlyDictionary<string, int> Scores, string Error)
{
    /// <summary>Every scorer gave 1 and the solver did not fail.</summary>
    public bool Passed => Error.Length == 0 && Scores.Values.All(s => s == 1);
}

/// <summary>Every result and which solver made them.</summary>
public sealed record Report(string Solver, bool Stub, IReadOnlyList<Result> Results)
{
    /// <summary>How many cases pass.</summary>
    public int Passes => Results.Count(r => r.Passed);

    /// <summary>The share of cases that pass.</summary>
    public double PassRate => (double)Passes / Results.Count;

    /// <summary>How many cases the solver failed on.</summary>
    public int Errors => Results.Count(r => r.Error.Length > 0);

    /// <summary>One scorer's mean over the cases.</summary>
    public double Mean(string scorer) => Results.Average(r => r.Scores.GetValueOrDefault(scorer));
}

/// <summary>The runner: the solver over the cases, every answer scored.</summary>
public static class Runner
{
    /// <summary>
    /// Runs the solver over the cases and scores each answer. A solver that fails is an error case
    /// with every score 0, never a pass. The error kept is the exception's type, never its message:
    /// a provider's message can echo the input.
    /// </summary>
    public static Report Evaluate(IReadOnlyList<Case> cases, ISolver solver)
    {
        ArgumentNullException.ThrowIfNull(cases);
        ArgumentNullException.ThrowIfNull(solver);
        var results = new List<Result>();
        foreach (var c in cases)
        {
            var answer = "";
            var error = "";
            try
            {
                answer = solver.Solve(c);
            }
            catch (Exception e)
            {
                error = e.GetType().Name;
            }
            var scores = Scorers.All.ToDictionary(s => s.Name, s => error.Length == 0 ? s.Score(c, answer) : 0);
            results.Add(new Result(c.Id, answer, scores, error));
        }
        return new Report(solver.Name, solver.Stub, results);
    }
}
