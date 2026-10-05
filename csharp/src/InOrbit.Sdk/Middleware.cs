using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;

namespace InOrbit.Sdk;

/// <summary>What one call carries between the built-ins, out of the user's sight.</summary>
/// <param name="timeout">The per-call timeout.</param>
/// <param name="traceparent">The per-call <c>traceparent</c>.</param>
/// <param name="takesKey">Whether the operation takes an <c>Idempotency-Key</c>.</param>
internal sealed class CallState(TimeSpan? timeout, string? traceparent, bool takesKey)
{
    internal TimeSpan? Timeout { get; } = timeout;

    internal string? Traceparent { get; } = traceparent;

    internal bool TakesKey { get; } = takesKey;

    /// <summary>The caller's own idempotency key.</summary>
    internal string? CallerKey { get; init; }

    /// <summary>Whether <c>auth</c> already fetched a fresh token after a 401.</summary>
    internal bool Refreshed { get; set; }

    /// <summary>The attempts made so far.</summary>
    internal int Attempts { get; set; }

    /// <summary>The call span, once <c>call_tracing</c> started it.</summary>
    internal Activity? CallActivity { get; set; }

    /// <summary>The attempt span's ids, for log records.</summary>
    internal (string TraceId, string SpanId)? Span { get; set; }
}

/// <summary>A token bucket for retries, after the AWS standard retry mode (docs/config.md section 7.4).</summary>
/// <param name="capacity">The bucket's size, full at the start.</param>
internal sealed class RetryBudget(int capacity)
{
    private readonly object _gate = new();
    private readonly int _capacity = capacity;
    private int _tokens = capacity;

    internal int Tokens
    {
        get
        {
            lock (_gate)
            {
                return _tokens;
            }
        }
    }

    /// <summary>Pays <paramref name="cost"/> when the bucket can.</summary>
    internal bool Take(int cost)
    {
        lock (_gate)
        {
            if (_tokens < cost)
            {
                return false;
            }

            _tokens -= cost;
            return true;
        }
    }

    /// <summary>Puts <paramref name="amount"/> back, up to the capacity.</summary>
    internal void Give(int amount)
    {
        lock (_gate)
        {
            _tokens = Math.Min(_capacity, _tokens + amount);
        }
    }
}

/// <summary>One client's settings and state, shared by its built-ins.</summary>
internal sealed class ClientContext
{
    internal required string Host { get; init; }

    internal required ITokenProvider Provider { get; init; }

    /// <summary>A refused static token from <c>load</c> ends the call with an <see cref="AuthException"/>.</summary>
    internal bool StaticToken { get; init; }

    internal required string UserAgent { get; init; }

    internal TimeSpan Timeout { get; init; }

    internal TimeSpan TotalTimeout { get; init; }

    internal int MaxRetries { get; init; }

    internal TimeSpan RetryBaseDelay { get; init; }

    internal TimeSpan RetryMaxDelay { get; init; }

    internal TimeSpan RetryAfterMax { get; init; }

    internal RetryBudget? Budget { get; init; }

    internal RateLimitMode RateLimit { get; init; }

    internal required Log Log { get; init; }

    internal required IReadOnlyList<IHook> Hooks { get; init; }

    internal bool Tracing { get; init; }

    internal bool Metrics { get; init; }

    /// <summary>The latest rate-limit snapshot, and when its window resets.</summary>
    internal (RateLimit Snapshot, DateTimeOffset? ResetAt)? Latest { get; set; }
}

