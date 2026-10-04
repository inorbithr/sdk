using System;
using System.Globalization;
using System.Text.RegularExpressions;

namespace InOrbit.Sdk;

/// <summary>
/// Timestamps as the API sends them: RFC 3339 strings, <c>""</c> when unset (rule N5 of
/// <c>spec/README.md</c>, settled behaviour). Models keep the string; this reads it.
/// </summary>
public static partial class Timestamps
{
    [GeneratedRegex(@"^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$", RegexOptions.CultureInvariant)]
    private static partial Regex Rfc3339();

    /// <summary>Reads a timestamp field: <c>null</c> for <c>""</c> (the field is unset), the instant for an RFC 3339 string.</summary>
    /// <param name="value">The field as sent.</param>
    /// <returns>The instant, or <c>null</c> when the field is unset.</returns>
    /// <exception cref="FormatException">The value is neither <c>""</c> nor RFC 3339.</exception>
    public static DateTimeOffset? Parse(string value)
    {
        ArgumentNullException.ThrowIfNull(value);
        if (value.Length == 0)
        {
            return null;
        }

        if (!Rfc3339().IsMatch(value) || !DateTimeOffset.TryParse(value, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out var at))
        {
            throw new FormatException("not an RFC 3339 timestamp");
        }

        return at;
    }
}
