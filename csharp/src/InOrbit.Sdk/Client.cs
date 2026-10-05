using System;
using System.Collections.Generic;
using System.Linq;
using System.Net.Http;
using System.Runtime.InteropServices;
using System.Security.Cryptography.X509Certificates;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;

namespace InOrbit.Sdk;

/// <summary>
/// How to build a client: give <see cref="Token"/>, or <see cref="KeyId"/> with <see cref="KeySecret"/>
/// (or <see cref="KeySecretFile"/>) and <see cref="Scopes"/>, a <see cref="TokenFile"/>, or a
/// <see cref="TokenProvider"/>. Every setting of docs/config.md section 3 is here;
/// <see cref="Client.Load(ClientOptions?, LoadOptions?)"/> also reads them from the environment and the
/// config file, and an option set here always wins.
/// </summary>
public sealed record ClientOptions
{
    /// <summary>An API token (from the console or <c>iohr token create</c>).</summary>
    public string? Token { get; init; }

    /// <summary>A file holding an API token, read again when it changes and after a 401.</summary>
    public string? TokenFile { get; init; }

    /// <summary>An API key's id.</summary>
    public string? KeyId { get; init; }

    /// <summary>An API key's secret.</summary>
    public string? KeySecret { get; init; }

    /// <summary>A file holding the key's secret, read before every token exchange.</summary>
    public string? KeySecretFile { get; init; }

    /// <summary>The scopes to ask for with a key; there is no default.</summary>
    public IReadOnlyList<string>? Scopes { get; init; }

    /// <summary>Your own token source.</summary>
    public ITokenProvider? TokenProvider { get; init; }

    /// <summary>The API's origin (default <c>https://api.inorbit.hr</c>; plain HTTP only to this machine).</summary>
    public Uri? BaseUrl { get; init; }

    /// <summary>The token endpoint for a key (default <c>https://auth.inorbit.hr/oauth2/token</c>).</summary>
    public Uri? TokenUrl { get; init; }

    /// <summary>
    /// The config file's profile, for <see cref="Client.Load(ClientOptions?, LoadOptions?)"/> (default
    /// <c>INORBIT_PROFILE</c>, then the file's <c>default</c>). Explicit construction reads no file.
    /// </summary>
    public string? Profile { get; init; }

    /// <summary>The config file, or <c>off</c> for none, for <see cref="Client.Load(ClientOptions?, LoadOptions?)"/> (default <c>INORBIT_CONFIG_FILE</c>, then the <c>iohr</c> location).</summary>
    public string? ConfigFile { get; init; }

    /// <summary>Reserved until the API offers regions; setting it is a <see cref="ConfigException"/>.</summary>
    public string? Region { get; init; }

    /// <summary>Which credential sources <c>Load</c> may use: <c>env</c>, <c>workload</c>, <c>file</c>, <c>cli</c> (default all four).</summary>
    public IReadOnlyList<string>? CredentialSources { get; init; }

    /// <summary>The <c>iohr</c> command line the <c>cli</c> source runs (default <c>iohr</c> on <c>PATH</c>).</summary>
    public string? CliPath { get; init; }

    /// <summary>How long a new connection may take, TLS included (default 10 seconds).</summary>
    public TimeSpan? ConnectTimeout { get; init; }

    /// <summary>How long each attempt may take, the whole answer included (default 30 seconds).</summary>
    public TimeSpan? Timeout { get; init; }

    /// <summary>How long one call may take, every attempt and wait included (default 120 seconds).</summary>
    public TimeSpan? TotalTimeout { get; init; }

    /// <summary>Retries after the first attempt (default 2; 0 disables).</summary>
    public int? MaxRetries { get; init; }

    /// <summary>The backoff's base (default 500 ms), with full jitter.</summary>
    public TimeSpan? RetryBaseDelay { get; init; }

    /// <summary>The backoff's cap (default 8 seconds).</summary>
    public TimeSpan? RetryMaxDelay { get; init; }

