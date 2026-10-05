using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.Diagnostics.Metrics;
using System.Linq;
using System.Text;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace InOrbit.Sdk;

/// <summary>
/// One structured log record (docs/config.md section 7.9): an <see cref="Event"/> and its fields, with
/// the same field names in every language. It is the state passed to <see cref="ILogger"/>, so a
/// structured sink sees every field by name.
/// </summary>
public sealed class LogRecord : IReadOnlyList<KeyValuePair<string, object?>>
{
    private readonly List<KeyValuePair<string, object?>> _fields;

    /// <summary>A record of <paramref name="fields"/>, which hold <c>event</c>.</summary>
    /// <param name="fields">The fields, in order.</param>
    public LogRecord(IEnumerable<KeyValuePair<string, object?>> fields)
    {
        ArgumentNullException.ThrowIfNull(fields);
        _fields = fields.ToList();
    }

    /// <summary>What happened: <c>request</c>, <c>response</c>, <c>call</c>, <c>retry</c>, <c>call_failed</c>...</summary>
    public string Event => this["event"] as string ?? string.Empty;

    /// <inheritdoc/>
    public int Count => _fields.Count;

    /// <inheritdoc/>
    public KeyValuePair<string, object?> this[int index] => _fields[index];

    /// <summary>A field's value, or <see langword="null"/>.</summary>
    /// <param name="name">The field's name.</param>
    /// <returns>The value.</returns>
    public object? this[string name] => _fields.FirstOrDefault(f => f.Key == name).Value;

    /// <inheritdoc/>
    public IEnumerator<KeyValuePair<string, object?>> GetEnumerator() => _fields.GetEnumerator();

    /// <summary>The record as one line: <c>inorbithr request operation=me method=GET ...</c>.</summary>
    /// <returns>The line.</returns>
    public override string ToString()
    {
        var text = new StringBuilder("inorbithr ").Append(Event);
        foreach (var (k, v) in _fields)
        {
            if (k != "event" && v is not null)
            {
                text.Append(' ').Append(k).Append('=').Append(v is IDictionary<string, string> d ? "{" + string.Join(", ", d.Select(x => $"{x.Key}: {x.Value}")) + "}" : v);
            }
        }

        return text.ToString();
    }

    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();
}

/// <summary>The SDK's records, by allowlist, to an <see cref="ILogger"/> of category <c>InOrbit.Sdk</c>.</summary>
internal sealed class Log
{
    internal static readonly string[] NeverLogged = ["authorization", "proxy-authorization", "cookie", "set-cookie"];

    private static readonly string[] RequestAllowlist = ["accept", "content-type", "content-length", "user-agent", "x-request-id", "traceparent", "idempotency-key"];

    private static readonly string[] ResponseAllowlist =
    [
        "content-type", "content-length", "date", "retry-after", "x-request-id", "idempotency-replayed",
        "x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset", "ratelimit", "ratelimit-policy",
    ];

    private readonly LogLevel _level;
    private readonly ILogger _logger;
    private readonly Func<LogRecord, LogRecord?>? _redact;
    private readonly string? _profile;
    private readonly HashSet<string> _request;
    private readonly HashSet<string> _response;

    internal Log(LogLevel level, ILoggerFactory? factory, Func<LogRecord, LogRecord?>? redact, string? profile, bool headers, IEnumerable<string>? allow)
    {
        _level = level;
        _logger = factory?.CreateLogger("InOrbit.Sdk") ?? NullLogger.Instance;
        _redact = redact;
        _profile = profile;
        Headers = headers;
        var extra = (allow ?? []).Select(h => h.ToLowerInvariant()).Where(h => Array.IndexOf(NeverLogged, h) < 0).ToArray();
        _request = [.. RequestAllowlist, .. extra];
        _response = [.. ResponseAllowlist, .. extra];
    }

    internal static Log Off { get; } = new(LogLevel.None, null, null, null, false, null);

    /// <summary>Whether header values are logged at <c>debug</c>.</summary>
    internal bool Headers { get; }

    /// <summary>The <c>log</c> setting's value as a level.</summary>
    internal static LogLevel Level(string? setting) => setting switch
    {
        "error" => LogLevel.Error,
        "warn" => LogLevel.Warning,
        "info" => LogLevel.Information,
        "debug" => LogLevel.Debug,
        _ => LogLevel.None,
    };

