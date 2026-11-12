using System.Diagnostics;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace App;

// Observability on the .NET libraries alone: JSON logs (System.Text.Json), spans that follow the
// W3C traceparent header, and request counters in the Prometheus text format. Nothing here
// records a header, a query string or an environment value: a log line and a span name the route
// that matched, the status, the duration and the trace id.

/// <summary>JSON strings for a body: the relaxed encoder keeps readable text readable (it is not for HTML).</summary>
public static class Json
{
    private static readonly JsonSerializerOptions Options = new() { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

    public static string Str(string s) => JsonSerializer.Serialize(s, Options);
}

/// <summary>One JSON object per line, written to a stream.</summary>
public sealed class JsonLog(TextWriter output)
{
    private readonly object gate = new();

    public void Info(string msg, params (string Key, object Value)[] attrs) => Write("INFO", msg, attrs);

    public void Error(string msg, params (string Key, object Value)[] attrs) => Write("ERROR", msg, attrs);

    private void Write(string level, string msg, (string Key, object Value)[] attrs)
    {
        var buffer = new MemoryStream();
        using (var w = new Utf8JsonWriter(buffer, new JsonWriterOptions { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping }))
        {
            w.WriteStartObject();
            w.WriteString("time", DateTime.UtcNow.ToString("O", CultureInfo.InvariantCulture));
            w.WriteString("level", level);
            w.WriteString("msg", msg);
            foreach (var (key, value) in attrs)
            {
                switch (value)
                {
                    case int i: w.WriteNumber(key, i); break;
                    case double d: w.WriteNumber(key, d); break;
                    default: w.WriteString(key, Convert.ToString(value, CultureInfo.InvariantCulture)); break;
                }
            }
            w.WriteEndObject();
        }
        lock (gate)
        {
            output.WriteLine(Encoding.UTF8.GetString(buffer.ToArray()));
            output.Flush();
        }
    }
}

/// <summary>One request's trace: the id it joined (or started) and its own id.</summary>
public sealed record Span(string TraceId, string SpanId, string? ParentId, long Started)
{
    private static readonly Regex Traceparent = new("^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$");

    private static string RandomHex(int bytes) => Convert.ToHexStringLower(RandomNumberGenerator.GetBytes(bytes));

    /// <summary>Joins the trace of a valid traceparent header, or starts a new one.</summary>
    public static Span Start(string? header)
    {
        var m = header is null ? null : Traceparent.Match(header);
        if (m is { Success: true }
            && m.Groups[1].Value != new string('0', 32) && m.Groups[2].Value != new string('0', 16))
        {
            return new Span(m.Groups[1].Value, RandomHex(8), m.Groups[2].Value, Stopwatch.GetTimestamp());
        }
        return new Span(RandomHex(16), RandomHex(8), null, Stopwatch.GetTimestamp());
    }

    /// <summary>The traceparent to send back (and on to callers): this span is the parent.</summary>
    public string Header() => $"00-{TraceId}-{SpanId}-01";

    public double Millis() => Stopwatch.GetElapsedTime(Started).TotalMilliseconds;
}

/// <summary>
/// Counts requests by route and status. Routes are the matched patterns, never raw paths, so the
/// label set is bounded by the code and holds no user input.
/// </summary>
public sealed class Metrics
{
    private readonly SortedDictionary<(string Route, int Status), ulong> counts = new();

    public void Inc(string route, int status)
    {
        lock (counts)
        {
            counts[(route, status)] = counts.GetValueOrDefault((route, status)) + 1;
        }
    }

    public string Render()
    {
        var b = new StringBuilder("# HELP http_requests_total Requests served, by route and status.\n# TYPE http_requests_total counter\n");
        lock (counts)
        {
            foreach (var ((route, status), n) in counts)
            {
                b.Append(CultureInfo.InvariantCulture, $"http_requests_total{{route={Json.Str(route)},status=\"{status}\"}} {n}\n");
            }
        }
        return b.ToString();
    }
}