    /// <summary>The longest <c>Retry-After</c> waited for (default 60 seconds); a longer one ends the call with the error.</summary>
    public TimeSpan? RetryAfterMax { get; init; }

    /// <summary>The per-client retry quota, a token bucket (default <see langword="true"/>).</summary>
    public bool? RetryBudget { get; init; }

    /// <summary>The retry quota's capacity (default 500); for tests.</summary>
    public int? RetryBudgetCapacity { get; init; }

    /// <summary>A proxy URL (<c>http://</c> or <c>https://</c>), or <c>off</c> for none, the standard variables included.</summary>
    public string? Proxy { get; init; }

    /// <summary>Hosts reached without the proxy (docs/config.md section 6.2).</summary>
    public IReadOnlyList<string>? NoProxy { get; init; }

    /// <summary>PEM certificates added to the system's trust store.</summary>
    public string? CaBundle { get; init; }

    /// <summary><see langword="false"/> trusts <see cref="CaBundle"/> only (default <see langword="true"/>).</summary>
    public bool? SystemTrust { get; init; }

    /// <summary>A PEM client certificate chain for mTLS.</summary>
    public string? ClientCert { get; init; }

    /// <summary>The PEM private key for <see cref="ClientCert"/>.</summary>
    public string? ClientKey { get; init; }

    /// <summary>The password of an encrypted <see cref="ClientKey"/> (PKCS#8).</summary>
    public string? ClientKeyPassword { get; init; }

    /// <summary>Client certificates with their keys (PKCS#12, the OS store, an HSM), instead of <see cref="ClientCert"/> and <see cref="ClientKey"/>.</summary>
    public X509Certificate2Collection? ClientCertificates { get; init; }

    /// <summary>Base64 SHA-256 hashes of pinned public keys (SPKI), at least two.</summary>
    public IReadOnlyList<string>? PinnedKeys { get; init; }

    /// <summary>The log level: <see cref="LogLevel.None"/> (the default), <c>Error</c>, <c>Warning</c>, <c>Information</c> or <c>Debug</c>.</summary>
    public LogLevel? Log { get; init; }

    /// <summary>Log allowlisted header values at <c>Debug</c> (default <see langword="false"/>).</summary>
    public bool? LogHeaders { get; init; }

    /// <summary>Header names added to the logging allowlist; the never-logged ones cannot be added.</summary>
    public IReadOnlyList<string>? LogAllowHeaders { get; init; }

    /// <summary>Where records go: a logger of category <c>InOrbit.Sdk</c>. Without one nothing is logged.</summary>
    public ILoggerFactory? LoggerFactory { get; init; }

    /// <summary>Sees every log record last, and returns it changed, or <see langword="null"/> to drop it.</summary>
    public Func<LogRecord, LogRecord?>? Redact { get; init; }

    /// <summary>Spans for calls and attempts through the <c>InOrbit.Sdk</c> <see cref="System.Diagnostics.ActivitySource"/> (default on; inert until a listener subscribes).</summary>
    public bool? Tracing { get; init; }

    /// <summary>Metrics through the <c>InOrbit.Sdk</c> <see cref="System.Diagnostics.Metrics.Meter"/> (default: as <see cref="Tracing"/>).</summary>
    public bool? Metrics { get; init; }

    /// <summary>What to do with rate-limit headers (default <see cref="RateLimitMode.Observe"/>).</summary>
    public RateLimitMode? RateLimit { get; init; }

    /// <summary>Appended to the user agent: product tokens, at most 128 characters.</summary>
    public string? UserAgentSuffix { get; init; }

    /// <summary>Observers of every attempt.</summary>
    public IReadOnlyList<IHook>? Hooks { get; init; }

    /// <summary>Edits the pipeline: add, insert, replace or remove middlewares by name.</summary>
    public Action<Pipeline>? Pipeline { get; init; }

