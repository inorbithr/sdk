using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;

namespace @@MODULE@@;

/// <summary>The dataset or the bar is wrong. The run stops and exits 2: nothing was graded.</summary>
public sealed class SuiteException : Exception
{
    /// <summary>Says what is wrong.</summary>
    public SuiteException(string message) : base(message) { }
}

/// <summary>One line of the dataset: an id, an input and the expected answer.</summary>
public sealed record Case(string Id, string Input, string Expected);

/// <summary>The dataset: JSON Lines, one case per line, loaded strictly.</summary>
public static class Dataset
{
    /// <summary>The default dataset, relative to the project folder.</summary>
    public const string DefaultDataset = "datasets/tickets.jsonl";

    /// <summary>The default bar, relative to the project folder.</summary>
    public const string DefaultBar = "thresholds.toml";

    /// <summary>
    /// A project file by its path from the project folder: where you are standing when it is there,
    /// else found by walking up from the program (the tests run from the build folder).
    /// </summary>
    public static string Locate(string relative)
    {
        if (File.Exists(relative))
        {
            return relative;
        }
        for (var dir = new DirectoryInfo(AppContext.BaseDirectory); dir is not null; dir = dir.Parent)
        {
            var candidate = Path.Combine(dir.FullName, relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }
        }
        return relative;
    }

    /// <summary>The reason an I/O error carries, without the OS code.</summary>
    internal static string Reason(Exception e) => e switch
    {
        FileNotFoundException or DirectoryNotFoundException => "no such file or directory",
        UnauthorizedAccessException => "permission denied",
        _ => e.Message,
    };

    /// <summary>Reads the dataset. A broken one fails loudly instead of quietly scoring fewer cases.</summary>
    public static IReadOnlyList<Case> Load(string path)
    {
        string text;
        try
        {
            text = File.ReadAllText(path);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            throw new SuiteException($"cannot read the dataset {path}: {Reason(e)}");
        }
        var name = Path.GetFileName(path);
        var cases = new List<Case>();
        var seen = new HashSet<string>();
        var lines = text.Split('\n');
        for (var i = 0; i < lines.Length; i++)
        {
            var number = i + 1;
            if (string.IsNullOrWhiteSpace(lines[i]))
            {
                continue;
            }
            JsonDocument doc;
            try
            {
                doc = JsonDocument.Parse(lines[i]);
            }
            catch (JsonException e)
            {
                throw new SuiteException($"{name} line {number}: not JSON ({e.Message})");
            }
            using (doc)
            {
                var root = doc.RootElement;
                if (root.ValueKind != JsonValueKind.Object)
                {
                    throw new SuiteException($"{name} line {number}: expected an object");
                }
                string Field(string key)
                {
                    if (root.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String
                        && !string.IsNullOrWhiteSpace(v.GetString()))
                    {
                        return v.GetString()!;
                    }
                    throw new SuiteException($"{name} line {number}: `{key}` must be a non-empty string");
                }
                var (id, input, expected) = (Field("id"), Field("input"), Field("expected"));
                if (!seen.Add(id))
                {
                    throw new SuiteException($"{name} line {number}: the id `{id}` is used twice");
                }
                cases.Add(new Case(id, input, expected));
            }
        }
        if (cases.Count == 0)
        {
            throw new SuiteException($"{name} has no cases: an empty suite passes nothing");
        }
        return cases;
    }
}
