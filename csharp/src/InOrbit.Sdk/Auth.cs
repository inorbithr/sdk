using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
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

/// <summary>How to build <see cref="ClientCredentials"/>: a key id, its secret or a file holding it, and the scopes.</summary>
public sealed record ClientCredentialsOptions
{
    /// <summary>The key's id (<c>ak_…</c>).</summary>
    public required string KeyId { get; init; }

    /// <summary>The key's secret. Give this or <see cref="KeySecretFile"/>.</summary>
    public string? KeySecret { get; init; }

    /// <summary>A file holding the secret, read before every token exchange, so a rotated secret is used at the next refresh.</summary>
    public string? KeySecretFile { get; init; }

    /// <summary>The scopes to ask for, a subset of the key's.</summary>
    public required IReadOnlyList<string> Scopes { get; init; }

    /// <summary>The token endpoint (default <see cref="ClientCredentials.DefaultTokenUrl"/>); plain HTTP only to this machine.</summary>
    public Uri? TokenUrl { get; init; }

    /// <summary>The client to send the exchange with (default: one of its own).</summary>
    public HttpClient? HttpClient { get; init; }

    /// <summary>Never the secret.</summary>
    /// <returns>The key id and a redacted secret.</returns>
    public override string ToString() => $"ClientCredentialsOptions({KeyId}, secret: <redacted>)";
}

