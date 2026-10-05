using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Net.Http;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;

namespace InOrbit.Sdk;

/// <summary>One client's machinery: the resolved settings, the HTTP client, the token, the pipeline (docs/config.md).</summary>
internal sealed partial class Transport : IDisposable
{
    internal static readonly Uri DefaultBaseUrl = new("https://api.inorbit.hr");

    /// <summary>The largest answer read, 16 MiB.</summary>
    internal const int MaxBody = 16 * 1024 * 1024;

    private const int RetryBudgetCapacity = 500;

    private readonly HttpClient _http;
    private readonly bool _ownsHttp;
    private readonly ClientContext _ctx;
    private readonly Pipeline _pipeline;
    private readonly MiddlewareNext _send;
    private readonly string? _profile;
    private readonly IReadOnlyList<IHook> _hooks;

    internal Transport(ClientOptions options, Resolution res, bool explicitly)
    {
        BaseUrl = new Uri(res.Get<string>("base_url") ?? DefaultBaseUrl.OriginalString);
        _profile = res.Profile;
        if (options.HttpClient is not null && options.HttpMessageHandler is not null)
        {
            throw new ConfigException("give HttpClient or HttpMessageHandler, not both");
        }

        if ((options.HttpClient is not null || options.HttpMessageHandler is not null) && options.ClientCertificates is not null)
        {
            throw ConfigException.Of([new("client_cert", "code", "configure this on your HTTP client (HttpClient or HttpMessageHandler), or leave http_client out")]);
        }

        if (options.HttpClient is { } http)
        {
            _http = http;
        }
        else if (options.HttpMessageHandler is { } handler)
        {
            _http = new HttpClient(handler, disposeHandler: false) { Timeout = System.Threading.Timeout.InfiniteTimeSpan };
            _ownsHttp = true;
        }
        else
        {
            _http = new HttpClient(Network.Handler(res, options.ClientCertificates)) { Timeout = System.Threading.Timeout.InfiniteTimeSpan };
            _ownsHttp = true;
        }

        var ua = UserAgent(res.Get<string>("user_agent_suffix"));
        var tracing = !res.Values.TryGetValue("tracing", out var t) || (bool)t;
        var metrics = res.Values.TryGetValue("metrics", out var m) ? (bool)m : tracing;
        var log = new Log(Log.Level(res.Get<string>("log")), options.LoggerFactory, options.Redact, res.Profile, res.Get<bool>("log_headers"), res.Get<string[]>("log_allow_headers"));
        var source = res.Credential.Source;
        var callbacks = new CachedToken.Callbacks(
            error =>
            {
                if (metrics)
                {
                    var tags = new TagList { { "inorbit.credential.source", source } };
                    if (error is not null)
                    {
                        tags.Add("error.type", BuiltIns.ErrorType(error));
                    }

                    Telemetry.Exchanges.Add(1, tags);
                }
            },
            error => log.Emit(LogLevel.Warning, ("event", "token_refresh_failed"), ("reason", BuiltIns.ErrorType(error))));
        _hooks = options.Hooks ?? [];
        _ctx = new ClientContext
        {
            Host = BaseUrl.Host,
            Provider = ProviderFor(res, options, _http, ua, callbacks),
            StaticToken = !explicitly && res.Credential.Kind == "static_token",
            UserAgent = ua,
            Timeout = res.Duration("timeout"),
            TotalTimeout = res.Duration("total_timeout"),
            MaxRetries = res.Get<int>("max_retries"),
            RetryBaseDelay = res.Duration("retry_base_delay"),
            RetryMaxDelay = res.Duration("retry_max_delay"),
            RetryAfterMax = res.Duration("retry_after_max"),
            Budget = res.Values.TryGetValue("retry_budget", out var b) && !(bool)b ? null : new RetryBudget(options.RetryBudgetCapacity ?? RetryBudgetCapacity),
            RateLimit = res.Get<string>("rate_limit") switch { "wait" => RateLimitMode.Wait, "off" => RateLimitMode.Off, _ => RateLimitMode.Observe },
            Log = log,
            Hooks = _hooks,
            Tracing = tracing,
            Metrics = metrics,
        };
        _pipeline = new Pipeline(BuiltIns.For(_ctx));
        options.Pipeline?.Invoke(_pipeline);
        _send = BuiltIns.Transport(_http, BaseUrl.Host);
        Config = res.Describe(_pipeline.Names);
        _streams = res.Get<string>("streams") == "socket" ? StreamTransport.Socket : StreamTransport.Sse;
        _idle = res.Duration("stream_idle_timeout");
    }

    internal Uri BaseUrl { get; }

    internal ResolvedConfig Config { get; }

    internal RateLimit? Latest => _ctx.Latest?.Snapshot;

    internal ITokenProvider Provider => _ctx.Provider;

    internal string UserAgentValue => _ctx.UserAgent;

    internal TimeSpan Timeout => _ctx.Timeout;

