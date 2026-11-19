using System;
using System.Collections.Generic;

namespace @@MODULE@@;

/// <summary>
/// The thing under test: it turns a case's input into an answer. The one here
/// (<see cref="StubSolver"/>) is a STUB (a fixed list of keyword rules), labelled as one everywhere
/// it shows up (<c>stub: true</c> in the report, a <c>stub-</c> name). It exists so the suite runs
/// offline, is free and gives the same answer every time. To grade a real model, write a class with
/// these three members that calls your provider, sign in the way your provider's subscription
/// allows and keep the credential in your secret store, never in this folder. A solver that throws
/// is counted as an error, never a pass.
/// </summary>
public interface ISolver
{
    /// <summary>The solver's name, in the report.</summary>
    string Name { get; }

    /// <summary>True for anything that is not a real model.</summary>
    bool Stub { get; }

    /// <summary>The answer for one case.</summary>
    string Solve(Case c);
}

/// <summary>Keyword rules. Not a model: nothing is learned, nothing is called.</summary>
public sealed class StubSolver : ISolver
{
    /// <summary>The answers the suite knows, in the order the rules list them.</summary>
    public static readonly IReadOnlyList<string> Labels = ["billing", "bug", "feature_request", "account", "other"];

    /// <summary>First match wins, in this order.</summary>
    private static readonly (string Label, string[] Words)[] Rules =
    [
        ("billing", ["charged", "refund", "invoice", "payment", "card"]),
        ("bug", ["crash", "error", "blank screen"]),
        ("feature_request", ["would be great", "add an option", "feature"]),
        ("account", ["password", "account"]),
    ];

    /// <inheritdoc/>
    public string Name => "stub-keyword-rules-v1";

    /// <inheritdoc/>
    public bool Stub => true;

    /// <inheritdoc/>
    public string Solve(Case c)
    {
        ArgumentNullException.ThrowIfNull(c);
        var text = c.Input.ToLowerInvariant();
        foreach (var (label, words) in Rules)
        {
            foreach (var word in words)
            {
                if (text.Contains(word, StringComparison.Ordinal))
                {
                    return label;
                }
            }
        }
        return "other";
    }
}