/// <summary>
/// Exchanges an API key for short-lived tokens (OAuth client credentials), caches the
/// token, refreshes it when less than a fifth of its life is left or after a 401, and
/// makes one exchange however many calls wait for it (<see cref="CachedToken"/>).
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
    private readonly string? _secret;
    private readonly string? _secretFile;
    private readonly IReadOnlyList<string> _scopes;
    private readonly Uri _tokenUrl;
    private readonly HttpClient _http;
    private readonly bool _ownsHttp;
    private readonly string? _userAgent;
    private readonly CachedToken _cache;

    /// <summary>A provider for one key.</summary>
    /// <param name="keyId">The key's id (<c>ak_…</c>).</param>
    /// <param name="keySecret">The key's secret. It is never printed.</param>
    /// <param name="scopes">The scopes to ask for, a subset of the key's.</param>
    /// <param name="tokenUrl">The token endpoint (default <see cref="DefaultTokenUrl"/>); plain HTTP only to this machine.</param>
    /// <param name="httpClient">The client to send the exchange with (default: one of its own).</param>
    public ClientCredentials(string keyId, string keySecret, IReadOnlyList<string> scopes, Uri? tokenUrl = null, HttpClient? httpClient = null)
        : this(new ClientCredentialsOptions { KeyId = keyId, KeySecret = Required(keySecret), Scopes = scopes, TokenUrl = tokenUrl, HttpClient = httpClient })
    {
    }

    /// <summary>A provider for one key, its secret given or read from a file before every exchange.</summary>
    /// <param name="options">The key, its secret or secret file, and the scopes.</param>
    /// <exception cref="ConfigException">No scopes, or not exactly one of the secret and the secret file.</exception>
    public ClientCredentials(ClientCredentialsOptions options)
        : this(options, null, null)
    {
    }

    internal ClientCredentials(ClientCredentialsOptions options, string? userAgent, CachedToken.Callbacks? callbacks)
    {
        ArgumentNullException.ThrowIfNull(options);
        ArgumentException.ThrowIfNullOrEmpty(options.KeyId);
        ArgumentNullException.ThrowIfNull(options.Scopes);
        if (options.Scopes.Count == 0)
        {
            throw new ConfigException("no scopes: set INORBIT_SCOPES (space-separated, such as \"identity:read account:read\")");
        }

        if (string.IsNullOrEmpty(options.KeySecret) == string.IsNullOrEmpty(options.KeySecretFile))
        {
            throw new ConfigException("give the key's secret or a file holding it, one of the two");
        }

        _keyId = options.KeyId;
        _secret = options.KeySecret;
        _secretFile = string.IsNullOrEmpty(options.KeySecretFile) ? null : options.KeySecretFile;
        _scopes = options.Scopes;
        _tokenUrl = Urls.Check("the token URL", options.TokenUrl ?? DefaultTokenUrl, originOnly: false);
        _ownsHttp = options.HttpClient is null;
        _http = options.HttpClient ?? Transport.NewHttpClient();
        _userAgent = userAgent;
        _cache = new CachedToken(new Exchanger(this), callbacks);
    }

    /// <summary>The key id this provider exchanges.</summary>
    public string KeyId => _keyId;

    /// <inheritdoc/>
    public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken) => _cache.GetTokenAsync(cancellationToken);

    /// <inheritdoc/>
    public ValueTask InvalidateAsync() => _cache.InvalidateAsync();

    /// <summary>Never the secret.</summary>
    /// <returns>The key id and a redacted secret.</returns>
    public override string ToString() => $"ClientCredentials({_keyId}, secret: <redacted>)";

    /// <inheritdoc/>
    public void Dispose()
    {
        if (_ownsHttp)
        {
            _http.Dispose();
        }
    }

    private static string Required(string secret)
    {
        ArgumentException.ThrowIfNullOrEmpty(secret, "keySecret");
        return secret;
    }

    private async Task<Token> ExchangeAsync(CancellationToken cancellationToken)
    {
        var secret = _secret;
        if (_secretFile is not null)
        {
            try
            {
                secret = (await File.ReadAllTextAsync(_secretFile, cancellationToken).ConfigureAwait(false)).Trim();
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                throw new AuthException($"cannot read the key secret file {_secretFile}", string.Empty, e);
            }
        }

        var basic = Convert.ToBase64String(Encoding.UTF8.GetBytes(
            Uri.EscapeDataString(_keyId) + ":" + Uri.EscapeDataString(secret ?? string.Empty)));
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
            if (_userAgent is not null)
            {
                request.Headers.TryAddWithoutValidation("user-agent", _userAgent);
            }

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

                throw new AuthException("the token endpoint: " + Describe(e), string.Empty, e);
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
                return new Token(access, DateTimeOffset.UtcNow.AddSeconds(expiresIn));
            }
        }
    }

    /// <summary>A transport failure in words: the message and, for a TLS or socket failure, its cause.</summary>
    internal static string Describe(Exception e) =>
        e.InnerException is { } inner && e is HttpRequestException ? $"{e.Message} ({inner.Message})" : e.Message;

    internal static JsonElement? ParseObject(string text)
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

    internal static string Str(JsonElement? e, string name) =>
        e is { } o && o.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() ?? string.Empty : string.Empty;

    private sealed class Exchanger(ClientCredentials owner) : ITokenProvider
    {
        public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken) => new(owner.ExchangeAsync(cancellationToken));
    }
}

/// <summary>
/// Wraps any provider with the caching rules of docs/config.md section 5.3: the token is kept in memory
/// and refreshed when less than a fifth of its life is left; one refresh at a time, which a caller that
/// gives up does not cancel for the others; when a refresh fails and the token is still valid it is
/// used, and the next refresh is tried no sooner than 5 s later. A provider that reads a vault is an
/// <see cref="ITokenProvider"/> wrapped in this.
/// </summary>
public sealed class CachedToken : ITokenProvider
{
    private static readonly TimeSpan SoftExpiryRetry = TimeSpan.FromSeconds(5);
    private readonly ITokenProvider _source;
    private readonly Callbacks? _callbacks;
    private readonly object _gate = new();
    private Held? _held;
    private Task<Held>? _pending;
    private DateTimeOffset _quietUntil;

    /// <summary>Caches what <paramref name="source"/> hands out.</summary>
    /// <param name="source">The provider to ask for a fresh token.</param>
    public CachedToken(ITokenProvider source)
        : this(source, null)
    {
    }