    /// <summary>The client to send requests with (default: one of its own, which follows no redirect). Proxy, trust, mTLS, pinning and the connect timeout then belong to it.</summary>
    public HttpClient? HttpClient { get; init; }

    /// <summary>The handler to send requests with (a <c>SocketsHttpHandler</c> you configured), not disposed by the client. Proxy, trust, mTLS, pinning and the connect timeout then belong to it.</summary>
    public HttpMessageHandler? HttpMessageHandler { get; init; }

    /// <summary>How streams open: server-sent events (the default) or every stream over one <c>/v1/ws</c> socket.</summary>
    public StreamTransport? Streams { get; init; }

    /// <summary>How long a stream may be silent (no event, no comment) before it fails with a timeout (default 45 seconds).</summary>
    public TimeSpan? StreamIdleTimeout { get; init; }

    /// <summary>Never a secret.</summary>
    /// <returns>Which credential is set, and the base URL.</returns>
    public override string ToString()
    {
        var credential = TokenProvider is not null ? "token provider" : Token is not null ? "token <redacted>" : TokenFile is not null ? $"token file {TokenFile}" : KeyId is not null ? $"key {KeyId}, secret <redacted>" : "none";
        return $"ClientOptions({credential}, {BaseUrl?.ToString() ?? Transport.DefaultBaseUrl.ToString()})";
    }

    /// <summary>The options set here, by catalogue name, as resolution reads them.</summary>
    internal Dictionary<string, object> Code()
    {
        var code = new Dictionary<string, object>(StringComparer.Ordinal);
        void Put(string name, object? value)
        {
            if (value is not null)
            {
                code[name] = value;
            }
        }

        Put("base_url", BaseUrl?.OriginalString);
        Put("token_url", TokenUrl?.OriginalString);
        Put("region", Region);
        Put("key_id", KeyId);
        Put("key_secret", KeySecret);
        Put("key_secret_file", KeySecretFile);
        Put("scopes", Scopes?.ToArray());
        Put("token", Token);
        Put("token_file", TokenFile);
        Put("credential_sources", CredentialSources?.ToArray());
        Put("cli_path", CliPath);
        Put("connect_timeout", ConnectTimeout);
        Put("timeout", Timeout);
        Put("total_timeout", TotalTimeout);
        Put("stream_idle_timeout", StreamIdleTimeout);
        Put("max_retries", MaxRetries);
        Put("retry_base_delay", RetryBaseDelay);
        Put("retry_max_delay", RetryMaxDelay);
        Put("retry_after_max", RetryAfterMax);
        Put("retry_budget", RetryBudget);
        Put("streams", Streams is { } s ? (s == StreamTransport.Socket ? "socket" : "sse") : null);
        Put("proxy", Proxy);
        Put("no_proxy", NoProxy?.ToArray());
        Put("ca_bundle", CaBundle);
        Put("system_trust", SystemTrust);
        Put("client_cert", ClientCert);
        Put("client_key", ClientKey);
        Put("client_key_password", ClientKeyPassword);
        Put("pinned_keys", PinnedKeys?.ToArray());
        Put("log", Log is { } l ? InOrbit.Sdk.Log.Name(l) : null);
        Put("log_headers", LogHeaders);
        Put("log_allow_headers", LogAllowHeaders?.ToArray());
        Put("tracing", Tracing);
        Put("metrics", Metrics);
        Put("rate_limit", RateLimit switch { RateLimitMode.Wait => "wait", RateLimitMode.Off => "off", RateLimitMode.Observe => "observe", _ => null });
        Put("user_agent_suffix", UserAgentSuffix);
        Put("profile", Profile);
        Put("config_file", ConfigFile);
        return code;
    }
}

/// <summary>Options for one call, given with <see cref="Client{TProfile}.WithOptions(CallOptions)"/>.</summary>
public sealed record CallOptions
{
    /// <summary>How long the call may take: it replaces the attempt timeout and shortens the total timeout, never extends it.</summary>
    public TimeSpan? Timeout { get; init; }

