using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Threading.Tasks;

namespace InOrbit.Sdk;

/// <summary>How to build a client: give <see cref="Token"/>, or <see cref="KeyId"/> with <see cref="KeySecret"/> and <see cref="Scopes"/>, or a <see cref="TokenProvider"/>.</summary>
public sealed record ClientOptions
{
    /// <summary>An API token (from the console or <c>iohr token create</c>).</summary>
    public string? Token { get; init; }

    /// <summary>An API key's id.</summary>
    public string? KeyId { get; init; }

    /// <summary>An API key's secret.</summary>
    public string? KeySecret { get; init; }

    /// <summary>The scopes to ask for with a key; there is no default.</summary>
    public IReadOnlyList<string>? Scopes { get; init; }

    /// <summary>Your own token source.</summary>
    public ITokenProvider? TokenProvider { get; init; }

    /// <summary>The API's origin (default <c>https://api.inorbit.hr</c>; plain HTTP only to this machine).</summary>
    public Uri? BaseUrl { get; init; }

    /// <summary>The token endpoint for a key (default <c>https://auth.inorbit.hr/oauth2/token</c>).</summary>
    public Uri? TokenUrl { get; init; }

    /// <summary>How long each attempt may take (default 30 seconds).</summary>
    public TimeSpan? Timeout { get; init; }

    /// <summary>Retries after the first attempt (default 2; 0 disables).</summary>
    public int? MaxRetries { get; init; }

    /// <summary>Appended to the user agent.</summary>
    public string? UserAgentSuffix { get; init; }

    /// <summary>Observers of every attempt.</summary>
    public IReadOnlyList<IHook>? Hooks { get; init; }

    /// <summary>The client to send requests with (default: one of its own, which follows no redirect).</summary>
    public HttpClient? HttpClient { get; init; }

    /// <summary>How streams open: server-sent events (the default) or every stream over one <c>/v1/ws</c> socket.</summary>
    public StreamTransport? Streams { get; init; }

    /// <summary>How long a stream may be silent (no event, no comment) before it fails with a timeout (default 45 seconds).</summary>
    public TimeSpan? StreamIdleTimeout { get; init; }

    /// <summary>Never a secret.</summary>
    /// <returns>Which credential is set, and the base URL.</returns>
    public override string ToString()
    {
        var credential = TokenProvider is not null ? "token provider" : Token is not null ? "token <redacted>" : KeyId is not null ? $"key {KeyId}, secret <redacted>" : "none";
        return $"ClientOptions({credential}, {BaseUrl?.ToString() ?? Transport.DefaultBaseUrl.ToString()})";
    }
}

/// <summary>How a client opens streams (design.md section 7).</summary>
public enum StreamTransport
{
    /// <summary>One server-sent events request per stream.</summary>
    Sse,

    /// <summary>Every stream of the client as a call on one multiplexed <c>/v1/ws</c> socket.</summary>
    Socket,
}

/// <summary>An HTTP method.</summary>
public enum Method
{
    /// <summary>GET.</summary>
    Get,

    /// <summary>POST.</summary>
    Post,

    /// <summary>PUT.</summary>
    Put,

    /// <summary>PATCH.</summary>
    Patch,

    /// <summary>DELETE.</summary>
    Delete,

    /// <summary>HEAD.</summary>
    Head,
}

/// <summary>One call, as a generated surface builds it (or you do, for a route the surface does not have).</summary>
public sealed class Operation
{
    /// <summary>A call of <paramref name="method"/> on <paramref name="path"/>.</summary>
    /// <param name="method">The method.</param>
    /// <param name="path">The path, parameters bound and encoded, starting with one <c>/</c> and without a query.</param>
    public Operation(Method method, string path)
    {
        ArgumentNullException.ThrowIfNull(path);
        Method = method;
        Path = path;
    }

    /// <summary>What hooks see (<c>radar.list_digests</c>); the path when not set.</summary>
    public string? Name { get; init; }

    /// <summary>The method.</summary>
    public Method Method { get; }

    /// <summary>The path.</summary>
    public string Path { get; }

