using System;
using System.Collections.Generic;
using System.Diagnostics.CodeAnalysis;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Threading;
using System.Threading.Tasks;

namespace InOrbit.Sdk;

/// <summary>Where a middleware runs: once per call, or on every attempt (docs/config.md section 7.1).</summary>
public enum PipelineStage
{
    /// <summary>Once per call, outside the retry loop.</summary>
    PerCall,

    /// <summary>On every attempt, inside the retry loop.</summary>
    PerRetry,
}

/// <summary>What a middleware may read about the call (docs/config.md section 7.13).</summary>
public sealed record CallInfo
{
    /// <summary>The operation (<c>radar.list_digests</c>), or the path for a raw call.</summary>
    public required string Operation { get; init; }

    /// <summary>The path template (<c>/v1/radar/digests/{digest_id}</c>), when the surface knows it.</summary>
    public string? Template { get; internal init; }

    /// <summary>Whether <c>retry</c> may repeat the call.</summary>
    public bool Idempotent { get; internal init; }

    /// <summary>The <c>Idempotency-Key</c> the call is sent with, once <c>idempotency_key</c> set it.</summary>
    public string? IdempotencyKey { get; internal init; }

    /// <summary>The call's <c>x-request-id</c>, once <c>request_id</c> set it.</summary>
    public string? RequestId { get; internal init; }

    /// <summary>The attempt, 1-based; 0 in the per-call stage.</summary>
    public int Attempt { get; internal init; }

    /// <summary>When the call must end, once <c>deadline</c> set it.</summary>
    public DateTimeOffset? Deadline { get; internal init; }

    /// <summary>Whether the answer is a stream, whose body a middleware must not read.</summary>
    public bool Stream { get; internal init; }

    /// <summary>The configuration profile the client was loaded for.</summary>
    public string? Profile { get; internal init; }

    /// <summary>The stage the middleware runs in.</summary>
    public PipelineStage Stage { get; internal init; }
}

/// <summary>A request on its way through the pipeline. Each attempt gets its own copy of the headers.</summary>
public sealed record SdkRequest
{
    /// <summary>The HTTP method (<c>GET</c>).</summary>
    public required string Method { get; init; }

    /// <summary>The full URL, query included.</summary>
    public required Uri Url { get; init; }

    /// <summary>The headers, by name without case; a middleware may change them.</summary>
    public required IDictionary<string, string> Headers { get; init; }

    /// <summary>The body as bytes; <see langword="null"/> when there is none. Never a stream.</summary>
    public ReadOnlyMemory<byte>? Body { get; init; }

    /// <summary>What the call is.</summary>
    public required CallInfo Info { get; init; }

    /// <summary>What one call carries between the built-ins.</summary>
    internal CallState State { get; init; } = new(null, null, false);
}

/// <summary>An answer on its way back.</summary>
public sealed record SdkResponse
{
    /// <summary>The HTTP status.</summary>
    public required int Status { get; init; }

    /// <summary>The response headers, by lower-case name.</summary>
    public required IReadOnlyDictionary<string, IReadOnlyList<string>> Headers { get; init; }

    /// <summary>The body, read whole; empty for a stream.</summary>
    public ReadOnlyMemory<byte> Body { get; init; }

    /// <summary>A stream's body, which a middleware must not read; <see langword="null"/> otherwise.</summary>
    public Stream? Stream { get; init; }

    /// <summary>The request as it was sent, after every middleware.</summary>
    public required SdkRequest Request { get; init; }

    /// <summary>The rate-limit snapshot, once <c>rate_limit</c> read it.</summary>
    public RateLimit? RateLimit { get; init; }

    /// <summary>The first value of a header, or <see langword="null"/>.</summary>
    /// <param name="name">The header's name, any case.</param>
    /// <returns>The value.</returns>
    public string? Header(string name)
    {
        ArgumentNullException.ThrowIfNull(name);
        return Headers.TryGetValue(name.ToLowerInvariant(), out var v) && v.Count > 0 ? v[0] : null;
    }

    /// <summary>Releases a stream's connection; nothing for a whole body.</summary>
    internal void Release()
    {
        Stream?.Dispose();
    }
}

/// <summary>The rest of the pipeline after a middleware.</summary>
/// <param name="request">The request to send on.</param>
/// <param name="cancellationToken">Cancels it.</param>
/// <returns>The answer.</returns>
public delegate ValueTask<SdkResponse> MiddlewareNext(SdkRequest request, CancellationToken cancellationToken);