    internal int MaxRetries => _ctx.MaxRetries;

    internal TimeSpan IdleTimeout => _idle;

    internal IReadOnlyList<string> PipelineNamesNow => _pipeline.Names;

    /// <summary>A client of explicit options: code and defaults only, with the checks and messages explicit construction always had.</summary>
    internal static Transport Explicit(ClientOptions options)
    {
        Urls.Check("the base URL", options.BaseUrl ?? DefaultBaseUrl, originOnly: true);
        var key = !string.IsNullOrEmpty(options.KeyId) && (!string.IsNullOrEmpty(options.KeySecret) || !string.IsNullOrEmpty(options.KeySecretFile));
        if (options.TokenProvider is null && string.IsNullOrEmpty(options.Token) && string.IsNullOrEmpty(options.TokenFile))
        {
            if (!key)
            {
                throw new ConfigException("no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET");
            }

            if (options.Scopes is null || options.Scopes.Count == 0)
            {
                throw new ConfigException("no scopes: set INORBIT_SCOPES (space-separated, such as \"identity:read account:read\")");
            }

            Urls.Check("the token URL", options.TokenUrl ?? ClientCredentials.DefaultTokenUrl, originOnly: false);
        }

        if (options.StreamIdleTimeout is { } idle && idle <= TimeSpan.Zero)
        {
            throw new ConfigException("the stream idle timeout must be positive");
        }

        return new Transport(options, Resolve(options, null, null, explicitly: true), explicitly: true);
    }

    /// <summary>Resolves <paramref name="options"/>: with the environment, the file and the chain, or, explicitly, from code and defaults only.</summary>
    internal static Resolution Resolve(ClientOptions options, string? profileType, LoadOptions? load, bool explicitly)
    {
        var code = options.Code();
        if (explicitly)
        {
            code.Remove("profile");
            code.Remove("config_file");
        }

        return Settings.Resolve(new ResolveInput
        {
            Code = code,
            HttpClient = options.HttpClient is not null || options.HttpMessageHandler is not null,
            TokenProvider = options.TokenProvider is not null,
            ProfileType = profileType,
            Explicit = explicitly,
            Load = load,
        });
    }

    /// <summary>The pipeline <paramref name="options"/> would build, by name.</summary>
    internal static IReadOnlyList<string> PipelineNames(ClientOptions options)
    {
        var placeholder = Middleware.Create("placeholder", (r, n, c) => n(r, c));
        var pipeline = new Pipeline(Settings.BuiltIns.ToDictionary(n => n, _ => placeholder, StringComparer.Ordinal));
        options.Pipeline?.Invoke(pipeline);
        return pipeline.Names;
    }

    internal static HttpClient NewHttpClient() =>
        new(new SocketsHttpHandler { AllowAutoRedirect = false, ConnectTimeout = TimeSpan.FromSeconds(10), UseProxy = false })
        {
            Timeout = System.Threading.Timeout.InfiniteTimeSpan,
        };

    /// <summary><c>inorbithr-sdk-csharp/&lt;version&gt; dotnet/&lt;version&gt; &lt;os&gt;/&lt;arch&gt;[ &lt;suffix&gt;]</c> (docs/config.md section 7.6).</summary>
    internal static string UserAgent(string? suffix)
    {
        var ua = $"inorbithr-sdk-csharp/{SdkVersion.Value} dotnet/{Environment.Version} {Platform.Os()}/{Platform.Arch()}";
        return string.IsNullOrEmpty(suffix) ? ua : $"{ua} {suffix}";
    }

    /// <summary>The provider the chain decided on (docs/config.md section 5.1).</summary>
    internal static ITokenProvider ProviderFor(Resolution res, ClientOptions options, HttpClient http, string userAgent, CachedToken.Callbacks? callbacks)
    {
        var plan = res.Credential;
        return plan.Kind switch
        {
            "custom" => options.TokenProvider!,
            "static_token" => new StaticToken(plan.Token!),
            "token_file" => new TokenFile(plan.Path!),
            "cli" => new CliToken(plan.Profile!, plan.Program, null, callbacks),
            _ => new ClientCredentials(
                new ClientCredentialsOptions
                {
                    KeyId = plan.KeyId!,
                    KeySecret = plan.KeySecret,
                    KeySecretFile = plan.KeySecretFile,
                    Scopes = res.Get<string[]>("scopes") ?? [],
                    TokenUrl = new Uri(res.Get<string>("token_url") ?? ClientCredentials.DefaultTokenUrl.OriginalString),
                    HttpClient = http,
                },
                userAgent,
                callbacks),
        };
    }

    public void Dispose()
    {
        _socket?.Dispose();
        if (_ownsHttp)
        {
            _http.Dispose();
        }
    }

    internal async Task<RawResponse> SendAsync(Operation op, CallOptions? call, CancellationToken cancellationToken) =>
        BuiltIns.RawOf(await CallAsync(op, call, stream: false, cancellationToken).ConfigureAwait(false));