    /// <summary>Query parameters, in order; a name may repeat.</summary>
    public IReadOnlyList<KeyValuePair<string, string>> Query { get; init; } = [];

    /// <summary>The JSON body, already encoded.</summary>
    public ReadOnlyMemory<byte>? Body { get; init; }

    /// <summary>The scopes the operation needs, for the record.</summary>
    public IReadOnlyList<string> Scopes { get; init; } = [];

    /// <summary>Retry it like an idempotent method although its method is not.</summary>
    public bool Idempotent { get; init; }

    /// <summary>The RPC a <c>/v1/ws</c> call frame names (<c>iohr.events.v1.EventsService/StreamEvents</c>); <see langword="null"/> when the operation has none.</summary>
    public string? Rpc { get; init; }

    /// <summary>The <c>/v1/ws</c> call frame's body: the path and query parameters as one JSON object.</summary>
    public ReadOnlyMemory<byte>? CallBody { get; init; }

    /// <summary>Whether a failed attempt may be retried.</summary>
    internal bool RetrySafe => Idempotent || Method is Method.Get or Method.Put or Method.Delete or Method.Head;

    /// <inheritdoc/>
    public override string ToString() => $"{Method.ToString().ToUpperInvariant()} {Path}";
}

/// <summary>A typed answer and the raw one beside it.</summary>
/// <typeparam name="T">The answer's model.</typeparam>
/// <param name="Value">The answer, typed.</param>
/// <param name="Raw">The answer as it came.</param>
public sealed record Response<T>(T Value, RawResponse Raw);

/// <summary>
/// A client for one credential, in the view of profile <typeparamref name="TProfile"/>. Share one
/// per credential; it holds no per-call state. <see cref="Client.FromEnv{TProfile}"/> builds one from the environment.
/// </summary>
/// <typeparam name="TProfile">The profile whose operations the surface lets this client call.</typeparam>
/// <example>
/// <code>
/// using var client = Client.FromEnv(); // INORBIT_TOKEN, or INORBIT_KEY_ID + _KEY_SECRET + _SCOPES
/// var me = await client.MeAsync();
/// </code>
/// </example>
public sealed class Client<TProfile> : IDisposable
    where TProfile : IProfile
{
    private readonly Transport _transport;

    /// <summary>A client with <paramref name="options"/>.</summary>
    /// <param name="options">The credential and settings.</param>
    /// <exception cref="ConfigException">No credential is given, a key has no scopes, or a URL is not https (plain http only to this machine).</exception>
    public Client(ClientOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        _transport = new Transport(options);
    }

    /// <summary>The API's origin this client calls.</summary>
    public Uri BaseUrl => _transport.BaseUrl;

    /// <summary>Calls <paramref name="operation"/> and reads its JSON answer as <typeparamref name="T"/>.</summary>
    /// <typeparam name="T">The answer's model.</typeparam>
    /// <param name="operation">The call.</param>
    /// <param name="cancellationToken">Cancels the call, retries included.</param>
    /// <returns>The typed answer and the raw one.</returns>
    /// <exception cref="ApiException">The API answered with an error.</exception>
    /// <exception cref="InOrbitException">The connection, a timeout, the token, the size or the decoding failed.</exception>
    public async Task<Response<T>> RequestAsync<T>(Operation operation, CancellationToken cancellationToken = default)
    {
        var raw = await SendAsync(operation, cancellationToken).ConfigureAwait(false);
        try
        {
            var value = raw.Body.Length == 0
                ? JsonSerializer.Deserialize<T>("{}"u8, Json.Options)
                : JsonSerializer.Deserialize<T>(raw.Body.Span, Json.Options);
            return new Response<T>(value!, raw);
        }
        catch (JsonException e)
        {
            throw new DecodeException(e.Message, raw, e);
        }
        catch (NotSupportedException e)
        {
            throw new DecodeException(e.Message, raw, e);
        }
    }

    /// <summary>
    /// Opens the stream <paramref name="operation"/> answers and yields each event as <typeparamref name="T"/>:
    /// over server-sent events, or as a call on the client's <c>/v1/ws</c> socket when
    /// <see cref="ClientOptions.Streams"/> says so (design.md section 7). The stream opens on the first
    /// step of the loop; leaving the loop, or <paramref name="cancellationToken"/>, closes it.
    /// </summary>
    /// <typeparam name="T">The event's model.</typeparam>
    /// <param name="operation">The streaming call.</param>
    /// <param name="cancellationToken">Stops the stream.</param>
    /// <returns>The events, in order.</returns>
    /// <exception cref="ApiException">The API refused the stream, or ended it with an error.</exception>
    /// <exception cref="InOrbitException">The connection, a timeout (silence past the idle timeout), the token, the size or the decoding failed.</exception>
    public IAsyncEnumerable<T> StreamAsync<T>(Operation operation, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(operation);
        return _transport.StreamAsync<T>(operation, cancellationToken);
    }

    /// <summary>Calls <paramref name="operation"/> and hands back the answer as it came, a 2xx one only.</summary>
    /// <param name="operation">The call.</param>
    /// <param name="cancellationToken">Cancels the call, retries included.</param>
    /// <returns>The answer.</returns>
    /// <exception cref="ApiException">The API answered with an error.</exception>
    /// <exception cref="InOrbitException">The connection, a timeout, the token or the size failed.</exception>
    public Task<RawResponse> SendAsync(Operation operation, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(operation);
        return _transport.SendAsync(operation, cancellationToken);
    }

    /// <inheritdoc/>
    public void Dispose() => _transport.Dispose();

    /// <summary>Never a secret.</summary>
    /// <returns>The profile and the base URL.</returns>
    public override string ToString() => $"Client<{TProfile.Name}>({BaseUrl})";
}