    /// <summary>A level as the <c>log</c> setting names it.</summary>
    internal static string Name(LogLevel level) => level switch
    {
        LogLevel.Error or LogLevel.Critical => "error",
        LogLevel.Warning => "warn",
        LogLevel.Information => "info",
        LogLevel.Debug or LogLevel.Trace => "debug",
        _ => "off",
    };

    internal bool On(LogLevel level) => _level != LogLevel.None && level >= _level;

    /// <summary>Headers by name: allowlisted values, every other one <c>REDACTED</c>.</summary>
    internal Dictionary<string, string> Show(IEnumerable<KeyValuePair<string, string>> headers, bool response)
    {
        var allow = response ? _response : _request;
        var shown = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var (name, value) in headers)
        {
            var n = name.ToLowerInvariant();
            shown[n] = allow.Contains(n) && Array.IndexOf(NeverLogged, n) < 0 ? value : "REDACTED";
        }

        return shown;
    }

    internal void Emit(LogLevel level, params (string Key, object? Value)[] fields)
    {
        if (!On(level))
        {
            return;
        }

        var list = fields.Where(f => f.Value is not null).Select(f => new KeyValuePair<string, object?>(f.Key, f.Value)).ToList();
        if (_profile is not null)
        {
            list.Add(new("profile", _profile));
        }

        LogRecord? record = new(list);
        if (_redact is not null)
        {
            try
            {
                record = _redact(record);
            }
#pragma warning disable CA1031 // A failing redactor drops the record; it never fails a call.
            catch (Exception)
#pragma warning restore CA1031
            {
                return;
            }
        }

        if (record is null)
        {
            return;
        }

        try
        {
            _logger.Log(level, new EventId(0, record.Event), record, null, static (r, _) => r.ToString());
        }
#pragma warning disable CA1031 // A failing sink never fails a call.
        catch (Exception)
#pragma warning restore CA1031
        {
        }
    }
}

/// <summary>
/// Spans and metrics (docs/config.md section 7.10) through <see cref="ActivitySource"/> and
/// <see cref="Meter"/> named <c>InOrbit.Sdk</c>: inert until a listener subscribes, as OpenTelemetry
/// .NET does with <c>AddSource("InOrbit.Sdk")</c> and <c>AddMeter("InOrbit.Sdk")</c>.
/// </summary>
internal static class Telemetry
{
    internal const string Name = "InOrbit.Sdk";

    internal static readonly ActivitySource Source = new(Name, SdkVersion.Value);

    internal static readonly Meter Meter = new(Name, SdkVersion.Value);

    internal static readonly Histogram<double> RequestDuration = Meter.CreateHistogram<double>(
        "http.client.request.duration", "s", "Duration of HTTP client requests.");

    internal static readonly Histogram<double> CallDuration = Meter.CreateHistogram<double>(
        "inorbit.client.call.duration", "s", "Duration of one call, every attempt and wait included.");

    internal static readonly Counter<long> Retries = Meter.CreateCounter<long>(
        "inorbit.client.retries", "{retry}", "Retries made.");

    internal static readonly Counter<long> Exchanges = Meter.CreateCounter<long>(
        "inorbit.client.token.exchanges", "{exchange}", "Token exchanges and refreshes.");

    /// <summary>The W3C <c>traceparent</c> of <paramref name="activity"/>.</summary>
    internal static string Traceparent(Activity activity) =>
        $"00-{activity.TraceId.ToHexString()}-{activity.SpanId.ToHexString()}-{((activity.ActivityTraceFlags & ActivityTraceFlags.Recorded) != 0 ? "01" : "00")}";

    /// <summary>The URL with every query value replaced by <c>REDACTED</c>.</summary>
    internal static string RedactedUrl(Uri url)
    {
        if (url.Query.Length <= 1)
        {
            return url.AbsoluteUri;
        }

        var names = url.Query[1..].Split('&').Select(p => p.Split('=')[0]).Distinct(StringComparer.Ordinal);
        return url.GetLeftPart(UriPartial.Path) + "?" + string.Join("&", names.Select(n => n + "=REDACTED"));
    }
}