    /// <summary>Whether a socket reconnect may be made: it draws from the retry budget like a retry.</summary>
    internal bool TakeReconnect() => _ctx.Budget?.Take(10) ?? true;

    /// <summary>The headers the built-ins would set on a socket upgrade, which no pipeline sees (docs/config.md section 7.12).</summary>
    internal IEnumerable<(string Name, string Value)> UpgradeHeaders()
    {
        var names = _pipeline.Names;
        if (names.Contains("request_id"))
        {
            yield return ("x-request-id", Retry.RequestId());
        }

        if (names.Contains("user_agent"))
        {
            yield return ("user-agent", _ctx.UserAgent);
        }
    }

    /// <summary>The invoker the socket upgrade goes through, so it shares the proxy, trust and mTLS settings.</summary>
    internal HttpMessageInvoker Invoker => _http;

    /// <summary>One call through the pipeline: the answer, or for a stream the opened response, whose body is the caller's to read and dispose.</summary>
    internal async Task<SdkResponse> CallAsync(Operation op, CallOptions? call, bool stream, CancellationToken cancellationToken)
    {
        var url = Url(op);
        var name = op.Name ?? op.Path;
        if (call?.IdempotencyKey is not null && !op.TakesIdempotencyKey)
        {
            throw new ConfigException($"{name} does not take an idempotency key: the API would ignore it, so repeating the call would not be safe");
        }

        var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
        {
            ["accept"] = stream ? "text/event-stream" : "application/json",
        };
        if (op.Body is not null)
        {
            headers["content-type"] = "application/json";
        }

        if (call?.Traceparent is { } tp)
        {
            headers["traceparent"] = tp;
        }

        var state = new CallState(call?.Timeout, call?.Traceparent, op.TakesIdempotencyKey) { CallerKey = call?.IdempotencyKey };
        var request = new SdkRequest
        {
            Method = op.Method.ToString().ToUpperInvariant(),
            Url = url,
            Headers = headers,
            Body = op.Body,
            Info = new CallInfo
            {
                Operation = name,
                Template = op.Template,
                Idempotent = op.RetrySafe,
                Stream = stream,
                Profile = _profile,
                Stage = PipelineStage.PerCall,
            },
            State = state,
        };
        var started = Stopwatch.GetTimestamp();
        SdkResponse resp;
        try
        {
            resp = await _pipeline.RunAsync(request, _send, cancellationToken).ConfigureAwait(false);
        }
        catch (InOrbitException e)
        {
            Failed(request, state.Attempts, e, started);
            throw;
        }

        if (resp.Status is >= 200 and < 300)
        {
            _ctx.Log.Emit(
                LogLevel.Information,
                ("event", "call"),
                ("operation", name),
                ("status", resp.Status),
                ("attempts", Math.Max(1, resp.Request.Info.Attempt)),
                ("duration_ms", (long)Stopwatch.GetElapsedTime(started).TotalMilliseconds),
                ("request_id", resp.Request.Info.RequestId),
                ("server_request_id", resp.Header("x-request-id")));
            return resp;
        }

        resp.Release();
        var error = ApiException.From(BuiltIns.RawOf(resp));
        error.RequestId = resp.Request.Info.RequestId;
        error.IdempotencyKey = resp.Request.Info.IdempotencyKey;
        Failed(resp.Request, Math.Max(1, resp.Request.Info.Attempt), error, started);
        throw error;
    }

    private void Failed(SdkRequest request, int attempts, InOrbitException error, long started)
    {
        var attempt = BuiltIns.AttemptOf(request) with
        {
            Number = Math.Max(1, attempts),
            RequestId = error.RequestId ?? request.Info.RequestId ?? string.Empty,
            IdempotencyKey = error.IdempotencyKey ?? request.Info.IdempotencyKey,
        };
        if (_pipeline.Names.Contains("hooks"))
        {
            foreach (var h in _hooks)
            {
                h.OnError(attempt, error);
            }
        }

        var code = (error as ApiException)?.Code.Value;
        int? status = (error as ApiException)?.Status;
        _ctx.Log.Emit(
            LogLevel.Information,
            ("event", "call"),
            ("operation", request.Info.Operation),
            ("error_kind", error.Kind),
            ("error_code", code),
            ("status", status),
            ("attempts", attempt.Number),
            ("duration_ms", (long)Stopwatch.GetElapsedTime(started).TotalMilliseconds),
            ("request_id", attempt.RequestId),
            ("server_request_id", (error as ApiException)?.Raw.ServerRequestId));
        _ctx.Log.Emit(
            LogLevel.Error,
            ("event", "call_failed"),
            ("operation", request.Info.Operation),
            ("error_kind", error.Kind),
            ("error_code", code),
            ("status", status),
            ("request_id", attempt.RequestId));
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
}