/// <summary>
/// One named step every call passes through (docs/config.md section 7). It may change the request,
/// call <c>next</c> zero times (answering itself), once, or more (each call of <c>next</c> from the
/// per-call stage is a fresh attempt), and inspect what comes back. It must not log secrets or bodies.
/// </summary>
/// <example>
/// <code>
/// sealed class Team : Middleware
/// {
///     public override string Name =&gt; "team";
///     public override ValueTask&lt;SdkResponse&gt; SendAsync(SdkRequest request, MiddlewareNext next, CancellationToken cancellationToken)
///     {
///         request.Headers["x-team"] = "payments";
///         return next(request, cancellationToken);
///     }
/// }
/// using var client = Client.Load(new ClientOptions { Pipeline = p =&gt; p.AddPerRetry(new Team()) });
/// </code>
/// </example>
public abstract class Middleware
{
    /// <summary>Its name, unique in the pipeline.</summary>
    public abstract string Name { get; }

    /// <summary>Handles <paramref name="request"/>, calling <paramref name="next"/> for the rest of the pipeline.</summary>
    /// <param name="request">The request.</param>
    /// <param name="next">The rest of the pipeline.</param>
    /// <param name="cancellationToken">Cancels the call: the caller's token, the deadline and, per attempt, the timeout.</param>
    /// <returns>The answer.</returns>
    [SuppressMessage("Naming", "CA1716", Justification = "next is the contract's name (docs/config.md section 8), as in ASP.NET Core middleware.")]
    public abstract ValueTask<SdkResponse> SendAsync(SdkRequest request, MiddlewareNext next, CancellationToken cancellationToken);

    /// <summary>A middleware from a function.</summary>
    /// <param name="name">Its name.</param>
    /// <param name="handle">What it does.</param>
    /// <returns>The middleware.</returns>
    public static Middleware Create(string name, Func<SdkRequest, MiddlewareNext, CancellationToken, ValueTask<SdkResponse>> handle)
    {
        ArgumentException.ThrowIfNullOrEmpty(name);
        ArgumentNullException.ThrowIfNull(handle);
        return new FunctionMiddleware(name, handle);
    }

    /// <summary>
    /// A <see cref="DelegatingHandler"/> (Polly, <c>Microsoft.Extensions.Http.Resilience</c>, your own) as a
    /// middleware, at either slot. The handler sees an <see cref="HttpRequestMessage"/>; what it sends on
    /// reaches the rest of the pipeline. Its <c>InnerHandler</c> must be unset; the SDK sets it.
    /// </summary>
    /// <param name="name">Its name in the pipeline.</param>
    /// <param name="handler">The handler.</param>
    /// <returns>The middleware.</returns>
    public static Middleware FromHandler(string name, DelegatingHandler handler)
    {
        ArgumentException.ThrowIfNullOrEmpty(name);
        ArgumentNullException.ThrowIfNull(handler);
        return new HandlerMiddleware(name, handler);
    }

    private sealed class FunctionMiddleware(string name, Func<SdkRequest, MiddlewareNext, CancellationToken, ValueTask<SdkResponse>> handle) : Middleware
    {
        public override string Name => name;

        public override ValueTask<SdkResponse> SendAsync(SdkRequest request, MiddlewareNext next, CancellationToken cancellationToken) =>
            handle(request, next, cancellationToken);
    }
}

/// <summary>A <see cref="DelegatingHandler"/> run as a middleware.</summary>
internal sealed class HandlerMiddleware : Middleware
{
    private static readonly HttpRequestOptionsKey<Exchange> Key = new("InOrbit.Sdk.exchange");
    private readonly string _name;
    private readonly DelegatingHandler _handler;

    internal HandlerMiddleware(string name, DelegatingHandler handler)
    {
        if (handler.InnerHandler is not null)
        {
            throw new ConfigException($"the handler of middleware {name} already has an InnerHandler; the SDK sets it");
        }

        _name = name;
        handler.InnerHandler = new Terminal();
        _handler = handler;
    }

    public override string Name => _name;

    public override async ValueTask<SdkResponse> SendAsync(SdkRequest request, MiddlewareNext next, CancellationToken cancellationToken)
    {
        using var message = new HttpRequestMessage(new HttpMethod(request.Method), request.Url);
        if (request.Body is { } body)
        {
            message.Content = new ReadOnlyMemoryContent(body);
        }

        foreach (var (name, value) in request.Headers)
        {
            if (!message.Headers.TryAddWithoutValidation(name, value))
            {
                message.Content?.Headers.TryAddWithoutValidation(name, value);
            }
        }

        var exchange = new Exchange(request, next);
        message.Options.Set(Key, exchange);
        using var invoker = new HttpMessageInvoker(_handler, disposeHandler: false);
        using var answer = await invoker.SendAsync(message, cancellationToken).ConfigureAwait(false);
        if (exchange.Answers.TryGetValue(answer, out var original))
        {
            return original;
        }

        // The handler answered itself.
        var bytes = await answer.Content.ReadAsByteArrayAsync(cancellationToken).ConfigureAwait(false);
        return new SdkResponse { Status = (int)answer.StatusCode, Headers = Wire.Headers(answer), Body = bytes, Request = request };
    }

