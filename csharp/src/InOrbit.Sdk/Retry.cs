using System;
using System.Globalization;
using System.Net.Http.Headers;
using System.Security.Cryptography;

namespace InOrbit.Sdk;

/// <summary>The retry rules of design.md section 6 and docs/config.md section 7.4.</summary>
internal static class Retry
{
    /// <summary><c>Retry-After</c> is honoured up to a minute by default.</summary>
    internal static readonly TimeSpan RetryAfterCap = TimeSpan.FromSeconds(60);

    private static readonly TimeSpan BackoffBase = TimeSpan.FromMilliseconds(500);
    private static readonly TimeSpan BackoffCap = TimeSpan.FromSeconds(8);

    /// <summary>Statuses worth another attempt on an idempotent call.</summary>
    internal static bool RetryableStatus(int status) => status is 429 or 503 or 504;

    /// <summary>The wait <c>Retry-After</c> asks for, capped at a minute: delay-seconds or an HTTP date.</summary>
    internal static TimeSpan? RetryAfter(RetryConditionHeaderValue? header)
    {
        var wait = Asked(header);
        return wait is { } w && w > RetryAfterCap ? RetryAfterCap : wait;
    }

    /// <summary>The wait <c>Retry-After</c> asks for, uncapped: delay-seconds or an HTTP date (RFC 9110 section 10.2.3).</summary>
    internal static TimeSpan? Asked(RetryConditionHeaderValue? header)
    {
        if (header?.Delta is { } delta)
        {
            return delta < TimeSpan.Zero ? null : delta;
        }

        if (header?.Date is { } date)
        {
            var wait = date - DateTimeOffset.UtcNow;
            return wait < TimeSpan.Zero ? TimeSpan.Zero : wait;
        }

        return null;
    }

    /// <summary>The wait a raw <c>Retry-After</c> value asks for: delay-seconds or an IMF-fixdate.</summary>
    internal static TimeSpan? Asked(string? value)
    {
        var v = value?.Trim();
        if (string.IsNullOrEmpty(v))
        {
            return null;
        }

        if (long.TryParse(v, NumberStyles.None, CultureInfo.InvariantCulture, out var s))
        {
            return s > TimeSpan.MaxValue.TotalSeconds / 2 ? TimeSpan.MaxValue / 2 : TimeSpan.FromSeconds(s);
        }

        if (DateTimeOffset.TryParseExact(v, "r", CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out var at))
        {
            var wait = at - DateTimeOffset.UtcNow;
            return wait < TimeSpan.Zero ? TimeSpan.Zero : wait;
        }

        return null;
    }

    /// <summary>Full jitter: a random wait up to 0.5 s doubled per retry, at most 8 s.</summary>
    internal static TimeSpan Backoff(int retry) => Backoff(retry, BackoffBase, BackoffCap);

    /// <summary>Full jitter: a random wait up to <paramref name="baseDelay"/> doubled per retry, at most <paramref name="cap"/>.</summary>
    internal static TimeSpan Backoff(int retry, TimeSpan baseDelay, TimeSpan cap)
    {
        var ceiling = Math.Min(baseDelay.TotalMilliseconds * Math.Pow(2, Math.Min(retry, 30)), cap.TotalMilliseconds);

        // Jitter only spreads retries out; Random.Shared is the right tool, not a secret.
#pragma warning disable CA5394
        return TimeSpan.FromMilliseconds(Math.Floor(Random.Shared.NextDouble() * (ceiling + 1)));
#pragma warning restore CA5394
    }

    /// <summary><c>iohr-&lt;16 hex&gt;</c>, the id each call is sent with.</summary>
    internal static string RequestId() => "iohr-" + Convert.ToHexString(RandomNumberGenerator.GetBytes(8)).ToLowerInvariant();
}