    internal CachedToken(ITokenProvider source, Callbacks? callbacks)
    {
        ArgumentNullException.ThrowIfNull(source);
        _source = source;
        _callbacks = callbacks;
    }

    /// <inheritdoc/>
    public async ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken)
    {
        var now = DateTimeOffset.UtcNow;
        var held = _held;
        if (held is not null && now < held.RefreshAt)
        {
            return held.Token;
        }

        if (held is not null && held.Token.ExpiresAt > now && now < _quietUntil)
        {
            return held.Token;
        }

        Task<Held> pending;
        lock (_gate)
        {
            pending = _pending ??= FetchAsync();
        }

        try
        {
            var fresh = await pending.WaitAsync(cancellationToken).ConfigureAwait(false);
            _held = fresh;
            return fresh.Token;
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            var still = _held;
            if (still?.Token.ExpiresAt is { } exp && DateTimeOffset.UtcNow < exp)
            {
                _quietUntil = DateTimeOffset.UtcNow + SoftExpiryRetry;
                _callbacks?.OnRefreshFailed?.Invoke(e);
                return still.Token;
            }

            throw;
        }
    }

    /// <inheritdoc/>
    public async ValueTask InvalidateAsync()
    {
        _held = null;
        _quietUntil = default;
        await _source.InvalidateAsync().ConfigureAwait(false);
    }

    /// <summary>Never the token.</summary>
    /// <returns><c>CachedToken(&lt;redacted&gt;)</c>.</returns>
    public override string ToString() => "CachedToken(<redacted>)";

    private async Task<Held> FetchAsync()
    {
        await Task.Yield();
        try
        {
            var fetchedAt = DateTimeOffset.UtcNow;
            Token token;
            try
            {
                token = await _source.GetTokenAsync(CancellationToken.None).ConfigureAwait(false);
            }
            catch (Exception e)
            {
                _callbacks?.OnRefresh?.Invoke(e);
                throw;
            }

            _callbacks?.OnRefresh?.Invoke(null);
            var refreshAt = token.ExpiresAt is { } exp
                ? fetchedAt + TimeSpan.FromTicks((long)(Math.Max(0, (exp - fetchedAt).Ticks) * 0.8))
                : DateTimeOffset.MaxValue;
            return new Held(token, refreshAt);
        }
        finally
        {
            lock (_gate)
            {
                _pending = null;
            }
        }
    }

    /// <summary>What the client hears about refreshes: a metric per exchange, a log record when one failed.</summary>
    internal sealed record Callbacks(Action<Exception?>? OnRefresh, Action<Exception>? OnRefreshFailed);

    private sealed record Held(Token Token, DateTimeOffset RefreshAt);
}

/// <summary>
/// A bearer token read from a file (a mounted Kubernetes Secret, a Vault agent's sink): read at first
/// use, again when its modification time or size changes (checked at most once a minute), and at once
/// after a 401. A JWT's <c>exp</c> claim, read and not verified, is its expiry.
/// </summary>
public sealed class TokenFile : ITokenProvider
{
    private static readonly TimeSpan CheckEvery = TimeSpan.FromSeconds(60);
    private readonly object _gate = new();
    private Token? _token;
    private (DateTime, long) _stamp;
    private DateTimeOffset _checked;
    private bool _reread = true;

    /// <summary>A provider reading <paramref name="path"/>.</summary>
    /// <param name="path">The file.</param>
    public TokenFile(string path)
    {
        ArgumentException.ThrowIfNullOrEmpty(path);
        Path = path;
    }

    /// <summary>The file.</summary>
    public string Path { get; }

