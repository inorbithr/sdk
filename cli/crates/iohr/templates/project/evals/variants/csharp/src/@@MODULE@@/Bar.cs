using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;

namespace @@MODULE@@;

/// <summary>What the suite must reach to pass, read from thresholds.toml.</summary>
public sealed partial record Bar(double MinPassRate, int MaxErrors, IReadOnlyDictionary<string, double> Scorers)
{
    [GeneratedRegex(@"^\[([A-Za-z0-9_.]+)\]$")]
    private static partial Regex TableLine();

    [GeneratedRegex(@"^([A-Za-z0-9_]+)\s*=\s*([+-]?[0-9]+(\.[0-9]+)?)$")]
    private static partial Regex KeyLine();

    private static string F3(double x) => x.ToString("F3", CultureInfo.InvariantCulture);

    /// <summary>
    /// Reads the small part of TOML the bar uses: <c>[table]</c> lines, <c>key = number</c> lines and
    /// <c>#</c> comments. The base class library has no TOML reader, and the bar needs nothing more,
    /// so anything else is refused rather than guessed at.
    /// </summary>
    private static Dictionary<string, Dictionary<string, string>> Tables(string name, string text)
    {
        var tables = new Dictionary<string, Dictionary<string, string>>();
        Dictionary<string, string>? current = null;
        var lines = text.Split('\n');
        for (var i = 0; i < lines.Length; i++)
        {
            var line = lines[i];
            var hash = line.IndexOf('#', StringComparison.Ordinal);
            if (hash >= 0)
            {
                line = line[..hash];
            }
            line = line.Trim();
            if (line.Length == 0)
            {
                continue;
            }
            var table = TableLine().Match(line);
            if (table.Success)
            {
                var title = table.Groups[1].Value;
                if (tables.ContainsKey(title))
                {
                    throw new SuiteException($"{name}: not valid TOML (line {i + 1}: table [{title}] twice)");
                }
                current = [];
                tables[title] = current;
                continue;
            }
            var key = KeyLine().Match(line);
            if (!key.Success || current is null)
            {
                throw new SuiteException(
                    $"{name}: not valid TOML (line {i + 1}: only `[table]` and `key = number` are read here)");
            }
            if (!current.TryAdd(key.Groups[1].Value, key.Groups[2].Value))
            {
                throw new SuiteException($"{name}: not valid TOML (line {i + 1}: key {key.Groups[1].Value} twice)");
            }
        }
        return tables;
    }

    /// <summary>Reads the bar and checks it guards every scorer.</summary>
    public static Bar Load(string path, IReadOnlyList<string> names)
    {
        ArgumentNullException.ThrowIfNull(names);
        var file = Path.GetFileName(path);
        string text;
        try
        {
            text = File.ReadAllText(path);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            throw new SuiteException($"cannot read the bar {path}: {Dataset.Reason(e)}");
        }
        var tables = Tables(file, text);
        if (!tables.TryGetValue("bar", out var bar))
        {
            throw new SuiteException($"{file}: the [bar] table is missing");
        }
        if (!bar.TryGetValue("min_pass_rate", out var rawRate) || !bar.TryGetValue("max_errors", out var rawErrors)
            || !tables.TryGetValue("bar.scorers", out var per) || rawErrors.Contains('.', StringComparison.Ordinal))
        {
            throw new SuiteException($"{file}: [bar] needs min_pass_rate, max_errors and [bar.scorers]");
        }
        var rate = double.Parse(rawRate, CultureInfo.InvariantCulture);
        var range = $"{file}: every rate is a number from 0 to 1";
        if (rate is < 0 or > 1)
        {
            throw new SuiteException(range);
        }
        var scorers = new SortedDictionary<string, double>(StringComparer.Ordinal);
        foreach (var (k, v) in per)
        {
            var f = double.Parse(v, CultureInfo.InvariantCulture);
            if (f is < 0 or > 1)
            {
                throw new SuiteException(range);
            }
            scorers[k] = f;
        }
        if (!int.TryParse(rawErrors, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var errors) || errors < 0)
        {
            throw new SuiteException($"{file}: max_errors cannot be negative");
        }
        var unguarded = names.Where(n => !scorers.ContainsKey(n)).Order(StringComparer.Ordinal).ToList();
        var unknown = scorers.Keys.Where(k => !names.Contains(k)).ToList();
        if (unguarded.Count > 0)
        {
            throw new SuiteException(
                $"{file}: no bar for the scorer(s) {string.Join(", ", unguarded)}; a scorer with no bar guards nothing");
        }
        if (unknown.Count > 0)
        {
            throw new SuiteException($"{file}: a bar for {string.Join(", ", unknown)}, which is not a scorer");
        }
        return new Bar(rate, errors, scorers);
    }

    /// <summary>Every line of the bar the report does not reach; empty means the bar is met.</summary>
    public IReadOnlyList<string> Missed(Report report)
    {
        ArgumentNullException.ThrowIfNull(report);
        var missed = new List<string>();
        if (report.PassRate < MinPassRate)
        {
            missed.Add($"pass rate {F3(report.PassRate)} is below {F3(MinPassRate)}");
        }
        if (report.Errors > MaxErrors)
        {
            missed.Add($"{report.Errors} error(s), at most {MaxErrors} allowed");
        }
        foreach (var (name, min) in Scorers)
        {
            var mean = report.Mean(name);
            if (mean < min)
            {
                missed.Add($"scorer {name}: mean {F3(mean)} is below {F3(min)}");
            }
        }
        return missed;
    }
}