    private sealed class Exchange(SdkRequest request, MiddlewareNext next)
    {
        internal SdkRequest Request { get; } = request;

        internal MiddlewareNext Next { get; } = next;

        internal Dictionary<HttpResponseMessage, SdkResponse> Answers { get; } = new(ReferenceEqualityComparer.Instance);
    }

    /// <summary>The handler's inner end: what it sends goes on through the pipeline.</summary>
    private sealed class Terminal : HttpMessageHandler
    {
        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage message, CancellationToken cancellationToken)
        {
            if (!message.Options.TryGetValue(Key, out var exchange))
            {
                throw new InvalidOperationException("this handler belongs to an InOrbit.Sdk pipeline");
            }

            var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            foreach (var (name, values) in message.Headers.NonValidated)
            {
                headers[name] = string.Join(", ", values);
            }

            ReadOnlyMemory<byte>? body = exchange.Request.Body;
            if (message.Content is not null)
            {
                foreach (var (name, values) in message.Content.Headers.NonValidated)
                {
                    headers[name] = string.Join(", ", values);
                }

                body = await message.Content.ReadAsByteArrayAsync(cancellationToken).ConfigureAwait(false);
            }

            var request = exchange.Request with
            {
                Method = message.Method.Method,
                Url = message.RequestUri ?? exchange.Request.Url,
                Headers = headers,
                Body = body,
            };
            var answer = await exchange.Next(request, cancellationToken).ConfigureAwait(false);
            var response = new HttpResponseMessage((System.Net.HttpStatusCode)answer.Status)
            {
                RequestMessage = message,
                Content = answer.Stream is { } s ? new StreamContent(s) : new ReadOnlyMemoryContent(answer.Body),
            };
            foreach (var (name, values) in answer.Headers)
            {
                if (!response.Headers.TryAddWithoutValidation(name, values))
                {
                    response.Content.Headers.TryAddWithoutValidation(name, values);
                }
            }

            exchange.Answers[response] = answer;
            return response;
        }
    }
}

/// <summary>
/// The client's pipeline, edited by name at construction: the built-ins of docs/config.md section 7.2,
/// outermost first, and what you add. Settings switch built-ins off without removing them
/// (<c>RateLimit = RateLimitMode.Off</c>, <c>Tracing = false</c>, <c>Log = LogLevel.None</c>).
/// </summary>
public sealed class Pipeline
{
    private static readonly string[] Kept = ["retry", "auth", "timeout"];
    private readonly List<(string Name, Middleware Middleware)> _slots;

    internal Pipeline(IReadOnlyDictionary<string, Middleware> builtIns)
    {
        _slots = Settings.BuiltIns.Select(n => (n, builtIns[n])).ToList();
    }

    /// <summary>The names, outermost first.</summary>
    public IReadOnlyList<string> Names => _slots.Select(s => s.Name).ToArray();

    /// <summary>Adds <paramref name="middleware"/> once per call, just before <c>retry</c>, after earlier additions.</summary>
    /// <param name="middleware">The middleware.</param>
    /// <returns>This pipeline.</returns>
    /// <exception cref="ConfigException">Its name is taken.</exception>
    public Pipeline AddPerCall(Middleware middleware) => Insert(Index("retry"), middleware);

    /// <summary>Adds <paramref name="middleware"/> on every attempt, just before <c>timeout</c>, after earlier additions.</summary>
    /// <param name="middleware">The middleware.</param>
    /// <returns>This pipeline.</returns>
    /// <exception cref="ConfigException">Its name is taken.</exception>
    public Pipeline AddPerRetry(Middleware middleware) => Insert(Index("timeout"), middleware);

    /// <summary>Adds <paramref name="middleware"/> just before the one named <paramref name="name"/>.</summary>
    /// <param name="name">A middleware's name.</param>
    /// <param name="middleware">The middleware.</param>
    /// <returns>This pipeline.</returns>
    /// <exception cref="ConfigException">No middleware has that name, or the new one's name is taken.</exception>
    public Pipeline InsertBefore(string name, Middleware middleware) => Insert(Index(name), middleware);

