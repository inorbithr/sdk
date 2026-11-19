namespace App;

/// <summary>The settings, read from the environment: the names are in .env.example.</summary>
public sealed record Config(string Host, int Port)
{
    public static Config Load(Func<string, string?> get)
    {
        var host = "127.0.0.1";
        var port = 8080;
        if (get("HOST") is { Length: > 0 } h)
        {
            host = h;
        }
        if (get("PORT") is { Length: > 0 } p)
        {
            if (!int.TryParse(p, out port) || port < 0 || port > 65535)
            {
                throw new ArgumentException("PORT must be a number from 0 to 65535");
            }
        }
        return new Config(host, port);
    }
}

/// <summary>
/// The service: /healthz, /readyz, /metrics and one route of your own. Ready is false until the
/// server listens and again as soon as shutdown starts, so a load balancer stops sending traffic
/// before the process stops.
/// </summary>
public sealed class Service(JsonLog log)
{
    public JsonLog Log { get; } = log;

    public Metrics Metrics { get; } = new();

    private volatile bool ready;

    private WebApplication? app;

    /// <summary>Starts listening and returns the port it got (PORT=0 picks a free one).</summary>
    public async Task<int> StartAsync(Config cfg)
    {
        var builder = WebApplication.CreateSlimBuilder();
        builder.Logging.ClearProviders(); // our JSON log is the only log
        builder.WebHost.ConfigureKestrel(k =>
        {
            k.AddServerHeader = false;
            k.Limits.RequestHeadersTimeout = TimeSpan.FromSeconds(5);
            k.Limits.KeepAliveTimeout = TimeSpan.FromSeconds(60);
            k.Limits.MaxRequestBodySize = 1024 * 1024;
        });
        builder.WebHost.UseUrls($"http://{cfg.Host}:{cfg.Port}");
        builder.Services.Configure<HostOptions>(o => o.ShutdownTimeout = TimeSpan.FromSeconds(10));
        app = builder.Build();
        app.Run(Observe);
        // SIGINT and SIGTERM reach the host's console lifetime: not ready from that moment.
        app.Lifetime.ApplicationStopping.Register(() => ready = false);
        await app.StartAsync();
        ready = true;
        var address = app.Urls.First();
        Log.Info("listening", ("addr", address.Replace("http://", string.Empty)));
        return new Uri(address).Port;
    }

    /// <summary>Returns when the process is asked to stop (SIGINT or SIGTERM).</summary>
    public Task WaitAsync() => app!.WaitForShutdownAsync();

    /// <summary>Stops accepting, lets requests in flight finish (10 s at most), then returns.</summary>
    public async Task StopAsync()
    {
        ready = false;
        Log.Info("shutting down");
        if (app is not null)
        {
            await app.StopAsync();
            await app.DisposeAsync();
        }
    }

    private record Reply(int Status, string Type, string Body);

    private static Reply JsonReply(int status, string body) => new(status, "application/json", body + "\n");

    /// <summary>Routes by exact path; every request is traced, logged and counted here, once.</summary>
    private async Task Observe(HttpContext ctx)
    {
        var span = Span.Start(ctx.Request.Headers["traceparent"].FirstOrDefault());
        ctx.Response.Headers["traceparent"] = span.Header();
        var method = ctx.Request.Method;
        var path = ctx.Request.Path.Value ?? "/";
        var route = "unmatched";
        Reply reply;
        if (HttpMethods.IsGet(method) && path == "/healthz")
        {
            route = path;
            reply = JsonReply(200, "{\"status\":\"ok\"}");
        }
        else if (HttpMethods.IsGet(method) && path == "/readyz")
        {
            route = path;
            reply = ready ? JsonReply(200, "{\"status\":\"ready\"}") : JsonReply(503, "{\"status\":\"not ready\"}");
        }
        else if (HttpMethods.IsGet(method) && path == "/metrics")
        {
            route = path;
            reply = new Reply(200, "text/plain; version=0.0.4", Metrics.Render());
        }
        else if (HttpMethods.IsGet(method) && path == "/v1/hello")
        {
            route = path;
            reply = Hello(ctx.Request.Query["name"].FirstOrDefault());
        }
        else
        {
            reply = JsonReply(404, "{\"error\":\"not found\"}");
        }
        ctx.Response.StatusCode = reply.Status;
        ctx.Response.ContentType = reply.Type;
        await ctx.Response.WriteAsync(reply.Body);
        var ms = span.Millis();
        if (span.ParentId is { } parent)
        {
            Log.Info("span", ("trace_id", span.TraceId), ("span_id", span.SpanId), ("name", $"{method} {route}"),
                ("route", route), ("status", reply.Status), ("duration_ms", ms), ("parent_span_id", parent));
        }
        else
        {
            Log.Info("span", ("trace_id", span.TraceId), ("span_id", span.SpanId), ("name", $"{method} {route}"),
                ("route", route), ("status", reply.Status), ("duration_ms", ms));
        }
        Log.Info("request", ("trace_id", span.TraceId), ("method", method), ("route", route),
            ("status", reply.Status), ("duration_ms", ms));
        Metrics.Inc(route, reply.Status);
    }

    /// <summary>
    /// Your first route: replace it. The name comes from the query string, so it is bounded and
    /// echoed back, never logged.
    /// </summary>
    private static Reply Hello(string? name)
    {
        name = string.IsNullOrEmpty(name) ? "world" : name;
        if (new System.Globalization.StringInfo(name).LengthInTextElements > 64)
        {
            return JsonReply(400, "{\"error\":\"name is at most 64 characters of text\"}");
        }
        return JsonReply(200, "{\"message\":" + Json.Str("Hello, " + name) + "}");
    }
}