    /// <inheritdoc/>
    public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken)
    {
        // A small file, read under a lock: at most once a minute, and after a 401.
        lock (_gate)
        {
            var now = DateTimeOffset.UtcNow;
            var held = _token;
            if (!_reread && held is not null && now - _checked < CheckEvery)
            {
                return ValueTask.FromResult(held);
            }

            _checked = now;
            try
            {
                var info = new FileInfo(Path);
                if (!info.Exists)
                {
                    throw new FileNotFoundException("the file does not exist");
                }

                var stamp = (info.LastWriteTimeUtc, info.Length);
                if (_reread || stamp != _stamp || held is null)
                {
                    var access = File.ReadAllText(Path).Trim();
                    if (access.Length == 0)
                    {
                        throw new IOException("the file is empty");
                    }

                    _token = new Token(access, JwtExpiry(access));
                    _stamp = stamp;
                }
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                if (held is null || _reread)
                {
                    _token = null;
                    throw new AuthException($"cannot read the token file {Path}: {e.Message}", string.Empty, e);
                }
            }

            _reread = false;
            return ValueTask.FromResult(_token ?? held ?? throw new AuthException($"cannot read the token file {Path}"));
        }
    }

    /// <inheritdoc/>
    public ValueTask InvalidateAsync()
    {
        _reread = true;
        return ValueTask.CompletedTask;
    }

    /// <summary>Names the file, never the token.</summary>
    /// <returns><c>TokenFile(&lt;path&gt;)</c>.</returns>
    public override string ToString() => $"TokenFile({Path})";

    private static DateTimeOffset? JwtExpiry(string token)
    {
        var parts = token.Split('.');
        if (parts.Length != 3)
        {
            return null;
        }

        try
        {
            var b64 = parts[1].Replace('-', '+').Replace('_', '/');
            b64 = b64.PadRight((b64.Length + 3) / 4 * 4, '=');
            using var doc = JsonDocument.Parse(Convert.FromBase64String(b64));
            return doc.RootElement.ValueKind == JsonValueKind.Object && doc.RootElement.TryGetProperty("exp", out var exp) && exp.TryGetInt64(out var s)
                ? DateTimeOffset.FromUnixTimeSeconds(s)
                : null;
        }
        catch (Exception e) when (e is FormatException or JsonException or ArgumentOutOfRangeException)
        {
            return null;
        }
    }
}

/// <summary>
/// The <c>iohr</c> login (docs/config.md section 5.4): runs <c>iohr auth token --profile &lt;name&gt;
/// --format json</c> without a shell, standard input closed, within 10 s, and caches the token it prints
/// by <see cref="CachedToken"/>'s rules. Standard output is never logged.
/// </summary>
public sealed class CliToken : ITokenProvider
{
    private readonly string _profile;
    private readonly string _program;
    private readonly TimeSpan _timeout;
    private readonly CachedToken _cache;

    /// <summary>A provider for profile <paramref name="profile"/> of the command line.</summary>
    /// <param name="profile">The command line's profile (<c>iohr profile list</c>).</param>
    /// <param name="program">The command line to run (default <c>iohr</c> on <c>PATH</c>).</param>
    /// <param name="timeout">How long it may take (default 10 s).</param>
    public CliToken(string profile, string? program = null, TimeSpan? timeout = null)
        : this(profile, program, timeout, null)
    {
    }

    internal CliToken(string profile, string? program, TimeSpan? timeout, CachedToken.Callbacks? callbacks)
    {
        ArgumentException.ThrowIfNullOrEmpty(profile);
        _profile = profile;
        _program = string.IsNullOrEmpty(program) ? "iohr" : program;
        _timeout = timeout ?? TimeSpan.FromSeconds(10);
        _cache = new CachedToken(new Runner(this), callbacks);
    }