    /// <summary>Adds <paramref name="middleware"/> just after the one named <paramref name="name"/>.</summary>
    /// <param name="name">A middleware's name.</param>
    /// <param name="middleware">The middleware.</param>
    /// <returns>This pipeline.</returns>
    /// <exception cref="ConfigException">No middleware has that name, or the new one's name is taken.</exception>
    public Pipeline InsertAfter(string name, Middleware middleware) => Insert(Index(name) + 1, middleware);

    /// <summary>Swaps the middleware named <paramref name="name"/> for <paramref name="middleware"/>, which takes its name and place.</summary>
    /// <param name="name">A middleware's name.</param>
    /// <param name="middleware">The replacement.</param>
    /// <returns>This pipeline.</returns>
    /// <exception cref="ConfigException">No middleware has that name.</exception>
    public Pipeline Replace(string name, Middleware middleware)
    {
        ArgumentNullException.ThrowIfNull(middleware);
        _slots[Index(name)] = (name, middleware);
        return this;
    }

    /// <summary>Drops the middleware named <paramref name="name"/>; <c>retry</c>, <c>auth</c> and <c>timeout</c> can be replaced only.</summary>
    /// <param name="name">A middleware's name.</param>
    /// <returns>This pipeline.</returns>
    /// <exception cref="ConfigException">No middleware has that name, or it is one that cannot be removed.</exception>
    public Pipeline Remove(string name)
    {
        if (Array.IndexOf(Kept, name) >= 0)
        {
            var why = name switch
            {
                "retry" => "set MaxRetries = 0 to retry nothing",
                "auth" => "without it no credential is sent",
                _ => "without it an attempt could wait forever",
            };
            throw new ConfigException($"{name} cannot be removed, only replaced: {why}");
        }

        _slots.RemoveAt(Index(name));
        return this;
    }

    /// <summary>Runs <paramref name="request"/> through every middleware, then <paramref name="transport"/>.</summary>
    internal ValueTask<SdkResponse> RunAsync(SdkRequest request, MiddlewareNext transport, CancellationToken cancellationToken)
    {
        var slots = _slots.ToArray();
        var retryAt = Array.FindIndex(slots, s => s.Name == "retry");
        ValueTask<SdkResponse> Step(int i, SdkRequest req, CancellationToken ct)
        {
            if (i >= slots.Length)
            {
                return transport(req, ct);
            }

            var stage = retryAt >= 0 && i > retryAt ? PipelineStage.PerRetry : PipelineStage.PerCall;
            var seen = req.Info.Stage == stage ? req : req with { Info = req.Info with { Stage = stage } };
            return slots[i].Middleware.SendAsync(seen, (r, c) => Step(i + 1, r, c), ct);
        }

        return Step(0, request, cancellationToken);
    }

    private int Index(string name)
    {
        var i = _slots.FindIndex(s => s.Name == name);
        if (i < 0)
        {
            throw new ConfigException($"the pipeline has no middleware named \"{name}\"; it has {string.Join(", ", Names)}");
        }

        return i;
    }

    private Pipeline Insert(int at, Middleware middleware)
    {
        ArgumentNullException.ThrowIfNull(middleware);
        if (string.IsNullOrEmpty(middleware.Name))
        {
            throw new ConfigException("a middleware needs a name");
        }

        if (_slots.Any(s => s.Name == middleware.Name))
        {
            throw new ConfigException($"the pipeline already has a middleware named \"{middleware.Name}\"");
        }

        _slots.Insert(at, (middleware.Name, middleware));
        return this;
    }
}

/// <summary>Conversions between the wire and the pipeline's shapes.</summary>
internal static class Wire
{
    internal static Dictionary<string, IReadOnlyList<string>> Headers(HttpResponseMessage response)
    {
        var all = new Dictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal);
        foreach (var (name, values) in response.Headers.NonValidated.Concat(response.Content.Headers.NonValidated))
        {
            all[name.ToLowerInvariant()] = values.ToArray();
        }

        return all;
    }

    internal static void Apply(HttpRequestMessage message, IDictionary<string, string> headers)
    {
        foreach (var (name, value) in headers)
        {
            if (name.Equals("content-type", StringComparison.OrdinalIgnoreCase) || name.StartsWith("content-", StringComparison.OrdinalIgnoreCase))
            {
                if (message.Content is not null)
                {
                    message.Content.Headers.Remove(name);
                    message.Content.Headers.TryAddWithoutValidation(name, value);
                }

                continue;
            }

            message.Headers.Remove(name);
            message.Headers.TryAddWithoutValidation(name, value);
        }
    }

    internal static MediaTypeHeaderValue Json { get; } = new("application/json");
}
