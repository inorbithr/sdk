using System;
using System.Collections.Generic;
using System.Linq;

namespace @@MODULE@@;

/// <summary>
/// A scorer takes the case and the solver's answer and returns 0 or 1. It is a pure function: the
/// same case and answer give the same score, with no clock, no randomness and no network, so a
/// change in the score is a change in the solver. (A model that grades another model's answer is a
/// solver of its own: label it a stub in tests and give it a bar of its own.)
/// </summary>
public static class Scorers
{
    /// <summary>Every scorer, by name, in the order the report lists them.</summary>
    public static readonly IReadOnlyList<(string Name, Func<Case, string, int> Score)> All =
    [
        ("exact", Exact),
        ("format", Format),
    ];

    /// <summary>The names of every scorer.</summary>
    public static IReadOnlyList<string> Names() => [.. All.Select(s => s.Name)];

    /// <summary>The answer is the expected label, ignoring case and surrounding space.</summary>
    public static int Exact(Case c, string answer)
    {
        ArgumentNullException.ThrowIfNull(c);
        return string.Equals(answer.Trim(), c.Expected.Trim(), StringComparison.OrdinalIgnoreCase) ? 1 : 0;
    }

    /// <summary>
    /// The answer is one bare label from the list and nothing else (no sentence, no full stop). A
    /// model that is right but chatty breaks the program that reads its answer, so it fails.
    /// </summary>
    public static int Format(Case c, string answer) => StubSolver.Labels.Any(l => l == answer) ? 1 : 0;
}
