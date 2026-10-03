using System;
using System.Collections.Generic;
using System.Globalization;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace InOrbit.Sdk;

/// <summary>A bearer token and, when known, when it stops working.</summary>
public sealed class Token
{
    /// <summary>A token that may expire at <paramref name="expiresAt"/>.</summary>
    /// <param name="access">The bearer token. It is never printed.</param>
    /// <param name="expiresAt">When it expires, if the provider knows.</param>
    public Token(string access, DateTimeOffset? expiresAt = null)
    {
        ArgumentException.ThrowIfNullOrEmpty(access);
        Access = access;
        ExpiresAt = expiresAt;
    }

    /// <summary>The bearer token. Never log it.</summary>
    public string Access { get; }

    /// <summary>When it expires, if the provider knows.</summary>
    public DateTimeOffset? ExpiresAt { get; }

    /// <summary>Never the token.</summary>
    /// <returns><c>Token(&lt;redacted&gt;)</c>.</returns>
    public override string ToString() => "Token(<redacted>)";
}

/// <summary>Hands out a valid token, and hears when the API refused the last one.</summary>
public interface ITokenProvider
{
    /// <summary>A token for the next attempt.</summary>
    /// <param name="cancellationToken">Cancels the wait for a token.</param>
    /// <returns>The token.</returns>
    ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken);

    /// <summary>The API answered 401 with the last token: drop any cached one.</summary>
    /// <returns>A task that completes when the cache is dropped.</returns>
    ValueTask InvalidateAsync() => ValueTask.CompletedTask;
}

/// <summary>A token you already hold, such as an API token from the console.</summary>
public sealed class StaticToken : ITokenProvider
{
    private readonly Token _token;

    /// <summary>A provider that always hands out <paramref name="token"/>.</summary>
    /// <param name="token">The bearer token.</param>
    public StaticToken(string token)
    {
        _token = new Token(token);
    }

    /// <inheritdoc/>
    public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken) => ValueTask.FromResult(_token);

    /// <summary>Never the token.</summary>
    /// <returns><c>StaticToken(&lt;redacted&gt;)</c>.</returns>
    public override string ToString() => "StaticToken(<redacted>)";
}

/// <summary>
/// Exchanges an API key for short-lived tokens (OAuth client credentials), caches the
/// token, refreshes it when less than a fifth of its life is left or after a 401, and
/// makes one exchange however many calls wait for it.
/// </summary>
public sealed class ClientCredentials : ITokenProvider, IDisposable
{
    /// <summary>The audience every token for the API is asked for.</summary>
    public const string Audience = "iohr-api";

    /// <summary>Where an API key is exchanged for a token.</summary>
    public static readonly Uri DefaultTokenUrl = new("https://auth.inorbit.hr/oauth2/token");

    private const int TokenRetries = 2;
    private static readonly TimeSpan TokenTimeout = TimeSpan.FromSeconds(30);

    private readonly string _keyId;
    private readonly string _secret;
    private readonly IReadOnlyList<string> _scopes;
    private readonly Uri _tokenUrl;
    private readonly HttpClient _http;
    private readonly bool _ownsHttp;
    private readonly SemaphoreSlim _gate = new(1, 1);
    private Cached? _cache;
    private long _generation;

    /// <summary>A provider for one key.</summary>
    /// <param name="keyId">The key's id (<c>ak_…</c>).</param>
    /// <param name="keySecret">The key's secret. It is never printed.</param>
    /// <param name="scopes">The scopes to ask for, a subset of the key's.</param>
    /// <param name="tokenUrl">The token endpoint (default <see cref="DefaultTokenUrl"/>); plain HTTP only to this machine.</param>
    /// <param name="httpClient">The client to send the exchange with (default: one of its own).</param>
    public ClientCredentials(string keyId, string keySecret, IReadOnlyList<string> scopes, Uri? tokenUrl = null, HttpClient? httpClient = null)
    {
        ArgumentException.ThrowIfNullOrEmpty(keyId);
        ArgumentException.ThrowIfNullOrEmpty(keySecret);
        ArgumentNullException.ThrowIfNull(scopes);
        if (scopes.Count == 0)
        {
            throw new ConfigException("no scopes: set INORBIT_SCOPES (space-separated, such as \"identity:read account:read\")");
        }

        _keyId = keyId;
        _secret = keySecret;
        _scopes = scopes;
        _tokenUrl = Urls.Check("the token URL", tokenUrl ?? DefaultTokenUrl, originOnly: false);
        _ownsHttp = httpClient is null;
        _http = httpClient ?? Transport.NewHttpClient();
    }

