using System;
using System.Net.Http.Headers;
using System.Security.Cryptography;

namespace InOrbit.Sdk;

/// <summary>The retry rules of design.md section 6.</summary>
internal static class Retry
{
    /// <summary><c>Retry-After</c> is honoured up to a minute.</summary>
    internal static readonly TimeSpan RetryAfterCap = TimeSpan.FromSeconds(60);

    private const int BackoffBaseMs = 500;
    private const int BackoffCapMs = 8_000;

    /// <summary>Statuses worth another attempt on an idempotent call.</summary>
    internal static bool RetryableStatus(int status) => status is 429 or 503 or 504;

    /// <summary>The wait <c>Retry-After</c> asks for, capped; seconds only.</summary>
    internal static TimeSpan? RetryAfter(RetryConditionHeaderValue? header)
    {
        if (header?.Delta is not { } delta || delta < TimeSpan.Zero)
        {
            return null;
        }

        return delta < RetryAfterCap ? delta : RetryAfterCap;
    }

    /// <summary>Full jitter: a random wait up to 0.5 s doubled per retry, at most 8 s.</summary>
    internal static TimeSpan Backoff(int retry)
    {
        var ceiling = Math.Min(BackoffBaseMs << Math.Min(retry, 5), BackoffCapMs);

        // Jitter only spreads retries out; Random.Shared is the right tool, not a secret.
#pragma warning disable CA5394
        return TimeSpan.FromMilliseconds(Random.Shared.Next(ceiling + 1));
#pragma warning restore CA5394
    }

    /// <summary><c>iohr-&lt;16 hex&gt;</c>, the id each call is sent with.</summary>
    internal static string RequestId() => "iohr-" + Convert.ToHexString(RandomNumberGenerator.GetBytes(8)).ToLowerInvariant();
}