/// <summary>Builds clients from the environment.</summary>
public static class Client
{
    /// <summary>
    /// A client for profile <typeparamref name="TProfile"/> from the environment:
    /// <c>INORBIT_&lt;PROFILE&gt;_TOKEN</c>, or <c>INORBIT_&lt;PROFILE&gt;_KEY_ID</c>,
    /// <c>_KEY_SECRET</c> and <c>_SCOPES</c>, and nothing else for a named profile.
    /// <c>INORBIT_BASE_URL</c> and <c>INORBIT_TOKEN_URL</c> apply to every profile.
    /// </summary>
    /// <typeparam name="TProfile">The profile.</typeparam>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">No credential is there; the message names the variables to set.</exception>
    public static Client<TProfile> FromEnv<TProfile>()
        where TProfile : IProfile => new(Environmental.Options(TProfile.Env));

    /// <summary>
    /// A client for the public profile from the environment: <c>INORBIT_TOKEN</c>, or
    /// <c>INORBIT_KEY_ID</c>, <c>INORBIT_KEY_SECRET</c> and <c>INORBIT_SCOPES</c>.
    /// </summary>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">No credential is there; the message names the variables to set.</exception>
    public static Client<PublicProfile> FromEnv() => FromEnv<PublicProfile>();
}

/// <summary>The JSON settings every surface uses.</summary>
internal static class Json
{
    internal static readonly JsonSerializerOptions Options = new(JsonSerializerDefaults.Web)
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };
}

/// <summary>Reads a client's options from the environment.</summary>
internal static class Environmental
{
    internal static ClientOptions Options(string profile)
    {
        var prefix = string.IsNullOrEmpty(profile) ? "INORBIT_" : $"INORBIT_{profile}_";
        static string? Get(string name)
        {
            var v = Environment.GetEnvironmentVariable(name);
            return string.IsNullOrEmpty(v) ? null : v;
        }

        string? Own(string name) => Get(prefix + name);
        string? Shared(string name) => Own(name) ?? Get("INORBIT_" + name);
        Uri? Url(string name, string what)
        {
            var v = Shared(name);
            if (v is null)
            {
                return null;
            }

            return Uri.TryCreate(v, UriKind.Absolute, out var u) ? u : throw new ConfigException($"{what} in INORBIT_{name} is not a URL");
        }

        var urls = new ClientOptions { BaseUrl = Url("BASE_URL", "the base URL"), TokenUrl = Url("TOKEN_URL", "the token URL") };
        if (Own("TOKEN") is { } token)
        {
            return urls with { Token = token };
        }

        if (Own("KEY_ID") is { } keyId && Own("KEY_SECRET") is { } secret)
        {
            var scopes = (Own("SCOPES") ?? string.Empty).Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            if (scopes.Length == 0)
            {
                throw new ConfigException($"no scopes: set {prefix}SCOPES (space-separated, such as \"identity:read account:read\")");
            }

            return urls with { KeyId = keyId, KeySecret = secret, Scopes = scopes };
        }

        throw new ConfigException($"no credentials: set {prefix}TOKEN, or {prefix}KEY_ID and {prefix}KEY_SECRET");
    }
}