    /// <summary>
    /// The <c>Idempotency-Key</c> to send, on an operation that takes one (default: a fresh UUID per call).
    /// Reuse it to repeat a call safely. On any other operation it is a <see cref="ConfigException"/>.
    /// </summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>A W3C <c>traceparent</c> to continue: the call span's parent, or sent as is when tracing is off.</summary>
    public string? Traceparent { get; init; }
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

    /// <summary>The path template (<c>/v1/radar/digests/{digest_id}</c>), which attempt spans are named for.</summary>
    public string? Template { get; init; }

    /// <summary>Query parameters, in order; a name may repeat.</summary>
    public IReadOnlyList<KeyValuePair<string, string>> Query { get; init; } = [];

    /// <summary>The JSON body, already encoded.</summary>
    public ReadOnlyMemory<byte>? Body { get; init; }

    /// <summary>The scopes the operation needs, for the record.</summary>
    public IReadOnlyList<string> Scopes { get; init; } = [];

    /// <summary>Retry it like an idempotent method although its method is not.</summary>
    public bool Idempotent { get; init; }

    /// <summary>
    /// The operation takes an <c>Idempotency-Key</c> (docs/config.md section 7.5): one is sent with every
    /// attempt, so the write is retried like a read.
    /// </summary>
    public bool TakesIdempotencyKey { get; init; }

    /// <summary>The RPC a <c>/v1/ws</c> call frame names (<c>iohr.events.v1.EventsService/StreamEvents</c>); <see langword="null"/> when the operation has none.</summary>
    public string? Rpc { get; init; }

    /// <summary>The <c>/v1/ws</c> call frame's body: the path and query parameters as one JSON object.</summary>
    public ReadOnlyMemory<byte>? CallBody { get; init; }

    /// <summary>Whether a failed attempt may be retried.</summary>
    internal bool RetrySafe => Idempotent || TakesIdempotencyKey || Method is Method.Get or Method.Put or Method.Delete or Method.Head;

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
/// per credential; it holds no per-call state. <see cref="Client.Load{TProfile}(ClientOptions?, LoadOptions?)"/>
/// builds one from code, the environment, the config file and the <c>iohr</c> login.
/// </summary>
/// <typeparam name="TProfile">The profile whose operations the surface lets this client call.</typeparam>
/// <example>
/// <code>
/// using var client = Client.Load(); // the environment, the iohr config file, the iohr login
/// var me = await client.MeAsync();
/// </code>
/// </example>
public sealed class Client<TProfile> : IDisposable
    where TProfile : IProfile
{
    private readonly Transport _transport;
    private readonly CallOptions? _call;
    private readonly bool _owner;

    /// <summary>A client with <paramref name="options"/> and nothing else: no environment, no file. For libraries and tests; applications use <see cref="Client.Load{TProfile}(ClientOptions?, LoadOptions?)"/>.</summary>
    /// <param name="options">The credential and settings.</param>
    /// <exception cref="ConfigException">No credential is given, a key has no scopes, a URL is not https (plain http only to this machine), or a setting is not usable.</exception>
    public Client(ClientOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        _transport = Transport.Explicit(options);
        _owner = true;
    }

    internal Client(Transport transport, CallOptions? call, bool owner)
    {
        _transport = transport;
        _call = call;
        _owner = owner;
    }

    /// <summary>The API's origin this client calls.</summary>
    public Uri BaseUrl => _transport.BaseUrl;

    /// <summary>What this client uses and where each value came from (<c>Describe()</c>), secrets redacted.</summary>
    public ResolvedConfig Config => _transport.Config;

    /// <summary>The latest rate-limit snapshot any answer carried, or <see langword="null"/>.</summary>
    public RateLimit? RateLimit => _transport.Latest;

    /// <summary>
    /// This client with <paramref name="options"/> for every call made through it: a caller's own
    /// idempotency key, a <c>traceparent</c>, a timeout. It shares the connection pool and the token;
    /// disposing it does nothing.
    /// </summary>
    /// <param name="options">The per-call options.</param>
    /// <returns>The client, with the options.</returns>
    /// <example>
    /// <code>
    /// await client.WithOptions(new CallOptions { IdempotencyKey = "order-42" }).Events().CreateEndpointAsync(body);
    /// </code>
    /// </example>
    public Client<TProfile> WithOptions(CallOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        return new Client<TProfile>(_transport, options, owner: false);
    }

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
        return _transport.StreamAsync<T>(operation, _call, cancellationToken);
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
        return _transport.SendAsync(operation, _call, cancellationToken);
    }