    /// <inheritdoc/>
    public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken) => _cache.GetTokenAsync(cancellationToken);

    /// <inheritdoc/>
    public ValueTask InvalidateAsync() => _cache.InvalidateAsync();

    /// <summary>Names the profile, never the token.</summary>
    /// <returns><c>CliToken(&lt;profile&gt;)</c>.</returns>
    public override string ToString() => $"CliToken({_profile})";

    private async Task<Token> RunAsync()
    {
        var start = new ProcessStartInfo(_program)
        {
            ArgumentList = { "auth", "token", "--profile", _profile, "--format", "json" },
            UseShellExecute = false,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true,
        };
        Process process;
        try
        {
            process = Process.Start(start) ?? throw new AuthException($"the iohr login: cannot run {_program}");
        }
        catch (Exception e) when (e is System.ComponentModel.Win32Exception or InvalidOperationException or IOException)
        {
            throw new AuthException($"the iohr login: cannot run {_program}", string.Empty, e);
        }

        using (process)
        {
            process.StandardInput.Close();
            var stdout = process.StandardOutput.ReadToEndAsync();
            var stderr = process.StandardError.ReadToEndAsync();
            using var timer = new CancellationTokenSource(_timeout);
            try
            {
                await process.WaitForExitAsync(timer.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                try
                {
                    process.Kill(entireProcessTree: true);
                }
                catch (InvalidOperationException)
                {
                    // It ended on its own meanwhile.
                }

                throw new AuthException(string.Create(CultureInfo.InvariantCulture, $"the iohr login: {_program} did not answer within {_timeout.TotalSeconds:0} s"));
            }

            var output = await stdout.ConfigureAwait(false);
            var error = await stderr.ConfigureAwait(false);
            if (process.ExitCode != 0)
            {
                var line = error.Split('\n')[0].Trim();
                var first = line.Length > 200 ? line[..200] : line;
                var code = process.ExitCode.ToString(CultureInfo.InvariantCulture);
                throw new AuthException($"the iohr login for profile {_profile} failed (exit {code}): {(first.Length > 0 ? first : "no message")}", "exit_" + code);
            }

            // Standard output holds the token: it is parsed, never quoted in an error.
            var answer = ClientCredentials.ParseObject(output);
            var access = ClientCredentials.Str(answer, "access_token");
            if (access.Length == 0)
            {
                throw new AuthException("the iohr login: iohr auth token printed no token");
            }

            var expires = ClientCredentials.Str(answer, "expires_at");
            return new Token(access, DateTimeOffset.TryParse(expires, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out var at) ? at : null);
        }
    }

    private sealed class Runner(CliToken owner) : ITokenProvider
    {
        public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken) => new(owner.RunAsync());
    }
}

/// <summary>Tries providers in order and keeps the first that hands out a token; when none does, the <see cref="AuthException"/> lists why each failed.</summary>
public sealed class ChainedCredential : ITokenProvider
{
    private readonly ITokenProvider[] _providers;
    private ITokenProvider? _chosen;

    /// <summary>A chain of <paramref name="providers"/>, tried in this order.</summary>
    /// <param name="providers">The providers.</param>
    public ChainedCredential(params ITokenProvider[] providers)
    {
        ArgumentNullException.ThrowIfNull(providers);
        _providers = providers;
    }

    /// <inheritdoc/>
    public async ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken)
    {
        if (_chosen is { } chosen)
        {
            return await chosen.GetTokenAsync(cancellationToken).ConfigureAwait(false);
        }

        var failures = new List<string>();
        foreach (var p in _providers)
        {
            try
            {
                var t = await p.GetTokenAsync(cancellationToken).ConfigureAwait(false);
                _chosen = p;
                return t;
            }
            catch (Exception e) when (e is not OperationCanceledException)
            {
                failures.Add($"{p}: {e.Message}");
            }
        }

        throw new AuthException("no credential in the chain gave a token; tried:\n  " + string.Join("\n  ", failures));
    }

    /// <inheritdoc/>
    public ValueTask InvalidateAsync() => _chosen?.InvalidateAsync() ?? ValueTask.CompletedTask;

    /// <summary>Never a token.</summary>
    /// <returns>How many providers the chain holds.</returns>
    public override string ToString() => $"ChainedCredential({_providers.Length})";
}