/// <summary>The one request path: the token, the attempt, the retries and the hooks (design.md sections 3 to 6).</summary>
internal sealed partial class Transport : IDisposable
{
    internal static readonly Uri DefaultBaseUrl = new("https://api.inorbit.hr");

    /// <summary>The largest answer read, 16 MiB.</summary>
    internal const int MaxBody = 16 * 1024 * 1024;

    private readonly ITokenProvider _provider;
    private readonly ClientCredentials? _ownedProvider;
    private readonly HttpClient _http;
    private readonly bool _ownsHttp;
    private readonly TimeSpan _timeout;
    private readonly int _maxRetries;
    private readonly string _userAgent;
    private readonly IReadOnlyList<IHook> _hooks;

    internal Transport(ClientOptions options)
    {
        BaseUrl = Urls.Check("the base URL", options.BaseUrl ?? DefaultBaseUrl, originOnly: true);
        _ownsHttp = options.HttpClient is null;
        _http = options.HttpClient ?? NewHttpClient();
        if (options.TokenProvider is not null)
        {
            _provider = options.TokenProvider;
        }
        else if (!string.IsNullOrEmpty(options.Token))
        {
            _provider = new StaticToken(options.Token);
        }
        else if (!string.IsNullOrEmpty(options.KeyId) && !string.IsNullOrEmpty(options.KeySecret))
        {
            if (options.Scopes is null || options.Scopes.Count == 0)
            {
                throw new ConfigException("no scopes: set INORBIT_SCOPES (space-separated, such as \"identity:read account:read\")");
            }

            var credentials = new ClientCredentials(options.KeyId, options.KeySecret, options.Scopes, options.TokenUrl, options.HttpClient);
            _provider = credentials;
            _ownedProvider = credentials;
        }
        else
        {
            throw new ConfigException("no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET");
        }

        _timeout = options.Timeout ?? TimeSpan.FromSeconds(30);
        _maxRetries = options.MaxRetries ?? 2;
        _userAgent = UserAgent(options.UserAgentSuffix);
        _hooks = options.Hooks ?? [];
        _streams = options.Streams ?? StreamTransport.Sse;
        _idle = options.StreamIdleTimeout ?? TimeSpan.FromSeconds(45);
        if (_idle <= TimeSpan.Zero)
        {
            throw new ConfigException("the stream idle timeout must be positive");
        }
    }

    internal Uri BaseUrl { get; }

    internal static HttpClient NewHttpClient() =>
        new(new SocketsHttpHandler { AllowAutoRedirect = false, ConnectTimeout = TimeSpan.FromSeconds(10) })
        {
            Timeout = System.Threading.Timeout.InfiniteTimeSpan,
        };

    public void Dispose()
    {
        _socket?.Dispose();
        _ownedProvider?.Dispose();
        if (_ownsHttp)
        {
            _http.Dispose();
        }
    }

    internal async Task<RawResponse> SendAsync(Operation op, CancellationToken cancellationToken) =>
        (await SendCoreAsync(op, stream: false, cancellationToken).ConfigureAwait(false)).Raw!;