    /// <inheritdoc/>
    public void Dispose()
    {
        if (_owner)
        {
            _transport.Dispose();
        }
    }

    /// <summary>Never a secret.</summary>
    /// <returns>The profile and the base URL.</returns>
    public override string ToString() => $"Client<{TProfile.Name}>({BaseUrl})";
}

/// <summary>Builds clients: <see cref="Load(ClientOptions?, LoadOptions?)"/> for applications, <see cref="FromEnv()"/> as before.</summary>
public static class Client
{
    /// <summary>
    /// A client for the public profile configured from code, the environment, the <c>iohr</c> config file
    /// and the <c>iohr</c> login, each setting from the first that sets it (docs/config.md section 2).
    /// Reads files and the environment now; contacts no host until the first call.
    /// </summary>
    /// <param name="options">Options set in code; they always win.</param>
    /// <param name="loadOptions">The environment, OS and home directory to resolve with, instead of the process's.</param>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">Every problem found, or every credential source tried when none had credentials.</exception>
    /// <example>
    /// <code>
    /// using var client = Client.Load(new ClientOptions { Timeout = TimeSpan.FromSeconds(5) });
    /// </code>
    /// </example>
    public static Client<PublicProfile> Load(ClientOptions? options = null, LoadOptions? loadOptions = null) => Load<PublicProfile>(options, loadOptions);

    /// <summary>
    /// A client for profile <typeparamref name="TProfile"/>, configured as <see cref="Load(ClientOptions?, LoadOptions?)"/>
    /// does. A generated profile is its own configuration profile: <c>INORBIT_&lt;P&gt;_*</c> first (credentials only
    /// there), then <c>INORBIT_*</c>, the table <c>[profiles.&lt;p&gt;]</c>, and its <c>iohr</c> login; neither
    /// <see cref="ClientOptions.Profile"/> nor <c>INORBIT_PROFILE</c> can point it elsewhere.
    /// </summary>
    /// <typeparam name="TProfile">The profile.</typeparam>
    /// <param name="options">Options set in code; they always win.</param>
    /// <param name="loadOptions">The environment, OS and home directory to resolve with, instead of the process's.</param>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">Every problem found, or every credential source tried when none had credentials.</exception>
    public static Client<TProfile> Load<TProfile>(ClientOptions? options = null, LoadOptions? loadOptions = null)
        where TProfile : IProfile
    {
        options ??= new ClientOptions();
        var profileType = string.IsNullOrEmpty(TProfile.Env) ? null : TProfile.Name;
        if (profileType is not null && options.Profile is not null)
        {
            throw new ConfigException($"profile {TProfile.Name} is its own configuration profile; leave ClientOptions.Profile out");
        }

        var res = Transport.Resolve(options, profileType, loadOptions, explicitly: false);
        return new Client<TProfile>(new Transport(options, res, explicitly: false), null, owner: true);
    }

