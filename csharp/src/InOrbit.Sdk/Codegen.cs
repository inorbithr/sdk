using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.Text;
using System.Text.Json;

namespace InOrbit.Sdk;

/// <summary>
/// What a generated surface uses from the runtime, and nothing else does. This class is a
/// contract with <c>iohr sdk generate</c>: a change that breaks generated code bumps
/// <see cref="Version"/>, and a surface generated for another version fails to compile.
/// </summary>
[EditorBrowsable(EditorBrowsableState.Never)]
public static class Codegen
{
    /// <summary>The surface contract this runtime implements.</summary>
    public const int Version = 1;

    /// <summary>
    /// Percent-encodes <paramref name="value"/> as one path segment: every byte but the
    /// RFC 3986 unreserved characters, so a slash or a space in an id never changes the route.
    /// </summary>
    /// <param name="value">The parameter's value.</param>
    /// <returns>The encoded segment.</returns>
    public static string PathSegment(string value)
    {
        ArgumentNullException.ThrowIfNull(value);
        var bytes = Encoding.UTF8.GetBytes(value);
        var out_ = new StringBuilder(bytes.Length);
        foreach (var b in bytes)
        {
            var c = (char)b;
            if (b < 0x80 && (char.IsAsciiLetterOrDigit(c) || c is '-' or '.' or '_' or '~'))
            {
                out_.Append(c);
            }
            else
            {
                out_.Append('%').Append(b.ToString("X2", CultureInfo.InvariantCulture));
            }
        }

        return out_.ToString();
    }

    /// <summary>An operation, as the generated surface describes it.</summary>
    /// <param name="name">What hooks see (<c>radar.list_digests</c>).</param>
    /// <param name="method">The method.</param>
    /// <param name="path">The path, parameters bound with <see cref="PathSegment"/>.</param>
    /// <param name="scopes">The scopes it needs.</param>
    /// <param name="idempotent">Retry it like an idempotent method although its method is not.</param>
    /// <returns>The operation.</returns>
    public static Operation Operation(string name, Method method, string path, string[] scopes, bool idempotent = false) =>
        new(method, path) { Name = name, Scopes = scopes, Idempotent = idempotent };

    /// <summary>Adds the query parameters given, in order; <see langword="null"/> ones are left out, a list repeats the name.</summary>
    /// <param name="operation">The operation.</param>
    /// <param name="query">Name and value pairs; a value is a string, a number, a bool or a list of them.</param>
    /// <returns>The operation with its query.</returns>
    public static Operation WithQuery(this Operation operation, params (string Name, object? Value)[] query)
    {
        ArgumentNullException.ThrowIfNull(operation);
        ArgumentNullException.ThrowIfNull(query);
        var list = new List<KeyValuePair<string, string>>();
        foreach (var (name, value) in query)
        {
            if (value is System.Collections.IEnumerable many and not string)
            {
                foreach (var v in many)
                {
                    if (v is not null)
                    {
                        list.Add(new(name, Format(v)));
                    }
                }
            }
            else if (value is not null)
            {
                list.Add(new(name, Format(value)));
            }
        }

        return new Operation(operation.Method, operation.Path)
        {
            Name = operation.Name,
            Scopes = operation.Scopes,
            Idempotent = operation.Idempotent,
            Body = operation.Body,
            Query = list,
        };
    }

    /// <summary>Encodes <paramref name="body"/> as the operation's JSON body.</summary>
    /// <typeparam name="T">The body's model.</typeparam>
    /// <param name="operation">The operation.</param>
    /// <param name="body">The body.</param>
    /// <returns>The operation with its body.</returns>
    public static Operation WithJson<T>(this Operation operation, T body)
    {
        ArgumentNullException.ThrowIfNull(operation);
        return new Operation(operation.Method, operation.Path)
        {
            Name = operation.Name,
            Scopes = operation.Scopes,
            Idempotent = operation.Idempotent,
            Query = operation.Query,
            Body = JsonSerializer.SerializeToUtf8Bytes(body, Json.Options),
        };
    }

    private static string Format(object v) => v switch
    {
        bool b => b ? "true" : "false",
        IFormattable f => f.ToString(null, CultureInfo.InvariantCulture),
        _ => v.ToString() ?? string.Empty,
    };
}

/// <summary>A string with a known set of values that keeps a newer value as the API wrote it; generated enums implement it.</summary>
/// <typeparam name="TSelf">The implementing type.</typeparam>
[EditorBrowsable(EditorBrowsableState.Never)]
public interface IStringValue<TSelf>
    where TSelf : IStringValue<TSelf>
{
    /// <summary>The value as it travels on the wire.</summary>
    string Value { get; }

    /// <summary>The value for <paramref name="value"/>, known or not.</summary>
    /// <param name="value">The wire value.</param>
    /// <returns>The value.</returns>
    static abstract TSelf From(string value);
}

/// <summary>Reads and writes an <see cref="IStringValue{TSelf}"/> as a JSON string.</summary>
/// <typeparam name="T">The value type.</typeparam>
[EditorBrowsable(EditorBrowsableState.Never)]
public sealed class StringValueConverter<T> : System.Text.Json.Serialization.JsonConverter<T>
    where T : IStringValue<T>
{
    /// <inheritdoc/>
    public override T Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        reader.TokenType == JsonTokenType.String
            ? T.From(reader.GetString() ?? string.Empty)
            : throw new JsonException($"expected a string for {typeof(T).Name}");

    /// <inheritdoc/>
    public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options)
    {
        ArgumentNullException.ThrowIfNull(writer);
        writer.WriteStringValue(value.Value);
    }
}

/// <summary>A model kept as raw JSON (a union or an alias the surface does not split); generated types implement it.</summary>
/// <typeparam name="TSelf">The implementing type.</typeparam>
[EditorBrowsable(EditorBrowsableState.Never)]
public interface IRawJson<TSelf>
    where TSelf : IRawJson<TSelf>
{
    /// <summary>The value as JSON.</summary>
    JsonElement Value { get; }

    /// <summary>The model for <paramref name="value"/>.</summary>
    /// <param name="value">The JSON.</param>
    /// <returns>The model.</returns>
    static abstract TSelf From(JsonElement value);
}

/// <summary>Reads and writes an <see cref="IRawJson{TSelf}"/> as the JSON it holds.</summary>
/// <typeparam name="T">The model type.</typeparam>
[EditorBrowsable(EditorBrowsableState.Never)]
public sealed class RawJsonConverter<T> : System.Text.Json.Serialization.JsonConverter<T>
    where T : IRawJson<T>
{
    /// <inheritdoc/>
    public override T Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        using var doc = JsonDocument.ParseValue(ref reader);
        return T.From(doc.RootElement.Clone());
    }

    /// <inheritdoc/>
    public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options)
    {
        ArgumentNullException.ThrowIfNull(writer);
        value.Value.WriteTo(writer);
    }
}