    /// <summary>The attempts of one call: the answer, or for a stream the opened response, whose body is the caller's to read and dispose.</summary>
    private async Task<Outcome> SendCoreAsync(Operation op, bool stream, CancellationToken cancellationToken)
    {
        var url = Url(op);
        var id = Retry.RequestId();
        var retries = 0;
        var refreshed = false;
        for (var number = 1; ; number++)
        {
            var attempt = new Attempt(op.Name ?? op.Path, op.Method.ToString().ToUpperInvariant(), op.Path, number, id);
            Outcome outcome;
            try
            {
                outcome = await AttemptAsync(op, url, attempt, stream, cancellationToken).ConfigureAwait(false);
            }
            catch (InOrbitException e)
            {
                Failed(attempt, e);
                throw;
            }

            switch (outcome.Kind)
            {
                case OutcomeKind.Done:
                    return outcome;
                case OutcomeKind.Unauthorized when !refreshed:
                    await _provider.InvalidateAsync().ConfigureAwait(false);
                    refreshed = true;
                    continue;
                case OutcomeKind.Retry when op.RetrySafe && retries < _maxRetries:
                    await Task.Delay(outcome.Wait ?? Retry.Backoff(retries), cancellationToken).ConfigureAwait(false);
                    retries++;
                    continue;
                default:
                    var error = outcome.Raw is null ? outcome.Error! : ApiException.From(outcome.Raw);
                    Failed(attempt, error);
                    throw error;
            }
        }
    }

    private static string UserAgent(string? suffix)
    {
        var os = OperatingSystem.IsWindows() ? "windows" : OperatingSystem.IsMacOS() ? "macos" : OperatingSystem.IsLinux() ? "linux" : "unknown";
        var arch = RuntimeInformation.OSArchitecture.ToString().ToLowerInvariant();
        var ua = $"inorbithr-sdk-csharp/{SdkVersion.Value} dotnet/{Environment.Version} {os}/{arch}";
        return string.IsNullOrEmpty(suffix) ? ua : $"{ua} {suffix}";
    }

    private void Failed(Attempt attempt, InOrbitException error)
    {
        foreach (var h in _hooks)
        {
            h.OnError(attempt, error);
        }
    }

    private Uri Url(Operation op)
    {
        var p = op.Path;
        if (!p.StartsWith('/') || p.StartsWith("//", StringComparison.Ordinal) || p.Contains('?', StringComparison.Ordinal) || p.Contains('#', StringComparison.Ordinal))
        {
            throw new ConfigException($"the path \"{p}\" is not usable: it starts with one / and has no query");
        }

        var builder = new StringBuilder(BaseUrl.GetLeftPart(UriPartial.Authority)).Append(p);
        var first = true;
        foreach (var (name, value) in op.Query)
        {
            builder.Append(first ? '?' : '&').Append(Uri.EscapeDataString(name)).Append('=').Append(Uri.EscapeDataString(value));
            first = false;
        }

        return new Uri(builder.ToString());
    }

    private async Task<Outcome> AttemptAsync(Operation op, Uri url, Attempt attempt, bool stream, CancellationToken cancellationToken)
    {
        Token token;
        try
        {
            token = await _provider.GetTokenAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (InOrbitException)
        {
            throw;
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            throw new AuthException("the token provider failed: " + e.Message, string.Empty, e);
        }

        using var request = new HttpRequestMessage(ToHttp(op.Method), url);
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token.Access);
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue(stream ? "text/event-stream" : "application/json"));
        request.Headers.TryAddWithoutValidation("user-agent", _userAgent);
        request.Headers.TryAddWithoutValidation("x-request-id", attempt.RequestId);
        if (op.Body is { } body)
        {
            request.Content = new ReadOnlyMemoryContent(body);
            request.Content.Headers.ContentType = new MediaTypeHeaderValue("application/json");
        }

        foreach (var h in _hooks)
        {
            h.OnRequest(attempt);
        }

        using var timer = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timer.CancelAfter(_timeout);
        HttpResponseMessage response;
        try
        {
            response = await _http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, timer.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            return Outcome.Retry(null, new RequestTimeoutException(BaseUrl.Host, _timeout));
        }
        catch (HttpRequestException e)
        {
            return Outcome.Retry(null, new ConnectionException(BaseUrl.Host, e.Message, e));
        }

