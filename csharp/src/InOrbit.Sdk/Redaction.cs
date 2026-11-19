using System;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace InOrbit.Sdk;

/// <summary>
/// How a secret shows wherever it could print (SR-10): a key secret, a token, a proxy
/// password or a client key password is <c>&lt;redacted&gt;</c> in every <c>ToString</c> and
/// in JSON. Every type that holds one shows it through this class.
/// </summary>
internal static class Redaction
{
    /// <summary>What a secret shows as.</summary>
    internal const string Marker = "<redacted>";

    /// <summary><paramref name="raw"/> with its user-info (a proxy's credentials) replaced by the marker.</summary>
    internal static string Userinfo(string raw)
    {
        var i = raw.IndexOf("://", StringComparison.Ordinal);
        if (i < 0)
        {
            return raw;
        }

        var rest = raw[(i + 3)..];
        var end = rest.IndexOfAny(['/', '?', '#']);
        var authority = end < 0 ? rest : rest[..end];
        var at = authority.LastIndexOf('@');
        return at < 0 ? raw : $"{raw[..i]}://{Marker}@{rest[(at + 1)..]}";
    }

    /// <summary>The marker when <paramref name="secret"/> is set, else <c>null</c>.</summary>
    internal static string? Shown(string? secret) => secret is null ? null : Marker;
}

/// <summary>Writes a secret string as <c>&lt;redacted&gt;</c>; reads one as it is.</summary>
internal sealed class RedactedJsonConverter : JsonConverter<string>
{
    /// <inheritdoc/>
    public override string? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) => reader.GetString();

    /// <inheritdoc/>
    public override void Write(Utf8JsonWriter writer, string value, JsonSerializerOptions options) => writer.WriteStringValue(Redaction.Marker);
}

/// <summary>Writes a URL with its user-info as <c>&lt;redacted&gt;</c>; reads one as it is.</summary>
internal sealed class RedactedUrlJsonConverter : JsonConverter<string>
{
    /// <inheritdoc/>
    public override string? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) => reader.GetString();

    /// <inheritdoc/>
    public override void Write(Utf8JsonWriter writer, string value, JsonSerializerOptions options) => writer.WriteStringValue(Redaction.Userinfo(value));
}
