using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Microsoft.Extensions.Logging;
using YamlDotNet.Core;
using YamlDotNet.RepresentationModel;

namespace InOrbit.Sdk.Tests.Conformance;

/// <summary>What the driver and the vectors share: YAML as JSON, subsets, header matchers, options by catalogue name, a log sink.</summary>
internal static class Support
{
    /// <summary>The repository's root.</summary>
    internal static string Root { get; } = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "../../../../../.."));

    /// <summary>A YAML file as JSON, with the YAML 1.2 core schema for plain scalars.</summary>
    internal static JsonNode? Yaml(string path)
    {
        var stream = new YamlStream();
        using var reader = new StreamReader(path);
        stream.Load(reader);
        return ToJson(stream.Documents[0].RootNode);
    }

    private static JsonNode? ToJson(YamlNode node) => node switch
    {
        YamlMappingNode map => new JsonObject(map.Children.Select(kv => KeyValuePair.Create(((YamlScalarNode)kv.Key).Value ?? string.Empty, ToJson(kv.Value)))),
        YamlSequenceNode seq => new JsonArray(seq.Children.Select(ToJson).ToArray()),
        YamlScalarNode s when s.Style is ScalarStyle.Plain => Plain(s.Value ?? string.Empty),
        YamlScalarNode s => JsonValue.Create(s.Value ?? string.Empty),
        _ => null,
    };

    private static JsonValue? Plain(string v)
    {
        if (v is "" or "~" or "null")
        {
            return null;
        }

        if (v is "true" or "false")
        {
            return JsonValue.Create(v == "true");
        }

        if (Regex.IsMatch(v, "^-?[0-9]+$") && long.TryParse(v, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var n))
        {
            return JsonValue.Create(n);
        }

        return JsonValue.Create(v);
    }

    internal static string Slash(string s) => s.Replace('\\', '/');

    /// <summary>Where <paramref name="want"/> is not a subset of <paramref name="got"/>, or <see langword="null"/>: objects by key, arrays by position with the same length, paths with <c>\</c> as <c>/</c>.</summary>
    internal static string? Subset(JsonNode? want, JsonNode? got, string at = "")
    {
        switch (want)
        {
            case JsonObject wo:
                if (got is not JsonObject go)
                {
                    return $"{at}: want {wo.ToJsonString()}, got {got?.ToJsonString() ?? "null"}";
                }

                foreach (var (k, v) in wo)
                {
                    if (Subset(v, go[k], $"{at}.{k}") is { } p)
                    {
                        return p;
                    }
                }

                return null;
            case JsonArray wa:
                if (got is not JsonArray ga || ga.Count != wa.Count)
                {
                    return $"{at}: want {wa.ToJsonString()}, got {got?.ToJsonString() ?? "null"}";
                }

                for (var i = 0; i < wa.Count; i++)
                {
                    if (Subset(wa[i], ga[i], $"{at}[{i}]") is { } p)
                    {
                        return p;
                    }
                }

                return null;
            case null:
                return got is null ? null : $"{at}: want null, got {got.ToJsonString()}";
            default:
                if (got is null)
                {
                    return $"{at}: want {want.ToJsonString()}, got null";
                }

                var w = want.GetValueKind() == JsonValueKind.String ? Slash(want.GetValue<string>()) : want.ToJsonString();
                var g = got.GetValueKind() == JsonValueKind.String ? Slash(got.GetValue<string>()) : got.ToJsonString();
                return w == g ? null : $"{at}: want {want.ToJsonString()}, got {got.ToJsonString()}";
        }
    }

    /// <summary>Header matchers: a literal, <c>*</c>, <c>$name</c> (captured, then equal) or <c>~regex</c> (the whole value).</summary>
    internal static bool Matches(string want, string? got, Dictionary<string, string> captures)
    {
        if (got is null)
        {
            return false;
        }

        if (want == "*")
        {
            return true;
        }

        if (want.StartsWith('$'))
        {
            if (!captures.TryGetValue(want, out var seen))
            {
                captures[want] = got;
                return true;
            }

            return seen == got;
        }

        return want.StartsWith('~') ? Regex.IsMatch(got, $"^(?:{want[1..]})$") : want == got;
    }

    /// <summary>Client options from settings by catalogue name, as a vector's <c>code</c> gives them.</summary>
    internal static ClientOptions Options(JsonObject? code)
    {
        var options = new ClientOptions();
        foreach (var (name, value) in code ?? [])
        {
            if (name == "http_client")
            {
                options = options with { HttpClient = new System.Net.Http.HttpClient() };
                continue;
            }

            var pascal = string.Concat(name.Split('_').Select(p => char.ToUpperInvariant(p[0]) + p[1..]));
            var prop = typeof(ClientOptions).GetProperty(pascal, BindingFlags.Public | BindingFlags.Instance)
                ?? throw new InvalidOperationException($"no option {pascal} for {name}");
            var type = Nullable.GetUnderlyingType(prop.PropertyType) ?? prop.PropertyType;
            object? v = type switch
            {
                _ when type == typeof(TimeSpan) => Settings.ParseDuration(value!.GetValue<string>()),
                _ when type == typeof(int) => (int)value!.GetValue<long>(),
                _ when type == typeof(bool) => value!.GetValue<bool>(),
                _ when type == typeof(Uri) => new Uri(value!.GetValue<string>()),
                _ when type == typeof(string) => value!.GetValue<string>(),
                _ when type == typeof(IReadOnlyList<string>) => value is JsonArray a
                    ? a.Select(x => x!.GetValue<string>()).ToArray()
                    : value!.GetValue<string>().Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries),
                _ => throw new InvalidOperationException($"no conversion for {name}"),
            };
            prop.SetValue(options, v);
        }

        return options;
    }

    /// <summary>A fake <c>iohr</c> on a <c>PATH</c> of its own, with the host's names: present, never run.</summary>
    internal static string FakeIohr(string dir)
    {
        var bin = Path.Combine(dir, "bin");
        Directory.CreateDirectory(bin);
        var file = Path.Combine(bin, OperatingSystem.IsWindows() ? "iohr.exe" : "iohr");
        File.WriteAllText(file, "#!/bin/sh\nexit 1\n");
        if (!OperatingSystem.IsWindows())
        {
            File.SetUnixFileMode(file, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);
        }

        return bin;
    }
}

/// <summary>Keeps every record the SDK logs.</summary>
internal sealed class CaptureLogs : ILoggerFactory, ILogger
{
    private readonly object _gate = new();
    private readonly List<JsonObject> _records = [];

    internal IReadOnlyList<JsonObject> Records
    {
        get
        {
            lock (_gate)
            {
                return [.. _records];
            }
        }
    }

    public ILogger CreateLogger(string categoryName) => this;

    public void AddProvider(ILoggerProvider provider)
    {
    }

    public void Dispose()
    {
    }

    public IDisposable? BeginScope<TState>(TState state)
        where TState : notnull => null;

    public bool IsEnabled(LogLevel logLevel) => true;

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
    {
        if (state is not LogRecord record)
        {
            return;
        }

        var o = new JsonObject();
        foreach (var (k, v) in record)
        {
            o[k] = v is null ? null : JsonSerializer.SerializeToNode(v, v.GetType());
        }

        lock (_gate)
        {
            _records.Add(o);
        }
    }
}