/// <summary>The built-in middlewares of docs/config.md section 7.2 and the transport after them.</summary>
internal static class BuiltIns
{
    /// <summary>Every built-in of one client, by name.</summary>
    internal static Dictionary<string, Middleware> For(ClientContext ctx) => new(StringComparer.Ordinal)
    {
        ["request_id"] = Middleware.Create("request_id", async (req, next, ct) =>
        {
            var id = Retry.RequestId();
            req.Headers["x-request-id"] = id;
            try
            {
                return await next(req with { Info = req.Info with { RequestId = id } }, ct).ConfigureAwait(false);
            }
            catch (InOrbitException e)
            {
                e.RequestId ??= id;
                throw;
            }
        }),
        ["user_agent"] = Middleware.Create("user_agent", (req, next, ct) =>
        {
            req.Headers["user-agent"] = ctx.UserAgent;
            return next(req, ct);
        }),
        ["idempotency_key"] = Middleware.Create("idempotency_key", async (req, next, ct) =>
        {
            if (!req.State.TakesKey)
            {
                return await next(req, ct).ConfigureAwait(false);
            }

            var key = req.State.CallerKey ?? Guid.NewGuid().ToString();
            req.Headers["idempotency-key"] = key;
            try
            {
                return await next(req with { Info = req.Info with { IdempotencyKey = key, Idempotent = true } }, ct).ConfigureAwait(false);
            }
            catch (InOrbitException e)
            {
                e.IdempotencyKey ??= key;
                throw;
            }
        }),
        ["call_tracing"] = Middleware.Create("call_tracing", (req, next, ct) => CallTracingAsync(ctx, req, next, ct)),
        ["deadline"] = Middleware.Create("deadline", async (req, next, ct) =>
        {
            var total = req.State.Timeout is { } t && t < ctx.TotalTimeout ? t : ctx.TotalTimeout;
            using var timer = CancellationTokenSource.CreateLinkedTokenSource(ct);
            timer.CancelAfter(total);
            try
            {
                return await next(req with { Info = req.Info with { Deadline = DateTimeOffset.UtcNow + total } }, timer.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!ct.IsCancellationRequested && timer.IsCancellationRequested)
            {
                throw new RequestTimeoutException(ctx.Host, total) { RequestId = req.Info.RequestId, IdempotencyKey = req.Info.IdempotencyKey };
            }
        }),
        ["retry"] = Middleware.Create("retry", (req, next, ct) => RetryAsync(ctx, req, next, ct)),
        ["auth"] = Middleware.Create("auth", async (req, next, ct) =>
        {
            var resp = await AuthorizedAsync(ctx, req, next, ct).ConfigureAwait(false);
            var st = req.State;
            if (resp.Status != 401 || st.Refreshed)
            {
                return resp;
            }

            st.Refreshed = true;
            resp.Release();
            if (ctx.StaticToken)
            {
                throw new AuthException(
                    "the API refused the token (HTTP 401): it has expired or was revoked; create a new one in the console or with `iohr token create`, and set it again",
                    "unauthenticated")
                { RequestId = req.Info.RequestId, IdempotencyKey = req.Info.IdempotencyKey };
            }

            // One fresh token, inside this middleware: not a retry, and nothing from the budget.
            await ctx.Provider.InvalidateAsync().ConfigureAwait(false);
            return await AuthorizedAsync(ctx, req with { Headers = new Dictionary<string, string>(req.Headers, StringComparer.OrdinalIgnoreCase) }, next, ct).ConfigureAwait(false);
        }),
        ["rate_limit"] = Middleware.Create("rate_limit", (req, next, ct) => RateLimitAsync(ctx, req, next, ct)),
        ["attempt_tracing"] = Middleware.Create("attempt_tracing", (req, next, ct) => AttemptTracingAsync(ctx, req, next, ct)),
        ["logging"] = Middleware.Create("logging", (req, next, ct) => LoggingAsync(ctx, req, next, ct)),
        ["hooks"] = Middleware.Create("hooks", async (req, next, ct) =>
        {
            if (ctx.Hooks.Count == 0)
            {
                return await next(req, ct).ConfigureAwait(false);
            }

            var attempt = AttemptOf(req);
            foreach (var h in ctx.Hooks)
            {
                h.OnRequest(attempt);
            }

            var resp = await next(req, ct).ConfigureAwait(false);
            var raw = RawOf(resp);
            foreach (var h in ctx.Hooks)
            {
                h.OnResponse(attempt, raw);
            }

            return resp;
        }),
        ["timeout"] = Middleware.Create("timeout", async (req, next, ct) =>
        {
            var t = req.State.Timeout ?? ctx.Timeout;
            var limit = t;
            if (req.Info.Deadline is { } deadline)
            {
                var left = deadline - DateTimeOffset.UtcNow;
                limit = left < TimeSpan.Zero ? TimeSpan.Zero : left < t ? left : t;
            }

            using var timer = CancellationTokenSource.CreateLinkedTokenSource(ct);
            timer.CancelAfter(limit);
            try
            {
                return await next(req, timer.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!ct.IsCancellationRequested && timer.IsCancellationRequested)
            {
                throw new RequestTimeoutException(ctx.Host, t);
            }
        }),
    };

    /// <summary>An attempt as hooks see it.</summary>
    internal static Attempt AttemptOf(SdkRequest request) =>
        new(request.Info.Operation, request.Method, request.Url.AbsolutePath, Math.Max(1, request.Info.Attempt), request.Info.RequestId ?? string.Empty)
        {
            IdempotencyKey = request.Info.IdempotencyKey,
            Stage = request.Info.Stage,
        };

    /// <summary>The raw answer a response stands for.</summary>
    internal static RawResponse RawOf(SdkResponse response) =>
        new(response.Status, response.Headers, response.Body.ToArray(), response.Request.Info.RequestId ?? string.Empty, Math.Max(1, response.Request.Info.Attempt), response.Request.Info.IdempotencyKey, response.RateLimit);

    /// <summary>What an error is called in a span or a metric: the API's code, or the kind.</summary>
    internal static string ErrorType(Exception e) => e switch
    {
        ApiException api => api.Code.Value,
        InOrbitException sdk => sdk.Kind,
        OperationCanceledException => "cancelled",
        _ => e.GetType().Name,
    };

    /// <summary>The innermost step: sends the request and reads the answer (16 MiB at most), or hands over a stream's body unread.</summary>
    internal static MiddlewareNext Transport(HttpClient http, string host) => async (req, ct) =>
    {
        var message = new HttpRequestMessage(new HttpMethod(req.Method), req.Url);
        if (req.Body is { } body)
        {
            message.Content = new ReadOnlyMemoryContent(body);
        }

        Wire.Apply(message, req.Headers);
        HttpResponseMessage response;
        try
        {
            response = await http.SendAsync(message, HttpCompletionOption.ResponseHeadersRead, ct).ConfigureAwait(false);
        }
        catch (HttpRequestException e)
        {
            message.Dispose();
            throw new ConnectionException(host, ClientCredentials.Describe(e), e);
        }
        catch
        {
            message.Dispose();
            throw;
        }

        var headers = Wire.Headers(response);
        var status = (int)response.StatusCode;
        if (req.Info.Stream && status is >= 200 and < 300)
        {
            // The body is the stream: handed over unread; whoever reads it disposes it.
            var stream = await response.Content.ReadAsStreamAsync(ct).ConfigureAwait(false);
            return new SdkResponse { Status = status, Headers = headers, Stream = new BodyStream(stream, response, null), Request = req };
        }

        using (message)
        using (response)
        {
            if (response.Content.Headers.ContentLength > global::InOrbit.Sdk.Transport.MaxBody)
            {
                throw new TooLargeException();
            }

            byte[] bytes;
            try
            {
                bytes = await ReadCappedAsync(response.Content, ct).ConfigureAwait(false);
            }
            catch (Exception e) when (e is HttpRequestException or IOException)
            {
                throw new ConnectionException(host, e.Message, e);
            }

            return new SdkResponse { Status = status, Headers = headers, Body = bytes, Request = req };
        }
    };

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
                if (buffer.Length + read > global::InOrbit.Sdk.Transport.MaxBody)
                {
                    throw new TooLargeException();
                }

                buffer.Write(chunk, 0, read);
            }

            return buffer.ToArray();
        }
    }

    private static async ValueTask<SdkResponse> AuthorizedAsync(ClientContext ctx, SdkRequest req, MiddlewareNext next, CancellationToken ct)
    {
        Token token;
        try
        {
            token = await ctx.Provider.GetTokenAsync(ct).ConfigureAwait(false);
        }
        catch (InOrbitException)
        {
            throw;
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            throw new AuthException("the token provider failed: " + e.Message, string.Empty, e);
        }

        req.Headers["authorization"] = "Bearer " + token.Access;
        return await next(req, ct).ConfigureAwait(false);
    }

    /// <summary>The wait a retry-worthy answer asks for: <c>Retry-After</c>, else the envelope's <c>retry</c> detail.</summary>
    private static TimeSpan? AskedWait(SdkResponse response)
    {
        if (Retry.Asked(response.Header("retry-after")) is { } header)
        {
            return header;
        }

        try
        {
            using var doc = JsonDocument.Parse(response.Body);
            if (doc.RootElement.ValueKind == JsonValueKind.Object && doc.RootElement.TryGetProperty("details", out var details) && details.ValueKind == JsonValueKind.Array)
            {
                foreach (var d in details.EnumerateArray())
                {
                    if (Detail.Read(d) is RetryDetail r && r.AfterSeconds >= 0)
                    {
                        return TimeSpan.FromSeconds(r.AfterSeconds);
                    }
                }
            }
        }
        catch (JsonException)
        {
            // Not JSON: nothing asked.
        }

        return null;
    }

    private static async ValueTask<SdkResponse> RetryAsync(ClientContext ctx, SdkRequest req, MiddlewareNext next, CancellationToken ct)
    {
        var st = req.State;
        var retries = 0;
        var lastCost = 0;
        for (var attempt = 1; ; attempt++)
        {
            st.Attempts = attempt;
            var areq = req with
            {
                Headers = new Dictionary<string, string>(req.Headers, StringComparer.OrdinalIgnoreCase),
                Info = req.Info with { Attempt = attempt, Stage = PipelineStage.PerRetry },
            };
            SdkResponse? resp = null;
            InOrbitException? error = null;
            try
            {
                resp = await next(areq, ct).ConfigureAwait(false);
            }
            catch (InOrbitException e) when (e is ConnectionException or RequestTimeoutException)
            {
                error = e;
            }

            string reason;
            TimeSpan wait;
            var cost = 10;
            if (resp is not null)
            {
                if (!Retry.RetryableStatus(resp.Status) || !req.Info.Idempotent)
                {
                    if (resp.Status is >= 200 and < 300)
                    {
                        ctx.Budget?.Give(retries == 0 ? 1 : lastCost);
                    }

                    return resp;
                }

                reason = resp.Status.ToString(System.Globalization.CultureInfo.InvariantCulture);
                var asked = AskedWait(resp);
                if (asked > ctx.RetryAfterMax)
                {
                    return resp;
                }

                if (resp.Status == 429 || (resp.Status == 503 && asked is not null))
                {
                    cost = 5;
                }

                wait = asked ?? Retry.Backoff(retries, ctx.RetryBaseDelay, ctx.RetryMaxDelay);
            }
            else
            {
                if (!req.Info.Idempotent || ct.IsCancellationRequested)
                {
                    throw error!;
                }

                reason = error!.Kind;
                wait = Retry.Backoff(retries, ctx.RetryBaseDelay, ctx.RetryMaxDelay);
            }

            var stop = retries >= ctx.MaxRetries
                || (req.Info.Deadline is { } deadline && DateTimeOffset.UtcNow + wait > deadline)
                || (ctx.Budget is not null && !ctx.Budget.Take(cost));
            if (stop)
            {
                return resp ?? throw error!;
            }

            lastCost = cost;
            var hooked = AttemptOf(areq);
            foreach (var h in ctx.Hooks)
            {
                h.OnRetry(hooked, reason, wait);
            }

            ctx.Log.Emit(LogLevel.Warning, ("event", "retry"), ("operation", req.Info.Operation), ("attempt", attempt), ("reason", reason), ("delay_ms", (long)wait.TotalMilliseconds), ("request_id", req.Info.RequestId));
            if (ctx.Metrics)
            {
                Telemetry.Retries.Add(1, new("inorbit.operation", req.Info.Operation), new("inorbit.retry.reason", reason));
            }

            resp?.Release();
            await Retry.SleepAsync(wait, ct).ConfigureAwait(false);
            retries++;
        }
    }

    private static async ValueTask<SdkResponse> RateLimitAsync(ClientContext ctx, SdkRequest req, MiddlewareNext next, CancellationToken ct)
    {
        if (ctx.RateLimit == RateLimitMode.Off)
        {
            return await next(req, ct).ConfigureAwait(false);
        }

        if (ctx.RateLimit == RateLimitMode.Wait && ctx.Latest is { } latest && latest.Snapshot.Remaining == 0 && latest.ResetAt is { } resetAt)
        {
            var now = DateTimeOffset.UtcNow;
            if (resetAt > now)
            {
#pragma warning disable CA5394 // Jitter only spreads the wait; not a secret.
                var wait = resetAt - now + TimeSpan.FromMilliseconds(Random.Shared.Next(101));
#pragma warning restore CA5394
                if (req.Info.Deadline is { } deadline && now + wait > deadline)
                {
                    throw new RequestTimeoutException(ctx.Host, ctx.TotalTimeout, "the call would pass its total timeout waiting for the rate-limit window to reset, of");
                }

                ctx.Log.Emit(LogLevel.Warning, ("event", "rate_limit_wait"), ("attempt", Math.Max(1, req.Info.Attempt)), ("reason", "rate_limit"), ("delay_ms", (long)wait.TotalMilliseconds), ("request_id", req.Info.RequestId));
                await Retry.SleepAsync(wait, ct).ConfigureAwait(false);
            }
        }

        var resp = await next(req, ct).ConfigureAwait(false);
        if (RateLimit.Read(resp.Headers) is not { } snapshot)
        {
            return resp;
        }

        ctx.Latest = (snapshot, snapshot.Reset is { } r ? DateTimeOffset.UtcNow + r : null);
        return resp with { RateLimit = snapshot };
    }

    private static async ValueTask<SdkResponse> CallTracingAsync(ClientContext ctx, SdkRequest req, MiddlewareNext next, CancellationToken ct)
    {
        if (!ctx.Tracing && !ctx.Metrics)
        {
            return await next(req, ct).ConfigureAwait(false);
        }

        var started = Stopwatch.GetTimestamp();
        Activity? span = null;
        if (ctx.Tracing && Telemetry.Source.HasListeners())
        {
            var parented = req.State.Traceparent is { } tp && ActivityContext.TryParse(tp, null, out var parent);
            span = parented
                ? Telemetry.Source.StartActivity(req.Info.Operation, ActivityKind.Internal, parent)
                : Telemetry.Source.StartActivity(req.Info.Operation, ActivityKind.Internal);
            span?.SetTag("inorbit.operation", req.Info.Operation);
            if (req.Info.RequestId is { } id)
            {
                span?.SetTag("inorbit.request_id", id);
            }

            req.State.CallActivity = span;
        }

        void Record(string? error)
        {
            if (ctx.Metrics)
            {
                var tags = new TagList { { "inorbit.operation", req.Info.Operation } };
                if (error is not null)
                {
                    tags.Add("error.type", error);
                }

                Telemetry.CallDuration.Record(Stopwatch.GetElapsedTime(started).TotalSeconds, tags);
            }
        }

        void Fail(string type)
        {
            span?.SetTag("error.type", type);
            span?.SetStatus(ActivityStatusCode.Error);
        }

        try
        {
            var resp = await next(req, ct).ConfigureAwait(false);
            var failed = resp.Status >= 400 ? resp.Status.ToString(System.Globalization.CultureInfo.InvariantCulture) : null;
            if (failed is not null)
            {
                Fail(failed);
            }

            if (resp.Stream is { } stream)
            {
                return resp with
                {
                    Stream = new BodyStream(stream, null, () =>
                    {
                        Record(null);
                        span?.Dispose();
                    }),
                };
            }

            Record(failed);
            span?.Dispose();
            return resp;
        }
        catch (Exception e) when (e is InOrbitException or OperationCanceledException)
        {
            Fail(ErrorType(e));
            Record(ErrorType(e));
            span?.Dispose();
            throw;
        }
    }

    private static async ValueTask<SdkResponse> AttemptTracingAsync(ClientContext ctx, SdkRequest req, MiddlewareNext next, CancellationToken ct)
    {
        if (!ctx.Tracing && !ctx.Metrics)
        {
            return await next(req, ct).ConfigureAwait(false);
        }

        var started = Stopwatch.GetTimestamp();
        var host = req.Url.Host.Trim('[', ']');
        Activity? span = null;
        if (ctx.Tracing && Telemetry.Source.HasListeners())
        {
            var template = req.Info.Template;
            var name = template is null ? req.Method : $"{req.Method} {template}";
            var parent = req.State.CallActivity?.Context ?? default;
            span = Telemetry.Source.StartActivity(name, ActivityKind.Client, parent);
            if (span is not null)
            {
                span.SetTag("http.request.method", req.Method);
                span.SetTag("server.address", host);
                span.SetTag("server.port", req.Url.Port);
                span.SetTag("url.full", Telemetry.RedactedUrl(req.Url));
                if (template is not null)
                {
                    span.SetTag("url.template", template);
                }

                if (req.Info.Attempt > 1)
                {
                    span.SetTag("http.request.resend_count", req.Info.Attempt - 1);
                }

                req.State.Span = (span.TraceId.ToHexString(), span.SpanId.ToHexString());
                req.Headers["traceparent"] = Telemetry.Traceparent(span);
                if (!string.IsNullOrEmpty(span.TraceStateString))
                {
                    req.Headers["tracestate"] = span.TraceStateString;
                }
            }
        }

        void Measure(int? status, string? error)
        {
            if (!ctx.Metrics)
            {
                return;
            }

            var tags = new TagList
            {
                { "http.request.method", req.Method },
                { "server.address", host },
                { "server.port", req.Url.Port },
            };
            if (status is { } s)
            {
                tags.Add("http.response.status_code", s);
            }

            if (error is not null)
            {
                tags.Add("error.type", error);
            }

            Telemetry.RequestDuration.Record(Stopwatch.GetElapsedTime(started).TotalSeconds, tags);
        }

        try
        {
            var resp = await next(req, ct).ConfigureAwait(false);
            var failed = resp.Status >= 400 ? resp.Status.ToString(System.Globalization.CultureInfo.InvariantCulture) : null;
            if (span is not null)
            {
                span.SetTag("http.response.status_code", resp.Status);
                if (resp.Header("x-request-id") is { } server)
                {
                    span.SetTag("inorbit.server_request_id", server);
                }

                if (string.Equals(resp.Header("idempotency-replayed")?.Trim(), "true", StringComparison.Ordinal))
                {
                    span.SetTag("inorbit.idempotency_replayed", true);
                }

                if (failed is not null)
                {
                    span.SetTag("error.type", failed);
                    span.SetStatus(ActivityStatusCode.Error);
                }

                span.Dispose();
            }

            Measure(resp.Status, failed);
            return resp;
        }
        catch (Exception e) when (e is InOrbitException or OperationCanceledException)
        {
            span?.SetTag("error.type", ErrorType(e));
            span?.SetStatus(ActivityStatusCode.Error);
            span?.Dispose();
            Measure(null, ErrorType(e));
            throw;
        }
    }

    private static async ValueTask<SdkResponse> LoggingAsync(ClientContext ctx, SdkRequest req, MiddlewareNext next, CancellationToken ct)
    {
        if (!ctx.Log.On(LogLevel.Debug))
        {
            return await next(req, ct).ConfigureAwait(false);
        }

        var span = req.State.Span;
        (string, object?)[] Base() =>
        [
            ("operation", req.Info.Operation),
            ("method", req.Method),
            ("path", req.Url.AbsolutePath),
            ("attempt", Math.Max(1, req.Info.Attempt)),
            ("request_id", req.Info.RequestId),
            ("trace_id", span?.TraceId),
            ("span_id", span?.SpanId),
        ];
        ctx.Log.Emit(LogLevel.Debug, [("event", "request"), .. Base(), ("headers", ctx.Log.Headers ? ctx.Log.Show(req.Headers, response: false) : null)]);
        var started = Stopwatch.GetTimestamp();
        var resp = await next(req, ct).ConfigureAwait(false);
        var shown = ctx.Log.Headers ? ctx.Log.Show(resp.Headers.Select(h => new KeyValuePair<string, string>(h.Key, string.Join(", ", h.Value))), response: true) : null;
        ctx.Log.Emit(
            LogLevel.Debug,
            [
                ("event", "response"), .. Base(), ("status", resp.Status), ("duration_ms", (long)Stopwatch.GetElapsedTime(started).TotalMilliseconds),
                ("server_request_id", resp.Header("x-request-id")), ("headers", shown),
            ]);
        return resp;
    }
}

