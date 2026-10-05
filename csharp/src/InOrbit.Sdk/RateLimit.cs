using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.RegularExpressions;

namespace InOrbit.Sdk;

/// <summary>What the client does with rate-limit headers (docs/config.md section 7.8).</summary>
public enum RateLimitMode
{
    /// <summary>Read them: every result and error carries a <see cref="RateLimit"/> snapshot. The default.</summary>
    Observe,

    /// <summary>As <see cref="Observe"/>, and before an attempt wait out a window whose <c>remaining</c> is 0.</summary>
    Wait,

    /// <summary>Do not read them.</summary>
    Off,
}

/// <summary>The policy the IETF <c>RateLimit-Policy</c> field named.</summary>
/// <param name="Name">The policy's name.</param>
/// <param name="Quota">Requests the window allows.</param>
/// <param name="Window">The window's length.</param>
public sealed record RateLimitPolicy(string Name, long? Quota, TimeSpan? Window);

/// <summary>What an answer said about the rate limit: Envoy's <c>X-RateLimit-*</c> headers, or the IETF <c>RateLimit</c> fields, which win.</summary>
/// <param name="Limit">Requests the window allows.</param>
/// <param name="Remaining">Requests left in the window.</param>
/// <param name="Reset">How long until the window resets.</param>
/// <param name="Policy">The policy, when the IETF fields named one.</param>
public sealed partial record RateLimit(long? Limit, long? Remaining, TimeSpan? Reset, RateLimitPolicy? Policy)
{
    /// <summary>The snapshot <paramref name="headers"/> carry, or <see langword="null"/> when they carry none or a malformed one.</summary>
    /// <param name="headers">Response headers by lower-case name.</param>
    /// <returns>The snapshot.</returns>
    internal static RateLimit? Read(IReadOnlyDictionary<string, IReadOnlyList<string>> headers)
    {
        string? H(string name) => headers.TryGetValue(name, out var v) && v.Count > 0 ? string.Join(", ", v) : null;
        if (H("ratelimit") is { } field && FirstItem(field) is { } item)
        {
            var policyItem = H("ratelimit-policy") is { } pf ? FirstItem(pf) : null;
            RateLimitPolicy? policy = null;
            if (policyItem is { } pi && pi.Name == item.Name)
            {
                var w = Int(pi.Params.GetValueOrDefault("w"));
                policy = new RateLimitPolicy(pi.Name, Int(pi.Params.GetValueOrDefault("q")), w is { } ws ? TimeSpan.FromSeconds(ws) : null);
            }

            var t = Int(item.Params.GetValueOrDefault("t"));
            return new RateLimit(policy?.Quota, Int(item.Params.GetValueOrDefault("r")), t is { } ts ? TimeSpan.FromSeconds(ts) : null, policy);
        }

        var (limit, badLimit) = Count(H("x-ratelimit-limit"));
        var (remaining, badRemaining) = Count(H("x-ratelimit-remaining"));
        var (reset, badReset) = Count(H("x-ratelimit-reset"));
        if (badLimit || badRemaining || badReset || (limit is null && remaining is null && reset is null))
        {
            return null;
        }

        return new RateLimit(limit, remaining, reset is { } r ? TimeSpan.FromSeconds(r) : null, null);
    }

    private static (long? Value, bool Bad) Count(string? v)
    {
        if (v is null)
        {
            return (null, false);
        }

        return long.TryParse(v.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var n) ? (n, false) : (null, true);
    }

    private static long? Int(string? v) => v is not null && long.TryParse(v, NumberStyles.None, CultureInfo.InvariantCulture, out var n) ? n : null;

    private static (string Name, Dictionary<string, string> Params)? FirstItem(string field)
    {
        var text = field.Split(',')[0].Trim();
        var parts = text.Split(';');
        var head = parts[0].Trim();
        var m = QuotedName().Match(head);
        if (!m.Success)
        {
            m = TokenName().Match(head);
        }

        if (!m.Success)
        {
            return null;
        }

        var ps = new Dictionary<string, string>(StringComparer.Ordinal);
        for (var i = 1; i < parts.Length; i++)
        {
            var p = parts[i].Trim();
            var eq = p.IndexOf('=', StringComparison.Ordinal);
            if (eq > 0)
            {
                ps[p[..eq].Trim()] = p[(eq + 1)..].Trim();
            }
        }

        return (m.Groups[1].Value, ps);
    }

    [GeneratedRegex("^\"([^\"\\\\]*)\"$", RegexOptions.CultureInvariant)]
    private static partial Regex QuotedName();

    [GeneratedRegex("^([A-Za-z*][A-Za-z0-9_\\-.:%*/]*)$", RegexOptions.CultureInvariant)]
    private static partial Regex TokenName();
}