    /// <summary><see cref="Load(ClientOptions?, LoadOptions?)"/> off the calling thread: the file reads and the check for <c>iohr</c> run on the thread pool.</summary>
    /// <param name="options">Options set in code; they always win.</param>
    /// <param name="loadOptions">The environment, OS and home directory to resolve with, instead of the process's.</param>
    /// <param name="cancellationToken">Cancels the wait.</param>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">Every problem found, or every credential source tried when none had credentials.</exception>
    public static Task<Client<PublicProfile>> LoadAsync(ClientOptions? options = null, LoadOptions? loadOptions = null, CancellationToken cancellationToken = default) =>
        LoadAsync<PublicProfile>(options, loadOptions, cancellationToken);

    /// <summary><see cref="Load{TProfile}(ClientOptions?, LoadOptions?)"/> off the calling thread.</summary>
    /// <typeparam name="TProfile">The profile.</typeparam>
    /// <param name="options">Options set in code; they always win.</param>
    /// <param name="loadOptions">The environment, OS and home directory to resolve with, instead of the process's.</param>
    /// <param name="cancellationToken">Cancels the wait.</param>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">Every problem found, or every credential source tried when none had credentials.</exception>
    public static Task<Client<TProfile>> LoadAsync<TProfile>(ClientOptions? options = null, LoadOptions? loadOptions = null, CancellationToken cancellationToken = default)
        where TProfile : IProfile => Task.Run(() => Load<TProfile>(options, loadOptions), cancellationToken);

    /// <summary>
    /// Resolves a configuration as <see cref="Load(ClientOptions?, LoadOptions?)"/> would, without building a
    /// client: what a program would use and where each value came from.
    /// </summary>
    /// <param name="options">Options set in code.</param>
    /// <param name="loadOptions">The environment, OS and home directory to resolve with.</param>
    /// <returns>The configuration.</returns>
    /// <exception cref="ConfigException">Every problem found.</exception>
    public static ResolvedConfig LoadConfig(ClientOptions? options = null, LoadOptions? loadOptions = null) =>
        LoadConfig<PublicProfile>(options, loadOptions);

    /// <summary>Resolves a configuration as <see cref="Load{TProfile}(ClientOptions?, LoadOptions?)"/> would, without building a client.</summary>
    /// <typeparam name="TProfile">The profile.</typeparam>
    /// <param name="options">Options set in code.</param>
    /// <param name="loadOptions">The environment, OS and home directory to resolve with.</param>
    /// <returns>The configuration.</returns>
    /// <exception cref="ConfigException">Every problem found.</exception>
    public static ResolvedConfig LoadConfig<TProfile>(ClientOptions? options = null, LoadOptions? loadOptions = null)
        where TProfile : IProfile
    {
        options ??= new ClientOptions();
        var res = Transport.Resolve(options, string.IsNullOrEmpty(TProfile.Env) ? null : TProfile.Name, loadOptions, explicitly: false);
        return res.Describe(Transport.PipelineNames(options));
    }

    /// <summary>
    /// A client for profile <typeparamref name="TProfile"/> from the environment:
    /// <c>INORBIT_&lt;PROFILE&gt;_TOKEN</c>, or <c>INORBIT_&lt;PROFILE&gt;_KEY_ID</c>,
    /// <c>_KEY_SECRET</c> and <c>_SCOPES</c>, and nothing else for a named profile.
    /// <c>INORBIT_BASE_URL</c> and <c>INORBIT_TOKEN_URL</c> apply to every profile. Kept as it is;
    /// <see cref="Load{TProfile}(ClientOptions?, LoadOptions?)"/> reads more and is the one to use.
    /// </summary>
    /// <typeparam name="TProfile">The profile.</typeparam>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">No credential is there; the message names the variables to set.</exception>
    public static Client<TProfile> FromEnv<TProfile>()
        where TProfile : IProfile => new(Environmental.Options(TProfile.Env));