/// <summary>A response body handed over unread: disposing it releases the response, and <c>onEnd</c> runs once, at its end or its disposal.</summary>
internal sealed class BodyStream(Stream inner, IDisposable? owner, Action? onEnd) : Stream
{
    private int _ended;

    public override bool CanRead => true;

    public override bool CanSeek => false;

    public override bool CanWrite => false;

    public override long Length => throw new NotSupportedException();

    public override long Position
    {
        get => throw new NotSupportedException();
        set => throw new NotSupportedException();
    }

    public override int Read(byte[] buffer, int offset, int count) => Ended(inner.Read(buffer, offset, count));

    public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default) =>
        Ended(await inner.ReadAsync(buffer, cancellationToken).ConfigureAwait(false));

    public override Task<int> ReadAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken) =>
        ReadAsync(buffer.AsMemory(offset, count), cancellationToken).AsTask();

    public override void Flush()
    {
    }

    public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();

    public override void SetLength(long value) => throw new NotSupportedException();

    public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();

    protected override void Dispose(bool disposing)
    {
        if (disposing)
        {
            End();
            inner.Dispose();
            owner?.Dispose();
        }

        base.Dispose(disposing);
    }

    private int Ended(int read)
    {
        if (read == 0)
        {
            End();
        }

        return read;
    }

    private void End()
    {
        if (Interlocked.Exchange(ref _ended, 1) == 0)
        {
            onEnd?.Invoke();
        }
    }
}
