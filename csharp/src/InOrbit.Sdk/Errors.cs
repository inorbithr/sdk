using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;
using System.Text.Json;

namespace InOrbit.Sdk;

/// <summary>
/// An error code from the platform (<c>spec/problem.json</c>): a known one, or a newer one
/// kept exactly as the API wrote it.
/// </summary>
/// <param name="Value">The code as it travels on the wire, such as <c>not_found</c>.</param>
public readonly record struct Code(string Value)
{
    /// <summary>The request is malformed.</summary>
    public static Code BadRequest { get; } = new("bad_request");

    /// <summary>The request cannot be served in the state the resource is in.</summary>
    public static Code FailedPrecondition { get; } = new("failed_precondition");

    /// <summary>The token is missing, expired or revoked.</summary>
    public static Code Unauthenticated { get; } = new("unauthenticated");

    /// <summary>The credential may not call this route.</summary>
    public static Code Forbidden { get; } = new("forbidden");

    /// <summary>No such route or resource.</summary>
    public static Code NotFound { get; } = new("not_found");

    /// <summary>The route does not take this method.</summary>
    public static Code MethodNotAllowed { get; } = new("method_not_allowed");

    /// <summary>The resource exists already.</summary>
    public static Code AlreadyExists { get; } = new("already_exists");

    /// <summary>The request conflicts with the resource's state.</summary>
    public static Code Conflict { get; } = new("conflict");

    /// <summary>The request body is too large.</summary>
    public static Code PayloadTooLarge { get; } = new("payload_too_large");

    /// <summary>The request body has a media type the route does not take.</summary>
    public static Code UnsupportedMediaType { get; } = new("unsupported_media_type");

    /// <summary>Too many requests; try again shortly.</summary>
    public static Code RateLimited { get; } = new("rate_limited");

    /// <summary>The account's units for the period are spent.</summary>
    public static Code QuotaExceeded { get; } = new("quota_exceeded");

    /// <summary>The call was cancelled.</summary>
    public static Code Cancelled { get; } = new("cancelled");

    /// <summary>The API failed.</summary>
    public static Code Internal { get; } = new("internal");

    /// <summary>The route is not implemented.</summary>
    public static Code Unimplemented { get; } = new("unimplemented");

    /// <summary>The API is unavailable for now.</summary>
    public static Code Unavailable { get; } = new("unavailable");

    /// <summary>The API did not answer in time.</summary>
    public static Code Timeout { get; } = new("timeout");

    private static readonly Dictionary<string, int> Known = new(StringComparer.Ordinal)
    {
        ["bad_request"] = 400,
        ["failed_precondition"] = 400,
        ["unauthenticated"] = 401,
        ["forbidden"] = 403,
        ["not_found"] = 404,
        ["method_not_allowed"] = 405,
        ["already_exists"] = 409,
        ["conflict"] = 409,
        ["payload_too_large"] = 413,
        ["unsupported_media_type"] = 415,
        ["rate_limited"] = 429,
        ["quota_exceeded"] = 429,
        ["cancelled"] = 499,
        ["internal"] = 500,
        ["unimplemented"] = 501,
        ["unavailable"] = 503,
        ["timeout"] = 504,
    };

    /// <summary>Whether this version of the SDK knows the code.</summary>
    public bool IsKnown => Value is not null && Known.ContainsKey(Value);

    /// <summary>The HTTP status the platform sends the code with, when the code is known.</summary>
    public int? HttpStatus => Value is not null && Known.TryGetValue(Value, out var s) ? s : null;

    /// <summary>The code a plain-text answer with <paramref name="status"/> stands for.</summary>
    /// <param name="status">The HTTP status.</param>
    /// <returns>The matching code, <c>internal</c> for another 5xx, else <c>http_&lt;status&gt;</c>.</returns>
    public static Code ForStatus(int status) => status switch
    {
        400 => BadRequest,
        401 => Unauthenticated,
        403 => Forbidden,
        404 => NotFound,
        405 => MethodNotAllowed,
        409 => Conflict,
        413 => PayloadTooLarge,
        415 => UnsupportedMediaType,
        429 => RateLimited,
        499 => Cancelled,
        501 => Unimplemented,
        503 => Unavailable,
        504 => Timeout,
        >= 500 => Internal,
        _ => new Code("http_" + status.ToString(CultureInfo.InvariantCulture)),
    };

    /// <inheritdoc/>
    public override string ToString() => Value ?? string.Empty;
}