    /// <summary>The key id this provider exchanges.</summary>
    public string KeyId => _keyId;

    /// <inheritdoc/>
    public async ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken)
    {
        var cached = _cache;
        if (cached is not null && cached.Fresh(DateTimeOffset.UtcNow))
        {
            return cached.Token;
        }

        var seen = Interlocked.Read(ref _generation);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            // Another call refreshed while this one waited: use what it got.
            cached = _cache;
            if (cached is not null && (cached.Fresh(DateTimeOffset.UtcNow) || Interlocked.Read(ref _generation) != seen))
            {
                return cached.Token;
            }

            var fresh = await ExchangeAsync(cancellationToken).ConfigureAwait(false);
            _cache = fresh;
            Interlocked.Increment(ref _generation);
            return fresh.Token;
        }
        finally
        {
            _gate.Release();
        }
    }

    /// <inheritdoc/>
    public ValueTask InvalidateAsync()
    {
        _cache = null;
        return ValueTask.CompletedTask;
    }

    /// <summary>Never the secret.</summary>
    /// <returns>The key id and a redacted secret.</returns>
    public override string ToString() => $"ClientCredentials({_keyId}, secret: <redacted>)";

    /// <inheritdoc/>
    public void Dispose()
    {
        _gate.Dispose();
        if (_ownsHttp)
        {
            _http.Dispose();
        }
    }

    private async Task<Cached> ExchangeAsync(CancellationToken cancellationToken)
    {
        var basic = Convert.ToBase64String(Encoding.UTF8.GetBytes(
            Uri.EscapeDataString(_keyId) + ":" + Uri.EscapeDataString(_secret)));
        for (var attempt = 0; ; attempt++)
        {
            using var request = new HttpRequestMessage(HttpMethod.Post, _tokenUrl)
            {
                Content = new FormUrlEncodedContent(new Dictionary<string, string>
                {
                    ["grant_type"] = "client_credentials",
                    ["audience"] = Audience,
                    ["scope"] = string.Join(' ', _scopes),
                }),
            };
            request.Headers.Authorization = new AuthenticationHeaderValue("Basic", basic);
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeout.CancelAfter(TokenTimeout);
            HttpResponseMessage response;
            try
            {
                response = await _http.SendAsync(request, HttpCompletionOption.ResponseContentRead, timeout.Token).ConfigureAwait(false);
            }
            catch (Exception e) when (e is HttpRequestException || (e is OperationCanceledException && !cancellationToken.IsCancellationRequested))
            {
                if (attempt < TokenRetries)
                {
                    await Task.Delay(Retry.Backoff(attempt), cancellationToken).ConfigureAwait(false);
                    continue;
                }

                throw new AuthException("the token endpoint: " + e.Message, string.Empty, e);
            }

            using (response)
            {
                var status = (int)response.StatusCode;
                if (Retry.RetryableStatus(status) && attempt < TokenRetries)
                {
                    await Task.Delay(Retry.RetryAfter(response.Headers.RetryAfter) ?? Retry.Backoff(attempt), cancellationToken).ConfigureAwait(false);
                    continue;
                }

                var text = await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
                if (status < 200 || status >= 300)
                {
                    var refusal = ParseObject(text);
                    var error = Str(refusal, "error");
                    error = error.Length > 0 ? error : "HTTP " + status.ToString(CultureInfo.InvariantCulture);
                    var description = Str(refusal, "error_description");
                    description = description.Length > 0 ? $" ({description})" : string.Empty;
                    throw new AuthException($"the token exchange for key {_keyId} failed: {error}{description}", error);
                }

                var answer = ParseObject(text);
                var access = Str(answer, "access_token");
                if (access.Length == 0)
                {
                    throw new AuthException("the token endpoint: the token answer could not be read");
                }

                var expiresIn = answer is { } a && a.TryGetProperty("expires_in", out var e) && e.TryGetInt64(out var n) ? n : 900;
                return new Cached(new Token(access, DateTimeOffset.UtcNow.AddSeconds(expiresIn)), DateTimeOffset.UtcNow, TimeSpan.FromSeconds(expiresIn));
            }
        }
    }

    private static JsonElement? ParseObject(string text)
    {
        try
        {
            using var doc = JsonDocument.Parse(text);
            return doc.RootElement.ValueKind == JsonValueKind.Object ? doc.RootElement.Clone() : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static string Str(JsonElement? e, string name) =>
        e is { } o && o.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() ?? string.Empty : string.Empty;

    private sealed record Cached(Token Token, DateTimeOffset Issued, TimeSpan Lifetime)
    {
        public bool Fresh(DateTimeOffset now) => now - Issued < Lifetime * 0.8;
    }
}