    /// <summary>
    /// A client for the public profile from the environment: <c>INORBIT_TOKEN</c>, or
    /// <c>INORBIT_KEY_ID</c>, <c>INORBIT_KEY_SECRET</c> and <c>INORBIT_SCOPES</c>. Kept as it is;
    /// <see cref="Load(ClientOptions?, LoadOptions?)"/> reads more and is the one to use.
    /// </summary>
    /// <returns>The client.</returns>
    /// <exception cref="ConfigException">No credential is there; the message names the variables to set.</exception>
    public static Client<PublicProfile> FromEnv() => FromEnv<PublicProfile>();
}

/// <summary>
/// The credential chain of docs/config.md section 5.1 as an <see cref="ITokenProvider"/>: code, the
/// environment, (workload identity, reserved), the config file, then the <c>iohr</c> login, decided
/// now. For a program that builds its own client or chain.
/// </summary>
public sealed class DefaultCredential : ITokenProvider, IDisposable
{
    private readonly ITokenProvider _provider;
    private readonly HttpClient _http;
    private readonly bool _ownsHttp;

    /// <summary>Resolves the chain as <see cref="Client.Load(ClientOptions?, LoadOptions?)"/> would with <paramref name="options"/>.</summary>
    /// <param name="options">Options set in code.</param>
    /// <param name="loadOptions">The environment, OS and home directory to resolve with.</param>
    /// <exception cref="ConfigException">Every source tried, when none has credentials.</exception>
    public DefaultCredential(ClientOptions? options = null, LoadOptions? loadOptions = null)
    {
        options ??= new ClientOptions();
        var res = Transport.Resolve(options, null, loadOptions, explicitly: false);
        Config = res.Describe(Transport.PipelineNames(options));
        _ownsHttp = options.HttpClient is null;
        _http = options.HttpClient ?? Transport.NewHttpClient();
        _provider = Transport.ProviderFor(res, options, _http, Transport.UserAgent(res.Get<string>("user_agent_suffix")), null);
    }

    /// <summary>Which source was chosen and what was tried.</summary>
    public ResolvedConfig Config { get; }

    /// <inheritdoc/>
    public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken) => _provider.GetTokenAsync(cancellationToken);

    /// <inheritdoc/>
    public ValueTask InvalidateAsync() => _provider.InvalidateAsync();

    /// <inheritdoc/>
    public void Dispose()
    {
        if (_ownsHttp)
        {
            _http.Dispose();
        }
    }

    /// <summary>Never a token.</summary>
    /// <returns>The source chosen.</returns>
    public override string ToString() => $"DefaultCredential({Config.Describe()["credential"]?["source"]})";
}

/// <summary>The JSON settings every surface uses.</summary>
internal static class Json
{
    internal static readonly JsonSerializerOptions Options = new(JsonSerializerDefaults.Web)
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };
}

/// <summary>Reads a client's options from the environment, as <c>FromEnv</c> always has.</summary>
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

/// <summary>The SDK's version, for the user agent.</summary>
internal static class SdkVersion
{
    internal static readonly string Value =
        typeof(SdkVersion).Assembly.GetName().Version?.ToString(3) ?? "0.0.0";
}

/// <summary>The user agent's vocabulary (docs/config.md section 7.6).</summary>
internal static class Platform
{
    internal static string Os()
    {
        if (OperatingSystem.IsWindows())
        {
            return "windows";
        }

        if (OperatingSystem.IsAndroid())
        {
            return "android";
        }

        if (OperatingSystem.IsIOS())
        {
            return "ios";
        }

        if (OperatingSystem.IsMacOS())
        {
            return "macos";
        }

        if (OperatingSystem.IsFreeBSD())
        {
            return "freebsd";
        }

        return OperatingSystem.IsLinux() ? "linux" : "other";
    }

    internal static string Arch() => RuntimeInformation.OSArchitecture.ToString() switch
    {
        "X64" => "x86_64",
        "Arm64" => "aarch64",
        "X86" => "x86",
        "Arm" => "arm",
        "RiscV64" => "riscv64",
        _ => "other",
    };
}