/// <summary>One entry of an error's <c>details</c>.</summary>
public abstract record Detail
{
    private protected Detail()
    {
    }

    internal static Detail Read(JsonElement d)
    {
        if (d.ValueKind != JsonValueKind.Object)
        {
            return new UnknownDetail(d.Clone());
        }

        string Str(string name) =>
            d.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() ?? string.Empty : string.Empty;

        var type = Str("type");
        switch (type)
        {
            case "field":
                return new FieldDetail(Str("field"), Str("description"));
            case "info":
                var metadata = new Dictionary<string, string>(StringComparer.Ordinal);
                if (d.TryGetProperty("metadata", out var m) && m.ValueKind == JsonValueKind.Object)
                {
                    foreach (var p in m.EnumerateObject())
                    {
                        metadata[p.Name] = p.Value.ValueKind == JsonValueKind.String ? p.Value.GetString() ?? string.Empty : p.Value.GetRawText();
                    }
                }

                return new InfoDetail(Str("reason"), Str("domain"), metadata);
            case "retry":
                long seconds = 0;
                if (d.TryGetProperty("after_seconds", out var s))
                {
                    if (s.ValueKind == JsonValueKind.Number && s.TryGetInt64(out var n))
                    {
                        seconds = n;
                    }
                    else if (s.ValueKind == JsonValueKind.String && long.TryParse(s.GetString(), NumberStyles.None, CultureInfo.InvariantCulture, out var parsed))
                    {
                        seconds = parsed;
                    }
                }

                return new RetryDetail(seconds);
            default:
                return new UnknownDetail(d.Clone());
        }
    }
}

/// <summary>A field of the request was wrong.</summary>
/// <param name="Field">The field's wire name.</param>
/// <param name="Description">What is wrong with it.</param>
public sealed record FieldDetail(string Field, string Description) : Detail;

/// <summary>Why the API refused, in machine-readable form.</summary>
/// <param name="Reason">A stable reason.</param>
/// <param name="Domain">The domain the reason belongs to.</param>
/// <param name="Metadata">Extra values the reason carries.</param>
public sealed record InfoDetail(string Reason, string Domain, IReadOnlyDictionary<string, string> Metadata) : Detail;

/// <summary>When to try again.</summary>
/// <param name="AfterSeconds">Seconds to wait.</param>
public sealed record RetryDetail(long AfterSeconds) : Detail;

/// <summary>A detail type this version does not know, kept as it came.</summary>
/// <param name="Value">The detail as JSON.</param>
public sealed record UnknownDetail(JsonElement Value) : Detail;

/// <summary>An HTTP answer as it came, for anything the typed result does not carry.</summary>
public sealed class RawResponse
{
    internal RawResponse(int status, IReadOnlyDictionary<string, IReadOnlyList<string>> headers, byte[] body, string requestId, int attempts)
    {
        Status = status;
        Headers = headers;
        Body = body;
        RequestId = requestId;
        ServerRequestId = Header("x-request-id");
        Attempts = attempts;
    }

    /// <summary>The HTTP status.</summary>
    public int Status { get; }

    /// <summary>The response headers, by lower-case name.</summary>
    public IReadOnlyDictionary<string, IReadOnlyList<string>> Headers { get; }

    /// <summary>The body, at most 16 MiB.</summary>
    public ReadOnlyMemory<byte> Body { get; }