        if (stream && response.IsSuccessStatusCode)
        {
            // The body is the stream: handed over unread, the caller reads and disposes it.
            var opened = new RawResponse((int)response.StatusCode, Headers(response), [], attempt.RequestId, attempt.Number);
            foreach (var h in _hooks)
            {
                h.OnResponse(attempt, opened);
            }

            return Outcome.Opened(opened, response);
        }

        using (response)
        {
            if (response.Content.Headers.ContentLength > MaxBody)
            {
                throw new TooLargeException();
            }

            byte[] bytes;
            try
            {
                bytes = await ReadCappedAsync(response.Content, timer.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                return Outcome.Retry(null, new RequestTimeoutException(BaseUrl.Host, _timeout));
            }
            catch (Exception e) when (e is HttpRequestException or IOException)
            {
                return Outcome.Retry(null, new ConnectionException(BaseUrl.Host, e.Message, e));
            }

            var raw = new RawResponse((int)response.StatusCode, Headers(response), bytes, attempt.RequestId, attempt.Number);
            foreach (var h in _hooks)
            {
                h.OnResponse(attempt, raw);
            }

            if (raw.Status == 401)
            {
                return Outcome.Unauthorized(raw);
            }

            if (Retry.RetryableStatus(raw.Status))
            {
                return Outcome.Retry(raw, null, Retry.RetryAfter(response.Headers.RetryAfter));
            }

            if (raw.Status is >= 200 and < 300)
            {
                return Outcome.Done(raw);
            }

            throw ApiException.From(raw);
        }
    }

    private static async Task<byte[]> ReadCappedAsync(HttpContent content, CancellationToken cancellationToken)
    {
        var stream = await content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
        await using (stream.ConfigureAwait(false))
        {
            using var buffer = new MemoryStream();
            var chunk = new byte[81920];
            int read;
            while ((read = await stream.ReadAsync(chunk, cancellationToken).ConfigureAwait(false)) > 0)
            {
                if (buffer.Length + read > MaxBody)
                {
                    throw new TooLargeException();
                }

                buffer.Write(chunk, 0, read);
            }

            return buffer.ToArray();
        }
    }

    private static Dictionary<string, IReadOnlyList<string>> Headers(HttpResponseMessage response)
    {
        var all = new Dictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal);
        foreach (var (name, values) in response.Headers.Concat(response.Content.Headers))
        {
            all[name.ToLowerInvariant()] = values.ToArray();
        }

        return all;
    }

    private static HttpMethod ToHttp(Method m) => m switch
    {
        Method.Get => HttpMethod.Get,
        Method.Post => HttpMethod.Post,
        Method.Put => HttpMethod.Put,
        Method.Patch => HttpMethod.Patch,
        Method.Delete => HttpMethod.Delete,
        Method.Head => HttpMethod.Head,
        _ => throw new ArgumentOutOfRangeException(nameof(m)),
    };

    private enum OutcomeKind
    {
        Done,
        Unauthorized,
        Retry,
    }

    private sealed record Outcome(OutcomeKind Kind, RawResponse? Raw, InOrbitException? Error, TimeSpan? Wait, HttpResponseMessage? Response = null)
    {
        public static Outcome Done(RawResponse raw) => new(OutcomeKind.Done, raw, null, null);

        public static Outcome Opened(RawResponse raw, HttpResponseMessage response) => new(OutcomeKind.Done, raw, null, null, response);

        public static Outcome Unauthorized(RawResponse raw) => new(OutcomeKind.Unauthorized, raw, null, null);

        public static Outcome Retry(RawResponse? raw, InOrbitException? error, TimeSpan? wait = null) => new(OutcomeKind.Retry, raw, error, wait);
    }
}

/// <summary>The SDK's version, for the user agent.</summary>
internal static class SdkVersion
{
    internal static readonly string Value =
        typeof(SdkVersion).Assembly.GetName().Version?.ToString(3) ?? "0.0.0";
}