    /// <summary>The <c>x-request-id</c> this SDK sent.</summary>
    public string RequestId { get; }

    /// <summary>The request id the API answered with, if any.</summary>
    public string? ServerRequestId { get; }

    /// <summary>How many attempts the call took.</summary>
    public int Attempts { get; }

    /// <summary>The first value of a header, or <see langword="null"/>.</summary>
    /// <param name="name">The header's name, any case.</param>
    /// <returns>The value, or <see langword="null"/> when the answer has no such header.</returns>
    public string? Header(string name)
    {
        ArgumentNullException.ThrowIfNull(name);
        return Headers.TryGetValue(name.ToLowerInvariant(), out var v) && v.Count > 0 ? v[0] : null;
    }

    /// <summary>The body as text.</summary>
    /// <returns>The body decoded as UTF-8.</returns>
    public string Text() => Encoding.UTF8.GetString(Body.Span);

    /// <summary>The body as JSON, or <see langword="null"/> when it is not JSON.</summary>
    /// <returns>A copy of the parsed document's root, or <see langword="null"/>.</returns>
    public JsonElement? Json()
    {
        try
        {
            using var doc = JsonDocument.Parse(Body);
            return doc.RootElement.Clone();
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>Shows the status, the size and the request ids; never the body.</summary>
    /// <returns>A one-line summary.</returns>
    public override string ToString() =>
        string.Create(CultureInfo.InvariantCulture, $"RawResponse({Status}, {Body.Length} bytes, request id {RequestId})");
}

/// <summary>The base of every exception this SDK throws. The message never holds a secret.</summary>
public abstract class InOrbitException : Exception
{
    private protected InOrbitException(string kind, string message, Exception? inner = null)
        : base(message, inner)
    {
        Kind = kind;
    }

    /// <summary>A stable kind: <c>api</c>, <c>connection</c>, <c>timeout</c>, <c>auth</c>, <c>config</c>, <c>too_large</c>, <c>decode</c>.</summary>
    public string Kind { get; }
}

/// <summary>The API answered with the problem envelope, or a plain-text error from the gateway.</summary>
public sealed class ApiException : InOrbitException
{
    private static readonly Dictionary<int, string> GatewayMessages = new()
    {
        [401] = "the token was refused: it is missing, expired or revoked",
        [403] = "this credential may not call this route: its scopes or role do not allow it",
        [404] = "no such route or resource",
        [429] = "too many requests; try again shortly",
    };

    private ApiException(RawResponse raw, Code code, string problem, IReadOnlyList<Detail> details)
        : base("api", Describe(raw, code, problem))
    {
        Status = raw.Status;
        Code = code;
        Problem = problem;
        Details = details;
        Raw = raw;
    }

    /// <summary>The HTTP status.</summary>
    public int Status { get; }

    /// <summary>The error code.</summary>
    public Code Code { get; }

    /// <summary>What the API said went wrong.</summary>
    public string Problem { get; }

    /// <summary>The typed details.</summary>
    public IReadOnlyList<Detail> Details { get; }

    /// <summary>The answer as it came.</summary>
    public RawResponse Raw { get; }

    /// <summary>Seconds the API asked to wait, from a <c>retry</c> detail or <c>Retry-After</c>.</summary>
    /// <returns>The seconds, or <see langword="null"/> when the answer names none.</returns>
    public long? RetryAfterSeconds()
    {
        var detail = Details.OfType<RetryDetail>().FirstOrDefault();
        if (detail is not null)
        {
            return detail.AfterSeconds;
        }

        var header = Raw.Header("retry-after")?.Trim();
        return long.TryParse(header, NumberStyles.None, CultureInfo.InvariantCulture, out var s) ? s : null;
    }

    internal static ApiException From(RawResponse raw)
    {
        var json = raw.Json();
        Code code;
        string message;
        var details = new List<Detail>();
        string Str(JsonElement e, string name) =>
            e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() ?? string.Empty : string.Empty;
        if (json is { ValueKind: JsonValueKind.Object } wire && (Str(wire, "code").Length > 0 || Str(wire, "error").Length > 0))
        {
            var wireCode = Str(wire, "code");
            code = wireCode.Length > 0 ? new Code(wireCode) : Code.ForStatus(raw.Status);
            message = Str(wire, "error");
            if (wire.TryGetProperty("details", out var list) && list.ValueKind == JsonValueKind.Array)
            {
                details.AddRange(list.EnumerateArray().Select(Detail.Read));
            }
        }
        else
        {
            code = Code.ForStatus(raw.Status);
            message = raw.Text().Trim();
        }

        message = message.Length == 0 ? GatewayMessage(raw.Status) : Truncate(message, 300);
        return new ApiException(raw, code, message, details);
    }

    private static string GatewayMessage(int status) =>
        GatewayMessages.TryGetValue(status, out var m) ? m : status >= 500 ? "the API failed to answer" : "the request was refused";

    private static string Truncate(string s, int max)
    {
        var info = new StringInfo(s);
        return info.LengthInTextElements > max ? info.SubstringByTextElements(0, max) + "…" : s;
    }

    private static string Describe(RawResponse raw, Code code, string problem)
    {
        var id = raw.ServerRequestId is null ? string.Empty : ", request id " + raw.ServerRequestId;
        return string.Create(CultureInfo.InvariantCulture, $"{problem} ({code}, HTTP {raw.Status}{id})");
    }
}

/// <summary>The API could not be reached: DNS, TCP, TLS, or a reset before an answer.</summary>
public sealed class ConnectionException : InOrbitException
{
    internal ConnectionException(string host, string reason, Exception? inner = null)
        : base("connection", $"cannot reach {host}: {reason}", inner)
    {
        Host = host;
    }

    /// <summary>The host that could not be reached.</summary>
    public string Host { get; }
}

/// <summary>An attempt ran out of time.</summary>
public sealed class RequestTimeoutException : InOrbitException
{
    internal RequestTimeoutException(string host, TimeSpan after)
        : base("timeout", string.Create(CultureInfo.InvariantCulture, $"{host} did not answer within {after.TotalSeconds:0.###} s"))
    {
        Host = host;
    }

    /// <summary>The host that did not answer.</summary>
    public string Host { get; }
}

/// <summary>The token exchange, or a custom token provider, failed.</summary>
public sealed class AuthException : InOrbitException
{
    /// <summary>An auth failure with its message.</summary>
    /// <param name="message">What failed; never a secret.</param>
    /// <param name="error">The token endpoint's <c>error</c>, or <c>HTTP &lt;status&gt;</c>; empty for a transport failure.</param>
    /// <param name="inner">The underlying failure, if any.</param>
    public AuthException(string message, string error = "", Exception? inner = null)
        : base("auth", message, inner)
    {
        Error = error;
    }

    /// <summary>The token endpoint's <c>error</c>, or <c>HTTP &lt;status&gt;</c>; empty for a transport failure.</summary>
    public string Error { get; }
}

/// <summary>The client was configured in a way it cannot work with.</summary>
public sealed class ConfigException : InOrbitException
{
    internal ConfigException(string message)
        : base("config", message)
    {
    }
}

/// <summary>The answer is larger than 16 MiB.</summary>
public sealed class TooLargeException : InOrbitException
{
    internal TooLargeException()
        : base("too_large", "the answer is larger than 16 MiB; refusing to read it")
    {
    }
}

/// <summary>The API answered something this version cannot read.</summary>
public sealed class DecodeException : InOrbitException
{
    internal DecodeException(string reason, RawResponse raw, Exception? inner = null)
        : base("decode", $"the API answered something this version of InOrbit.Sdk cannot read: {reason}", inner)
    {
        Raw = raw;
    }

    /// <summary>The answer as it came.</summary>
    public RawResponse Raw { get; }
}
